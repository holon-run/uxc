//! OpenAPI/Swagger adapter

use super::{
    Adapter, ExecutionMetadata, ExecutionResult, Operation, OperationDetail, Parameter,
    ProtocolType,
};
use crate::auth::{oauth, AuthType, Profile};
use crate::error::UxcError;
use anyhow::{Context, Result};
use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};

pub struct OpenAPIAdapter {
    client: reqwest::Client,
    cache: Option<Arc<dyn crate::cache::Cache>>,
    auth_profile: Option<Profile>,
    runtime_auth_profile: Arc<Mutex<Option<Profile>>>,
    oauth_refresh_lock: Arc<Mutex<()>>,
    discovered_schema_urls: Arc<RwLock<HashMap<String, String>>>,
    schema_url_override: Option<String>,
    force_refresh_schema: bool,
}

impl OpenAPIAdapter {
    const PATH_SEGMENT_ENCODE_SET: &'static AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    const MAX_SCHEMA_EXPANSION_DEPTH: usize = 8;
    const SCHEMA_ENDPOINTS: [&'static str; 7] = [
        "/openapi.json",
        "/swagger.json",
        "/api-docs",
        "/swagger/v1/swagger.json",
        "/api/docs",
        "/docs/swagger.json",
        "/swagger-docs",
    ];
    const HTTP_METHODS: [&'static str; 8] = [
        "get", "post", "put", "patch", "delete", "head", "options", "trace",
    ];

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            cache: None,
            auth_profile: None,
            runtime_auth_profile: Arc::new(Mutex::new(None)),
            oauth_refresh_lock: Arc::new(Mutex::new(())),
            discovered_schema_urls: Arc::new(RwLock::new(HashMap::new())),
            schema_url_override: None,
            force_refresh_schema: false,
        }
    }

    pub fn with_cache(mut self, cache: Arc<dyn crate::cache::Cache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn with_auth(mut self, profile: Profile) -> Self {
        self.auth_profile = Some(profile);
        self
    }

    pub fn with_schema_url_override(mut self, schema_url: Option<String>) -> Self {
        self.schema_url_override = schema_url;
        self
    }

    pub fn with_refresh_schema(mut self, refresh: bool) -> Self {
        self.force_refresh_schema = refresh;
        self
    }

    async fn effective_auth_profile(&self) -> Option<Profile> {
        self.runtime_auth_profile
            .lock()
            .await
            .clone()
            .or_else(|| self.auth_profile.clone())
    }

    fn schema_requests_apply_auth(&self) -> bool {
        self.schema_url_override.is_none()
    }

    async fn set_effective_auth_profile(&self, profile: Profile) {
        *self.runtime_auth_profile.lock().await = Some(profile);
    }

    fn apply_auth_profile(
        mut req: reqwest::RequestBuilder,
        profile: Option<&Profile>,
    ) -> Result<reqwest::RequestBuilder> {
        if let Some(profile) = profile {
            req = crate::auth::apply_profile_auth_to_request(req, profile)?;
        }
        Ok(req)
    }

    fn apply_auth_profile_to_url(url: &str, profile: Option<&Profile>) -> Result<String> {
        match profile {
            Some(profile) => crate::auth::apply_profile_auth_to_url(url, profile),
            None => Ok(url.to_string()),
        }
    }

    fn apply_schema_auth_profile(
        &self,
        req: reqwest::RequestBuilder,
        profile: Option<&Profile>,
    ) -> Result<reqwest::RequestBuilder> {
        if self.schema_requests_apply_auth() {
            Self::apply_auth_profile(req, profile)
        } else {
            Ok(req)
        }
    }

    fn apply_schema_auth_profile_to_url(
        &self,
        url: &str,
        profile: Option<&Profile>,
    ) -> Result<String> {
        if self.schema_requests_apply_auth() {
            Self::apply_auth_profile_to_url(url, profile)
        } else {
            Ok(url.to_string())
        }
    }

    async fn refresh_effective_oauth_profile(&self, force: bool) -> Result<Option<Profile>> {
        let _refresh_guard = self.oauth_refresh_lock.lock().await;
        let mut profile = self.effective_auth_profile().await;
        if let Some(active) = profile.as_mut() {
            if active.auth_type == AuthType::OAuth {
                let refreshed = if force {
                    oauth::refresh_oauth_profile(active, &self.client).await?;
                    true
                } else {
                    oauth::maybe_refresh_oauth_profile(active, &self.client, 60).await?
                };
                if refreshed {
                    crate::auth::persist_profile_if_named(active)?;
                    self.set_effective_auth_profile(active.clone()).await;
                }
            }
        }
        Ok(profile)
    }

    async fn send_with_oauth_retry<F>(&self, build_request: F) -> Result<reqwest::Response>
    where
        F: Fn(Option<&Profile>) -> Result<reqwest::RequestBuilder>,
    {
        let mut profile = self.refresh_effective_oauth_profile(false).await?;

        let mut response = build_request(profile.as_ref())?.send().await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && profile
                .as_ref()
                .is_some_and(|active| active.auth_type == AuthType::OAuth)
        {
            profile = self.refresh_effective_oauth_profile(true).await?;
            response = build_request(profile.as_ref())?.send().await?;
        }

        Ok(response)
    }

    fn normalized_url(url: &str) -> String {
        url.trim_end_matches('/').to_string()
    }

    fn schema_cache_key(url: &str, schema_url: &str) -> String {
        format!("{}#schema={}", Self::normalized_url(url), schema_url)
    }

    fn schema_candidates(url: &str) -> Vec<String> {
        let normalized = Self::normalized_url(url);
        if Self::SCHEMA_ENDPOINTS
            .iter()
            .any(|endpoint| normalized.ends_with(endpoint))
        {
            return vec![normalized];
        }

        let mut candidates = Vec::new();
        for endpoint in Self::SCHEMA_ENDPOINTS {
            candidates.push(format!("{}{}", normalized, endpoint));
        }

        candidates.sort();
        candidates.dedup();
        candidates
    }

    fn is_openapi_document(body: &Value) -> bool {
        body.get("openapi").is_some() || body.get("swagger").is_some()
    }

    async fn check_schema_url(&self, schema_url: &str) -> Result<bool> {
        let response = self
            .send_with_oauth_retry(|profile| {
                let schema_url = self.apply_schema_auth_profile_to_url(schema_url, profile)?;
                let req = self
                    .client
                    .get(&schema_url)
                    .timeout(std::time::Duration::from_secs(10))
                    .header("Accept", "application/json");
                self.apply_schema_auth_profile(req, profile)
            })
            .await?;

        if !response.status().is_success() {
            return Ok(false);
        }

        let body = response.json::<Value>().await?;
        Ok(Self::is_openapi_document(&body))
    }

    fn is_http_method(method: &str) -> bool {
        Self::HTTP_METHODS.contains(&method)
    }

    fn operation_id(method: &str, path: &str) -> String {
        format!("{}:{}", method.to_lowercase(), path)
    }

    fn display_name(method: &str, path: &str) -> String {
        format!("{} {}", method.to_uppercase(), path)
    }

    fn parse_operation_id(operation_id: &str) -> Result<(String, String)> {
        let (method, path) = operation_id.split_once(':').ok_or_else(|| {
            UxcError::InvalidArguments(
                "Invalid operation ID format. Use 'method:/path'".to_string(),
            )
        })?;

        if method.is_empty() || path.is_empty() || !path.starts_with('/') {
            return Err(UxcError::InvalidArguments(
                "Invalid operation ID format. Use 'method:/path'".to_string(),
            )
            .into());
        }

        let method = method.to_lowercase();
        if !Self::is_http_method(&method) {
            return Err(UxcError::InvalidArguments(format!(
                "Unsupported HTTP method in operation ID: {}",
                method
            ))
            .into());
        }

        Ok((method, path.to_string()))
    }

    async fn discover_schema_url(&self, url: &str) -> Result<Option<String>> {
        let normalized = Self::normalized_url(url);
        {
            let cache = self.discovered_schema_urls.read().await;
            if let Some(discovered) = cache.get(&normalized) {
                return Ok(Some(discovered.clone()));
            }
        }

        if let Some(schema_url) = &self.schema_url_override {
            let is_openapi = self.check_schema_url(schema_url).await.with_context(|| {
                format!(
                    "Failed to fetch OpenAPI schema from --schema-url '{}'",
                    schema_url
                )
            })?;
            if !is_openapi {
                return Err(anyhow::anyhow!(
                    "Schema URL does not contain an OpenAPI document: {}",
                    schema_url
                ));
            }
            let mut cache = self.discovered_schema_urls.write().await;
            cache.insert(normalized, schema_url.clone());
            return Ok(Some(schema_url.clone()));
        }

        if let Some(mapping) = crate::schema_mapping::resolve_openapi_schema_mapping(&normalized) {
            match self.check_schema_url(&mapping.schema_url).await {
                Ok(true) => {
                    info!(
                        "Resolved OpenAPI schema via {}: {} -> {}",
                        mapping.source.as_str(),
                        normalized,
                        mapping.schema_url
                    );
                    let mut cache = self.discovered_schema_urls.write().await;
                    cache.insert(normalized, mapping.schema_url.clone());
                    return Ok(Some(mapping.schema_url));
                }
                Ok(false) => {
                    debug!(
                        "Mapped schema URL did not contain OpenAPI document: {}",
                        mapping.schema_url
                    );
                }
                Err(err) => {
                    debug!(
                        "Failed to fetch mapped schema URL '{}': {}",
                        mapping.schema_url, err
                    );
                }
            }
        }

        for full_url in Self::schema_candidates(&normalized) {
            let resp = match self
                .send_with_oauth_retry(|profile| {
                    let full_url = self.apply_schema_auth_profile_to_url(&full_url, profile)?;
                    let req = self
                        .client
                        .get(&full_url)
                        .timeout(std::time::Duration::from_secs(2))
                        .header("Accept", "application/json");
                    self.apply_schema_auth_profile(req, profile)
                })
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };

            if !resp.status().is_success() {
                continue;
            }

            if let Ok(body) = resp.json::<Value>().await {
                if Self::is_openapi_document(&body) {
                    let mut cache = self.discovered_schema_urls.write().await;
                    cache.insert(normalized, full_url.clone());
                    return Ok(Some(full_url));
                }
            }
        }

        Ok(None)
    }

    fn resolve_local_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
        if !reference.starts_with("#/") {
            return None;
        }
        root.pointer(&reference[1..])
    }

    fn dereference_value<'a>(value: &'a Value, root: &'a Value) -> &'a Value {
        let mut current = value;
        for _ in 0..Self::MAX_SCHEMA_EXPANSION_DEPTH {
            let Some(reference) = current.get("$ref").and_then(|v| v.as_str()) else {
                break;
            };
            let Some(resolved) = Self::resolve_local_ref(root, reference) else {
                break;
            };
            current = resolved;
        }
        current
    }

    fn schema_type_hint(schema: &Value, root: &Value) -> String {
        let resolved = Self::dereference_value(schema, root);
        if let Some(type_name) = resolved.get("type").and_then(|t| t.as_str()) {
            return type_name.to_string();
        }
        if resolved.get("properties").is_some()
            || resolved.get("allOf").is_some()
            || resolved.get("oneOf").is_some()
            || resolved.get("anyOf").is_some()
        {
            return "object".to_string();
        }
        if resolved.get("items").is_some() {
            return "array".to_string();
        }
        "string".to_string()
    }

    fn parse_parameter(parameter: &Value, root: &Value) -> Option<Parameter> {
        let resolved = Self::dereference_value(parameter, root);
        let name = resolved.get("name").and_then(|n| n.as_str())?;
        let param_type = resolved
            .get("schema")
            .map(|schema| Self::schema_type_hint(schema, root))
            .or_else(|| {
                if resolved.get("content").is_some() {
                    Some("object".to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "string".to_string());

        Some(Parameter {
            name: name.to_string(),
            param_type,
            required: resolved
                .get("required")
                .and_then(|r| r.as_bool())
                .unwrap_or(false),
            description: resolved
                .get("description")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string()),
        })
    }

    fn collect_parameters(
        path_item: &Value,
        operation_spec: &Value,
        root: &Value,
    ) -> Vec<Parameter> {
        let mut parameters = Vec::new();
        let mut seen = HashSet::new();

        for source in [
            operation_spec.get("parameters").and_then(|p| p.as_array()),
            path_item.get("parameters").and_then(|p| p.as_array()),
        ]
        .into_iter()
        .flatten()
        {
            for parameter in source {
                let resolved = Self::dereference_value(parameter, root);
                let key = format!(
                    "{}:{}",
                    resolved.get("in").and_then(|v| v.as_str()).unwrap_or(""),
                    resolved.get("name").and_then(|v| v.as_str()).unwrap_or("")
                );
                if seen.contains(&key) {
                    continue;
                }
                if let Some(parsed) = Self::parse_parameter(parameter, root) {
                    seen.insert(key);
                    parameters.push(parsed);
                }
            }
        }

        parameters
    }

    fn expand_schema(
        value: &Value,
        root: &Value,
        visited: &mut HashSet<String>,
        depth: usize,
    ) -> Value {
        if depth == 0 {
            return value.clone();
        }

        match value {
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(|v| v.as_str()) {
                    if !visited.insert(reference.to_string()) {
                        return serde_json::json!({ "$ref": reference });
                    }

                    let expanded_target = Self::resolve_local_ref(root, reference)
                        .map(|target| Self::expand_schema(target, root, visited, depth - 1))
                        .unwrap_or_else(|| value.clone());
                    visited.remove(reference);

                    if object.len() == 1 {
                        return expanded_target;
                    }

                    if let Value::Object(mut merged) = expanded_target {
                        for (key, nested) in object {
                            if key == "$ref" {
                                continue;
                            }
                            merged.insert(
                                key.clone(),
                                Self::expand_schema(nested, root, visited, depth - 1),
                            );
                        }
                        return Value::Object(merged);
                    }

                    let mut merged = Map::new();
                    merged.insert("allOf".to_string(), Value::Array(vec![expanded_target]));
                    for (key, nested) in object {
                        if key == "$ref" {
                            continue;
                        }
                        merged.insert(
                            key.clone(),
                            Self::expand_schema(nested, root, visited, depth - 1),
                        );
                    }
                    return Value::Object(merged);
                }

                let mut expanded = Map::new();
                for (key, nested) in object {
                    expanded.insert(
                        key.clone(),
                        Self::expand_schema(nested, root, visited, depth - 1),
                    );
                }
                Value::Object(expanded)
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| Self::expand_schema(item, root, visited, depth - 1))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }

    fn extract_request_body_input_schema(
        path_item: &Value,
        operation_spec: &Value,
        root: &Value,
    ) -> Option<Value> {
        if let Some(schema) = Self::extract_oas3_request_body_input_schema(operation_spec, root) {
            return Some(schema);
        }

        Self::extract_swagger2_request_body_input_schema(path_item, operation_spec, root)
    }

    fn extract_oas3_request_body_input_schema(
        operation_spec: &Value,
        root: &Value,
    ) -> Option<Value> {
        let request_body_raw = operation_spec.get("requestBody")?;
        let request_body = Self::dereference_value(request_body_raw, root);
        let content = request_body.get("content")?.as_object()?;

        let mut content_map = Map::new();
        for (media_type, media_spec) in content {
            let Some(schema) = media_spec.get("schema") else {
                continue;
            };

            let source_ref = schema
                .get("$ref")
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            let expanded_schema = Self::expand_schema(
                schema,
                root,
                &mut HashSet::new(),
                Self::MAX_SCHEMA_EXPANSION_DEPTH,
            );

            let mut media_obj = Map::new();
            media_obj.insert("schema".to_string(), expanded_schema);
            if let Some(reference) = source_ref {
                media_obj.insert("source_ref".to_string(), Value::String(reference));
            }
            if let Some(example) = media_spec.get("example") {
                media_obj.insert("example".to_string(), example.clone());
            }
            content_map.insert(media_type.clone(), Value::Object(media_obj));
        }
        if content_map.is_empty() {
            return None;
        }

        let mut body = Map::new();
        body.insert(
            "kind".to_string(),
            Value::String("openapi_request_body".to_string()),
        );
        body.insert(
            "required".to_string(),
            Value::Bool(
                request_body
                    .get("required")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false),
            ),
        );
        if let Some(description) = request_body.get("description").and_then(|d| d.as_str()) {
            body.insert(
                "description".to_string(),
                Value::String(description.to_string()),
            );
        }
        body.insert("content".to_string(), Value::Object(content_map));
        Some(Value::Object(body))
    }

    fn extract_swagger2_request_body_input_schema(
        path_item: &Value,
        operation_spec: &Value,
        root: &Value,
    ) -> Option<Value> {
        let body_parameter =
            Self::collect_effective_operation_parameters(path_item, operation_spec)
                .into_iter()
                .find_map(|parameter| {
                    let resolved = Self::dereference_value(parameter, root);
                    (resolved.get("in").and_then(|v| v.as_str()) == Some("body"))
                        .then_some(resolved)
                })?;
        let schema = body_parameter.get("schema")?;

        let source_ref = schema
            .get("$ref")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());
        let expanded_schema = Self::expand_schema(
            schema,
            root,
            &mut HashSet::new(),
            Self::MAX_SCHEMA_EXPANSION_DEPTH,
        );

        let mut media = Map::new();
        media.insert("schema".to_string(), expanded_schema);
        if let Some(reference) = source_ref {
            media.insert("source_ref".to_string(), Value::String(reference));
        }

        let mut content = Map::new();
        content.insert("application/json".to_string(), Value::Object(media));

        let mut body = Map::new();
        body.insert(
            "kind".to_string(),
            Value::String("openapi_request_body".to_string()),
        );
        body.insert(
            "required".to_string(),
            Value::Bool(
                body_parameter
                    .get("required")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false),
            ),
        );
        if let Some(description) = body_parameter.get("description").and_then(|d| d.as_str()) {
            body.insert(
                "description".to_string(),
                Value::String(description.to_string()),
            );
        }
        body.insert("content".to_string(), Value::Object(content));
        Some(Value::Object(body))
    }

    fn prepare_request(
        method: &str,
        base_url: &str,
        path_template: &str,
        path_item: &Value,
        operation_spec: &Value,
        root: &Value,
        args: &HashMap<String, Value>,
    ) -> Result<PreparedRequest> {
        let mut remaining = args.clone();
        let mut headers = Vec::new();
        let mut query_pairs = Vec::new();
        let mut form_pairs = Vec::new();
        let mut explicit_body = None;
        let mut missing_required_body_param = None;
        let mut resolved_path = path_template.to_string();
        let mut seen = HashSet::new();

        for source in [
            operation_spec.get("parameters").and_then(|p| p.as_array()),
            path_item.get("parameters").and_then(|p| p.as_array()),
        ]
        .into_iter()
        .flatten()
        {
            for parameter in source {
                let resolved = Self::dereference_value(parameter, root);
                let Some(name) = resolved.get("name").and_then(|value| value.as_str()) else {
                    continue;
                };
                let location = resolved
                    .get("in")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let key = format!("{}:{}", location, name);
                if !seen.insert(key) {
                    continue;
                }

                let required = if location == "path" {
                    true
                } else {
                    resolved
                        .get("required")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                };
                let value = remaining.remove(name);
                match location {
                    "path" => {
                        if required && value.is_none() {
                            anyhow::bail!("Missing required parameter '{}'", name);
                        }
                        let Some(value) = value else {
                            continue;
                        };
                        let rendered = Self::value_to_string(&value, name)?;
                        resolved_path = resolved_path.replace(
                            &format!("{{{}}}", name),
                            &Self::encode_path_param_value(&rendered),
                        );
                    }
                    "query" => {
                        if required && value.is_none() {
                            anyhow::bail!("Missing required parameter '{}'", name);
                        }
                        let Some(value) = value else {
                            continue;
                        };
                        let rendered = Self::value_to_string(&value, name)?;
                        query_pairs.push((name.to_string(), rendered));
                    }
                    "header" => {
                        if required && value.is_none() {
                            anyhow::bail!("Missing required parameter '{}'", name);
                        }
                        let Some(value) = value else {
                            continue;
                        };
                        let rendered = Self::value_to_string(&value, name)?;
                        headers.push((name.to_string(), rendered));
                    }
                    "body" => {
                        if let Some(value) = value {
                            explicit_body = Some(value);
                        } else if required {
                            missing_required_body_param = Some(name.to_string());
                        }
                    }
                    "formData" => {
                        if required && value.is_none() {
                            anyhow::bail!("Missing required parameter '{}'", name);
                        }
                        let Some(value) = value else {
                            continue;
                        };
                        let rendered = Self::value_to_string(&value, name)?;
                        form_pairs.push((name.to_string(), rendered));
                    }
                    _ => {}
                }
            }
        }

        let body_config = Self::request_body_config(path_item, operation_spec, root)?;
        let (json_body, form_body) = match body_config {
            RequestBodyConfig::None => (None, None),
            RequestBodyConfig::FormUrlEncoded => {
                if let Some(body) = remaining.remove("body") {
                    if !remaining.is_empty() || !form_pairs.is_empty() {
                        anyhow::bail!("Cannot mix 'body' with form arguments for this operation");
                    }
                    form_pairs.extend(Self::form_pairs_from_value(body)?);
                } else {
                    form_pairs.extend(Self::query_pairs_from_remaining(&mut remaining)?);
                }
                (None, Some(form_pairs))
            }
            RequestBodyConfig::Json => {
                if explicit_body.is_none() && remaining.is_empty() {
                    if let Some(name) = &missing_required_body_param {
                        anyhow::bail!("Missing required parameter '{}'", name);
                    }
                }
                let json_body =
                    Self::json_body_from_remaining_with_explicit(&mut remaining, explicit_body)?;
                (Some(json_body), None)
            }
            RequestBodyConfig::UnsupportedMultipart => {
                anyhow::bail!(
                    "Unsupported OpenAPI request body content type 'multipart/form-data'. Supported: application/json, application/x-www-form-urlencoded"
                );
            }
        };

        let has_parameter_schema = !seen.is_empty();
        if matches!(body_config, RequestBodyConfig::None) {
            if Self::method_prefers_implicit_json_body(method) && !has_parameter_schema {
                let body = Self::json_body_from_remaining(&mut remaining)?;
                return Ok(PreparedRequest {
                    url: format!("{}{}", base_url.trim_end_matches('/'), resolved_path),
                    headers,
                    query_pairs,
                    json_body: Some(body),
                    form_body: None,
                });
            }

            query_pairs.extend(Self::query_pairs_from_remaining(&mut remaining)?);
        } else if !remaining.is_empty() {
            anyhow::bail!(
                "Unexpected arguments for operation request body: {}",
                remaining.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }

        Ok(PreparedRequest {
            url: format!("{}{}", base_url.trim_end_matches('/'), resolved_path),
            headers,
            query_pairs,
            json_body,
            form_body,
        })
    }

    fn collect_effective_operation_parameters<'a>(
        path_item: &'a Value,
        operation_spec: &'a Value,
    ) -> Vec<&'a Value> {
        let mut out = Vec::new();
        if let Some(parameters) = operation_spec
            .get("parameters")
            .and_then(|value| value.as_array())
        {
            out.extend(parameters.iter());
        }
        if let Some(parameters) = path_item
            .get("parameters")
            .and_then(|value| value.as_array())
        {
            out.extend(parameters.iter());
        }
        out
    }

    fn request_body_config(
        path_item: &Value,
        operation_spec: &Value,
        root: &Value,
    ) -> Result<RequestBodyConfig> {
        if let Some(config) = Self::request_body_config_from_oas3(operation_spec, root)? {
            return Ok(config);
        }

        Self::request_body_config_from_swagger2(path_item, operation_spec, root)
    }

    fn request_body_config_from_oas3(
        operation_spec: &Value,
        root: &Value,
    ) -> Result<Option<RequestBodyConfig>> {
        let Some(request_body_raw) = operation_spec.get("requestBody") else {
            return Ok(None);
        };
        let request_body = Self::dereference_value(request_body_raw, root);
        let Some(content) = request_body
            .get("content")
            .and_then(|value| value.as_object())
        else {
            return Ok(Some(RequestBodyConfig::None));
        };
        if content.contains_key("application/json") {
            return Ok(Some(RequestBodyConfig::Json));
        }
        if content.contains_key("application/x-www-form-urlencoded") {
            return Ok(Some(RequestBodyConfig::FormUrlEncoded));
        }
        if content.contains_key("multipart/form-data") {
            return Ok(Some(RequestBodyConfig::UnsupportedMultipart));
        }
        if content.len() == 1 {
            if let Some(kind) = content.keys().next() {
                anyhow::bail!(
                    "Unsupported OpenAPI request body content type '{}'. Supported: application/json, application/x-www-form-urlencoded",
                    kind
                );
            }
        }
        Ok(Some(RequestBodyConfig::None))
    }

    fn request_body_config_from_swagger2(
        path_item: &Value,
        operation_spec: &Value,
        root: &Value,
    ) -> Result<RequestBodyConfig> {
        if root.get("swagger").and_then(|v| v.as_str()) != Some("2.0") {
            return Ok(RequestBodyConfig::None);
        }

        let mut has_body_param = false;
        let mut has_form_param = false;
        let mut has_file_param = false;

        for parameter in Self::collect_effective_operation_parameters(path_item, operation_spec) {
            let resolved = Self::dereference_value(parameter, root);
            match resolved.get("in").and_then(|value| value.as_str()) {
                Some("body") => has_body_param = true,
                Some("formData") => {
                    has_form_param = true;
                    if resolved.get("type").and_then(|value| value.as_str()) == Some("file") {
                        has_file_param = true;
                    }
                }
                _ => {}
            }
        }

        if has_body_param && has_form_param {
            anyhow::bail!("Swagger 2.0 operation cannot mix 'body' and 'formData' parameters");
        }

        if !has_body_param && !has_form_param {
            return Ok(RequestBodyConfig::None);
        }

        let consumes = operation_spec
            .get("consumes")
            .or_else(|| root.get("consumes"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        if has_body_param {
            if consumes.is_empty()
                || consumes
                    .iter()
                    .any(|item| item.as_str() == Some("application/json"))
            {
                return Ok(RequestBodyConfig::Json);
            }
            if consumes
                .iter()
                .any(|item| item.as_str() == Some("application/x-www-form-urlencoded"))
            {
                return Ok(RequestBodyConfig::FormUrlEncoded);
            }
            if consumes
                .iter()
                .any(|item| item.as_str() == Some("multipart/form-data"))
            {
                return Ok(RequestBodyConfig::UnsupportedMultipart);
            }
            anyhow::bail!(
                "Unsupported Swagger 2.0 body content type '{}'. Supported: application/json, application/x-www-form-urlencoded",
                consumes
                    .iter()
                    .filter_map(|item| item.as_str())
                    .next()
                    .unwrap_or("unknown")
            );
        }

        if has_file_param
            || consumes
                .iter()
                .any(|item| item.as_str() == Some("multipart/form-data"))
        {
            return Ok(RequestBodyConfig::UnsupportedMultipart);
        }

        Ok(RequestBodyConfig::FormUrlEncoded)
    }

    fn encode_path_param_value(value: &str) -> String {
        utf8_percent_encode(value, Self::PATH_SEGMENT_ENCODE_SET).to_string()
    }

    fn method_prefers_implicit_json_body(method: &str) -> bool {
        matches!(method, "post" | "put" | "patch")
    }

    fn json_body_from_remaining(remaining: &mut HashMap<String, Value>) -> Result<Value> {
        Self::json_body_from_remaining_with_explicit(remaining, None)
    }

    fn json_body_from_remaining_with_explicit(
        remaining: &mut HashMap<String, Value>,
        explicit_body: Option<Value>,
    ) -> Result<Value> {
        if let Some(body) = explicit_body.or_else(|| remaining.remove("body")) {
            if !remaining.is_empty() {
                anyhow::bail!(
                    "Cannot mix 'body' with other request body arguments: {}",
                    remaining.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }
            return Ok(body);
        }

        let mut object = Map::new();
        for (name, value) in remaining.drain() {
            object.insert(name, value);
        }
        Ok(Value::Object(object))
    }

    fn strip_schema_endpoint(url: &str) -> String {
        let normalized = Self::normalized_url(url);
        for endpoint in Self::SCHEMA_ENDPOINTS {
            if normalized.ends_with(endpoint) {
                let stripped = normalized.trim_end_matches(endpoint).trim_end_matches('/');
                return stripped.to_string();
            }
        }
        normalized
    }

    fn operation_base_url(endpoint: &str, schema: &Value) -> String {
        if schema.get("swagger").and_then(|v| v.as_str()) != Some("2.0") {
            return endpoint.to_string();
        }

        let fallback = Self::strip_schema_endpoint(endpoint);

        let host = match schema.get("host").and_then(|v| v.as_str()) {
            Some(host) if !host.trim().is_empty() => host.trim(),
            _ => return fallback,
        };

        let parsed_endpoint = url::Url::parse(endpoint).ok();
        let scheme = schema
            .get("schemes")
            .and_then(|v| v.as_array())
            .and_then(|schemes| schemes.iter().find_map(|item| item.as_str()))
            .or_else(|| parsed_endpoint.as_ref().map(|parsed| parsed.scheme()))
            .unwrap_or("https");

        let base_path = schema
            .get("basePath")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let base_path = if base_path.is_empty() {
            String::new()
        } else if base_path.starts_with('/') {
            base_path.trim_end_matches('/').to_string()
        } else {
            format!("/{}", base_path.trim_end_matches('/'))
        };

        format!("{}://{}{}", scheme, host, base_path)
    }

    fn form_pairs_from_value(value: Value) -> Result<Vec<(String, String)>> {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Form request body must be an object"))?;
        let mut pairs = Vec::with_capacity(object.len());
        for (name, value) in object {
            pairs.push((name.clone(), Self::value_to_string(value, name)?));
        }
        Ok(pairs)
    }

    fn query_pairs_from_remaining(
        remaining: &mut HashMap<String, Value>,
    ) -> Result<Vec<(String, String)>> {
        let mut keys = remaining.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        let mut pairs = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(value) = remaining.remove(&key) {
                pairs.push((key.clone(), Self::value_to_string(&value, &key)?));
            }
        }
        Ok(pairs)
    }

    fn value_to_string(value: &Value, field_name: &str) -> Result<String> {
        match value {
            Value::String(string) => Ok(string.clone()),
            Value::Number(number) => Ok(number.to_string()),
            Value::Bool(boolean) => Ok(boolean.to_string()),
            Value::Null => Ok(String::new()),
            _ => anyhow::bail!(
                "Argument '{}' must be a string, number, bool, or null for non-JSON request placement",
                field_name
            ),
        }
    }

    fn security_requirement(requirement: &Value) -> Option<OperationAuthRequirement> {
        let items = requirement.as_array()?;
        if items.is_empty() {
            return Some(OperationAuthRequirement::Public);
        }

        let mut has_non_empty_object = false;
        for item in items {
            let Some(obj) = item.as_object() else {
                continue;
            };
            if obj.is_empty() {
                return Some(OperationAuthRequirement::Public);
            }
            has_non_empty_object = true;
        }

        if has_non_empty_object {
            Some(OperationAuthRequirement::RequiresAuth)
        } else {
            None
        }
    }

    fn schema_has_any_operation_security(root: &Value) -> bool {
        root.get("paths")
            .and_then(|paths| paths.as_object())
            .is_some_and(|paths| {
                paths.values().any(|path_item| {
                    path_item.as_object().is_some_and(|methods| {
                        methods.iter().any(|(method, spec)| {
                            Self::is_http_method(&method.to_lowercase())
                                && spec.get("security").is_some()
                        })
                    })
                })
            })
    }

    fn operation_auth_requirement(
        operation_spec: &Value,
        root: &Value,
    ) -> OperationAuthRequirement {
        if let Some(requirement) = operation_spec.get("security") {
            return Self::security_requirement(requirement)
                .unwrap_or(OperationAuthRequirement::Unknown);
        }

        if let Some(requirement) = root.get("security") {
            return Self::security_requirement(requirement)
                .unwrap_or(OperationAuthRequirement::Unknown);
        }

        if Self::schema_has_any_operation_security(root) {
            // When security requirements exist elsewhere and this operation has none,
            // OpenAPI semantics treat this operation as public.
            return OperationAuthRequirement::Public;
        }

        OperationAuthRequirement::Unknown
    }
}

#[derive(Debug)]
struct PreparedRequest {
    url: String,
    headers: Vec<(String, String)>,
    query_pairs: Vec<(String, String)>,
    json_body: Option<Value>,
    form_body: Option<Vec<(String, String)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestBodyConfig {
    None,
    Json,
    FormUrlEncoded,
    UnsupportedMultipart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationAuthRequirement {
    Public,
    RequiresAuth,
    Unknown,
}

impl Default for OpenAPIAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for OpenAPIAdapter {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::OpenAPI
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        Ok(self.discover_schema_url(url).await?.is_some())
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        let normalized_url = Self::normalized_url(url);

        // Try endpoint-level cache first so protocol resolution can be cache-first.
        if !self.force_refresh_schema {
            if let Some(cache) = &self.cache {
                match cache.get(&normalized_url)? {
                    crate::cache::CacheResult::Hit(schema) => {
                        if Self::is_openapi_document(&schema) {
                            debug!("OpenAPI endpoint cache hit for: {}", normalized_url);
                            return Ok(schema);
                        }
                    }
                    crate::cache::CacheResult::Bypassed => {
                        debug!("OpenAPI endpoint cache bypassed for: {}", normalized_url);
                    }
                    crate::cache::CacheResult::Miss => {}
                }
            }
        }

        let schema_url = self
            .discover_schema_url(url)
            .await?
            .ok_or_else(|| anyhow::anyhow!("OpenAPI schema endpoint not found for {}", url))?;

        let cache_key = Self::schema_cache_key(url, &schema_url);

        // Try cache first if available
        if !self.force_refresh_schema {
            if let Some(cache) = &self.cache {
                match cache.get(&cache_key)? {
                    crate::cache::CacheResult::Hit(schema) => {
                        debug!("OpenAPI cache hit for: {}", cache_key);
                        return Ok(schema);
                    }
                    crate::cache::CacheResult::Bypassed => {
                        debug!("OpenAPI cache bypassed for: {}", cache_key);
                    }
                    crate::cache::CacheResult::Miss => {
                        debug!("OpenAPI cache miss for: {}", cache_key);
                    }
                }
            }
        }

        // Fetch from remote
        let resp = self
            .send_with_oauth_retry(|profile| {
                let schema_url = self.apply_schema_auth_profile_to_url(&schema_url, profile)?;
                let req = self.client.get(&schema_url);
                self.apply_schema_auth_profile(req, profile)
            })
            .await?;
        let schema: Value = resp.json().await?;

        // Store in cache if available
        if let Some(cache) = &self.cache {
            if let Err(e) = cache.put(&cache_key, &schema) {
                debug!("Failed to cache OpenAPI schema: {}", e);
            } else {
                info!("Cached OpenAPI schema for: {}", cache_key);
            }
            if let Err(e) = cache.put(&normalized_url, &schema) {
                debug!("Failed to cache OpenAPI endpoint schema: {}", e);
            }
        }

        Ok(schema)
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        let schema = self.fetch_schema(url).await?;
        let mut operations = Vec::new();

        if let Some(paths) = schema.get("paths").and_then(|p| p.as_object()) {
            for (path, methods) in paths {
                if let Some(methods_obj) = methods.as_object() {
                    for (method, spec) in methods_obj {
                        let method = method.to_lowercase();
                        if !Self::is_http_method(&method) {
                            continue;
                        }

                        let operation_id = Self::operation_id(&method, path);
                        let display_name = Self::display_name(&method, path);
                        let parameters = Self::collect_parameters(methods, spec, &schema);

                        operations.push(Operation {
                            operation_id,
                            display_name,
                            description: spec
                                .get("description")
                                .or(spec.get("summary"))
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            parameters,
                            return_type: None,
                        });
                    }
                }
            }
        }

        Ok(operations)
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        let (method, path) = Self::parse_operation_id(operation)?;
        let schema = self.fetch_schema(url).await?;
        let paths = schema
            .get("paths")
            .and_then(|p| p.as_object())
            .ok_or_else(|| {
                UxcError::SchemaRetrievalFailed("OpenAPI schema missing paths".to_string())
            })?;
        let path_item = paths
            .get(&path)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;
        let operation_spec = path_item
            .get(&method)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;

        let parameters = Self::collect_parameters(path_item, operation_spec, &schema);
        let description = operation_spec
            .get("description")
            .or(operation_spec.get("summary"))
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());
        let input_schema =
            Self::extract_request_body_input_schema(path_item, operation_spec, &schema);

        Ok(OperationDetail {
            operation_id: operation.to_string(),
            display_name: Self::display_name(&method, &path),
            description,
            parameters,
            return_type: None,
            input_schema,
        })
    }

    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let (method, path) = Self::parse_operation_id(operation)?;
        let schema = self.fetch_schema(url).await?;
        let paths = schema
            .get("paths")
            .and_then(|p| p.as_object())
            .ok_or_else(|| {
                UxcError::SchemaRetrievalFailed("OpenAPI schema missing paths".to_string())
            })?;
        let path_item = paths
            .get(&path)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;
        let operation_spec = path_item
            .get(&method)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;
        let operation_base_url = Self::operation_base_url(url, &schema);
        let prepared = Self::prepare_request(
            &method,
            &operation_base_url,
            &path,
            path_item,
            operation_spec,
            &schema,
            &args,
        )?;
        let prepared_url = prepared.url.clone();
        let prepared_headers = prepared.headers.clone();
        let prepared_query_pairs = prepared.query_pairs.clone();
        let prepared_json_body = prepared.json_body.clone();
        let prepared_form_body = prepared.form_body.clone();
        let auth_requirement = Self::operation_auth_requirement(operation_spec, &schema);
        let should_apply_auth = !matches!(auth_requirement, OperationAuthRequirement::Public);

        let resp = self
            .send_with_oauth_retry(|profile| {
                let full_url = {
                    let mut parsed = url::Url::parse(&prepared_url).with_context(|| {
                        format!("Invalid prepared OpenAPI request URL '{}'", prepared_url)
                    })?;
                    if !prepared_query_pairs.is_empty() {
                        parsed
                            .query_pairs_mut()
                            .extend_pairs(prepared_query_pairs.iter().map(|(k, v)| (&**k, &**v)));
                    }
                    let with_args = parsed.to_string();
                    if should_apply_auth {
                        Self::apply_auth_profile_to_url(&with_args, profile)?
                    } else {
                        with_args
                    }
                };
                let mut req = match method.as_str() {
                    "get" => self.client.get(&full_url),
                    "post" => self.client.post(&full_url),
                    "put" => self.client.put(&full_url),
                    "delete" => self.client.delete(&full_url),
                    "patch" => self.client.patch(&full_url),
                    "head" => self.client.head(&full_url),
                    "options" => self.client.request(reqwest::Method::OPTIONS, &full_url),
                    "trace" => self.client.request(reqwest::Method::TRACE, &full_url),
                    _ => {
                        return Err(UxcError::InvalidArguments(format!(
                            "Unsupported HTTP method: {}",
                            method
                        ))
                        .into())
                    }
                };
                for (name, value) in &prepared_headers {
                    req = req.header(name, value);
                }
                let mut req = if should_apply_auth {
                    Self::apply_auth_profile(req, profile)?
                } else {
                    req
                };
                if let Some(body) = &prepared_json_body {
                    req = req.json(body);
                } else if let Some(body) = &prepared_form_body {
                    req = req.form(body);
                }
                Ok(req)
            })
            .await?;
        let status = resp.status();

        // Check HTTP status and provide detailed error info
        if !status.is_success() {
            // Try to get response body for error context
            let error_body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("[Failed to read error body: {}]", e));

            // Truncate body if too long for error message
            let truncated_body = if error_body.len() > 500 {
                format!("{}...", &error_body[..500])
            } else {
                error_body
            };

            return Err(UxcError::HttpError {
                status_code: status.as_u16(),
                message: truncated_body,
            }
            .into());
        }

        // Parse JSON response only on success
        // Handle empty responses (e.g., 204 No Content)
        let data: Value = match status.as_u16() {
            204 => serde_json::Value::Null,
            _ => {
                // Read the response body and detect emptiness from the actual bytes
                let bytes = resp.bytes().await.with_context(|| {
                    format!(
                        "error reading response body: HTTP {} from {}",
                        status.as_u16(),
                        prepared_url
                    )
                })?;

                if bytes.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_slice::<Value>(&bytes).with_context(|| {
                        format!(
                            "error decoding response body: HTTP {} from {}",
                            status.as_u16(),
                            prepared_url
                        )
                    })?
                }
            }
        };

        Ok(ExecutionResult {
            data,
            metadata: ExecutionMetadata {
                duration_ms: start.elapsed().as_millis() as u64,
                operation: operation.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn swagger_doc() -> &'static str {
        r#"{
  "swagger": "2.0",
  "info": { "title": "Test", "version": "1.0.0" },
  "paths": {}
}"#
    }

    fn openapi_doc() -> &'static str {
        r#"{
  "openapi": "3.0.0",
  "info": { "title": "Test", "version": "1.0.0" },
  "paths": {}
}"#
    }

    #[tokio::test]
    async fn can_handle_discovers_swagger_json() {
        let mut server = mockito::Server::new_async().await;
        let _swagger = server
            .mock("GET", "/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(swagger_doc())
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new();
        assert!(adapter.can_handle(&server.url()).await.unwrap());
    }

    #[tokio::test]
    async fn fetch_schema_uses_discovered_swagger_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _swagger = server
            .mock("GET", "/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(swagger_doc())
            .expect(2)
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new();
        assert!(adapter.can_handle(&server.url()).await.unwrap());
        let schema = adapter.fetch_schema(&server.url()).await.unwrap();
        assert_eq!(schema["swagger"], "2.0");
    }

    #[tokio::test]
    async fn fetch_schema_supports_api_docs_endpoint() {
        let mut server = mockito::Server::new_async().await;
        let _api_docs = server
            .mock("GET", "/api-docs")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(openapi_doc())
            .expect(2)
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new();
        assert!(adapter.can_handle(&server.url()).await.unwrap());
        let schema = adapter.fetch_schema(&server.url()).await.unwrap();
        assert_eq!(schema["openapi"], "3.0.0");
    }

    #[test]
    fn schema_candidates_do_not_append_to_schema_url() {
        let candidates = OpenAPIAdapter::schema_candidates("https://example.com/openapi.json");
        assert_eq!(candidates, vec!["https://example.com/openapi.json"]);
    }

    #[test]
    fn parse_operation_id_accepts_method_path_format() {
        let (method, path) = OpenAPIAdapter::parse_operation_id("post:/pet").unwrap();
        assert_eq!(method, "post");
        assert_eq!(path, "/pet");
    }

    #[test]
    fn parse_operation_id_rejects_legacy_display_format() {
        let err = OpenAPIAdapter::parse_operation_id("POST /pet").unwrap_err();
        assert!(
            err.to_string().contains("method:/path"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn operation_auth_requirement_uses_operation_security() {
        let root = json!({
            "openapi": "3.0.0",
            "security": [{"api_key": []}]
        });
        let operation = json!({
            "security": []
        });

        assert_eq!(
            OpenAPIAdapter::operation_auth_requirement(&operation, &root),
            OperationAuthRequirement::Public
        );
    }

    #[test]
    fn operation_auth_requirement_uses_root_security() {
        let root = json!({
            "swagger": "2.0",
            "securityDefinitions": {
                "api_key": {"type": "apiKey", "name": "X-API-Key", "in": "header"}
            },
            "security": [{"api_key": []}]
        });
        let operation = json!({});

        assert_eq!(
            OpenAPIAdapter::operation_auth_requirement(&operation, &root),
            OperationAuthRequirement::RequiresAuth
        );
    }

    #[test]
    fn operation_auth_requirement_public_when_other_operations_are_secured() {
        let root = json!({
            "openapi": "3.1.0",
            "paths": {
                "/public": {"get": {}},
                "/private": {"get": {"security": [{"api_key": []}]}}
            }
        });
        let operation = json!({});

        assert_eq!(
            OpenAPIAdapter::operation_auth_requirement(&operation, &root),
            OperationAuthRequirement::Public
        );
    }

    #[test]
    fn operation_auth_requirement_unknown_without_security_metadata() {
        let root = json!({
            "openapi": "3.0.0",
            "paths": {
                "/public": {"get": {}}
            }
        });
        let operation = json!({});

        assert_eq!(
            OpenAPIAdapter::operation_auth_requirement(&operation, &root),
            OperationAuthRequirement::Unknown
        );
    }

    #[tokio::test]
    async fn describe_operation_includes_expanded_request_body_schema() {
        let mut server = mockito::Server::new_async().await;
        let _openapi = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r##"{
  "openapi": "3.0.0",
  "info": { "title": "Test", "version": "1.0.0" },
  "paths": {
    "/pet": {
      "post": {
        "summary": "Add a new pet",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": { "$ref": "#/components/schemas/PetRequest" }
            }
          }
        },
        "responses": { "200": { "description": "ok" } }
      }
    }
  },
  "components": {
    "schemas": {
      "PetRequest": {
        "type": "object",
        "required": ["name"],
        "properties": {
          "name": { "type": "string" },
          "category": { "$ref": "#/components/schemas/Category" }
        }
      },
      "Category": {
        "type": "object",
        "properties": {
          "id": { "type": "integer" }
        }
      }
    }
  }
}"##,
            )
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new();
        let detail = adapter
            .describe_operation(&server.url(), "post:/pet")
            .await
            .unwrap();

        let input_schema = detail.input_schema.expect("input schema should exist");
        assert_eq!(input_schema["kind"], "openapi_request_body");
        assert_eq!(input_schema["required"], true);
        assert_eq!(
            input_schema["content"]["application/json"]["source_ref"],
            "#/components/schemas/PetRequest"
        );
        assert_eq!(
            input_schema["content"]["application/json"]["schema"]["properties"]["category"]
                ["properties"]["id"]["type"],
            "integer"
        );
    }

    #[tokio::test]
    async fn describe_operation_omits_input_schema_when_request_body_has_no_schema() {
        let mut server = mockito::Server::new_async().await;
        let _openapi = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r##"{
  "openapi": "3.0.0",
  "info": { "title": "Test", "version": "1.0.0" },
  "paths": {
    "/pet": {
      "post": {
        "summary": "Add a new pet",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "example": { "name": "doggie" }
            }
          }
        },
        "responses": { "200": { "description": "ok" } }
      }
    }
  }
}"##,
            )
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new();
        let detail = adapter
            .describe_operation(&server.url(), "post:/pet")
            .await
            .unwrap();
        assert!(detail.input_schema.is_none());
    }

    #[tokio::test]
    async fn execute_with_oauth_401_refreshes_and_retries() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/openapi.json", server.url());
        let token_endpoint = format!("{}/token", server.url());

        let _schema = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "openapi": "3.0.0",
                  "info": {"title":"Test","version":"1.0.0"},
                  "paths": {
                    "/health": {
                      "get": {
                        "responses": {
                          "200": {
                            "description":"ok",
                            "content": {"application/json": {"schema": {"type":"object"}}}
                          }
                        }
                      }
                    }
                  }
                }"#,
            )
            .expect(2)
            .create_async()
            .await;

        let _unauthorized = server
            .mock("GET", "/health")
            .match_header("authorization", "Bearer old-token")
            .with_status(401)
            .with_body("Unauthorized")
            .expect(1)
            .create_async()
            .await;

        let _refresh = server
            .mock("POST", "/token")
            .match_body(mockito::Matcher::Regex(
                "grant_type=refresh_token".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "access_token":"new-token",
                    "token_type":"Bearer",
                    "expires_in":3600,
                    "refresh_token":"refresh-2"
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let _success = server
            .mock("GET", "/health")
            .match_header("authorization", "Bearer new-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"status":"ok"}"#)
            .expect(1)
            .create_async()
            .await;

        let mut profile = Profile::new("old-token".to_string(), crate::auth::AuthType::OAuth);
        profile.oauth = Some(crate::auth::OAuthProfile {
            token_endpoint: Some(token_endpoint),
            refresh_token: Some("refresh-1".to_string()),
            access_token: Some("old-token".to_string()),
            token_type: Some("Bearer".to_string()),
            oauth_flow: Some(crate::auth::OAuthFlow::AuthorizationCode),
            ..Default::default()
        });

        let adapter = OpenAPIAdapter::new()
            .with_auth(profile)
            .with_schema_url_override(Some(schema_url));
        let result = adapter
            .execute(&server.url(), "get:/health", HashMap::new())
            .await
            .unwrap();
        assert_eq!(result.data["status"], "ok");
    }

    #[tokio::test]
    async fn schema_url_override_does_not_apply_business_query_auth() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/schema.json", server.url());

        let _schema = server
            .mock("GET", "/schema.json")
            .match_query(mockito::Matcher::Missing)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(openapi_doc())
            .expect(1)
            .create_async()
            .await;

        let mut profile = Profile::new("secret".to_string(), crate::auth::AuthType::ApiKey);
        profile.auth_query_params = Some(vec![crate::auth::AuthQueryParam::parse(
            "apiKey={{secret}}",
        )
        .unwrap()]);

        let adapter = OpenAPIAdapter::new()
            .with_auth(profile)
            .with_schema_url_override(Some(schema_url));

        assert!(adapter.can_handle(&server.url()).await.unwrap());
    }

    #[tokio::test]
    async fn execute_skips_all_auth_injection_for_public_operations() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/openapi.json", server.url());
        let _schema = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "openapi": "3.1.0",
                  "info": {"title":"Test","version":"1.0.0"},
                  "components": {
                    "securitySchemes": {
                      "api_key": {"type":"apiKey","in":"header","name":"X-MBX-APIKEY"}
                    }
                  },
                  "paths": {
                    "/public": {"get": {"responses": {"200": {"description":"ok"}}}},
                    "/signed": {
                      "get": {
                        "security": [{"api_key": []}],
                        "responses": {"200": {"description":"ok"}}
                      }
                    }
                  }
                }"#,
            )
            .expect(2)
            .create_async()
            .await;

        let _public = server
            .mock("GET", "/public")
            .match_query(mockito::Matcher::Missing)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .expect(1)
            .create_async()
            .await;

        let _signed = server
            .mock("GET", "/signed")
            .match_query(mockito::Matcher::UrlEncoded(
                "apiKey".to_string(),
                "secret".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .expect(1)
            .create_async()
            .await;

        let mut profile = Profile::new("secret".to_string(), crate::auth::AuthType::ApiKey);
        profile.auth_query_params = Some(vec![crate::auth::AuthQueryParam::parse(
            "apiKey={{secret}}",
        )
        .unwrap()]);

        let adapter = OpenAPIAdapter::new()
            .with_auth(profile)
            .with_schema_url_override(Some(schema_url));
        let public_result = adapter
            .execute(&server.url(), "get:/public", HashMap::new())
            .await
            .unwrap();
        assert_eq!(public_result.data["ok"], true);

        let signed_result = adapter
            .execute(&server.url(), "get:/signed", HashMap::new())
            .await
            .unwrap();
        assert_eq!(signed_result.data["ok"], true);
    }

    #[tokio::test]
    async fn execute_keeps_auth_injection_when_security_is_unknown() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/openapi.json", server.url());
        let _schema = server
            .mock("GET", "/openapi.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "openapi": "3.1.0",
                  "info": {"title":"Test","version":"1.0.0"},
                  "paths": {
                    "/public": {"get": {"responses": {"200": {"description":"ok"}}}}
                  }
                }"#,
            )
            .expect(2)
            .create_async()
            .await;

        let _public = server
            .mock("GET", "/public")
            .match_query(mockito::Matcher::UrlEncoded(
                "apiKey".to_string(),
                "secret".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .expect(1)
            .create_async()
            .await;

        let mut profile = Profile::new("secret".to_string(), crate::auth::AuthType::ApiKey);
        profile.auth_query_params = Some(vec![crate::auth::AuthQueryParam::parse(
            "apiKey={{secret}}",
        )
        .unwrap()]);
        let adapter = OpenAPIAdapter::new()
            .with_auth(profile)
            .with_schema_url_override(Some(schema_url));

        let result = adapter
            .execute(&server.url(), "get:/public", HashMap::new())
            .await
            .unwrap();
        assert_eq!(result.data["ok"], true);
    }

    #[tokio::test]
    async fn describe_operation_extracts_swagger2_body_input_schema() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/v2/swagger.json", server.url());
        let _schema = server
            .mock("GET", "/v2/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r##"{
                  "swagger": "2.0",
                  "host": "petstore.swagger.io",
                  "basePath": "/v2",
                  "schemes": ["https"],
                  "info": {"title":"Petstore","version":"1.0.0"},
                  "paths": {
                    "/pet": {
                      "post": {
                        "parameters": [
                          {
                            "in": "body",
                            "name": "body",
                            "required": true,
                            "description": "Pet payload",
                            "schema": { "$ref": "#/definitions/Pet" }
                          }
                        ],
                        "responses": {"200": {"description":"ok"}}
                      }
                    }
                  },
                  "definitions": {
                    "Pet": {
                      "type": "object",
                      "required": ["name"],
                      "properties": {
                        "name": {"type":"string"},
                        "photoUrls": {
                          "type":"array",
                          "items": {"type":"string"}
                        }
                      }
                    }
                  }
                }"##,
            )
            .expect(2)
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new().with_schema_url_override(Some(schema_url));
        let detail = adapter
            .describe_operation(&server.url(), "post:/pet")
            .await
            .unwrap();

        let input_schema = detail.input_schema.expect("input schema should exist");
        assert_eq!(input_schema["kind"], "openapi_request_body");
        assert_eq!(input_schema["required"], true);
        assert_eq!(
            input_schema["content"]["application/json"]["source_ref"],
            "#/definitions/Pet"
        );
        assert_eq!(
            input_schema["content"]["application/json"]["schema"]["properties"]["name"]["type"],
            "string"
        );
    }

    #[tokio::test]
    async fn execute_swagger2_body_uses_schema_url_base_and_supports_top_level_payload() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/v2/swagger.json", server.url());
        let host = server
            .url()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string();
        let schema = json!({
            "swagger": "2.0",
            "host": host,
            "basePath": "/v2",
            "schemes": ["http"],
            "info": {"title":"Petstore","version":"1.0.0"},
            "paths": {
                "/pet": {
                    "post": {
                        "parameters": [
                            {"in":"body","name":"body","required":true,"schema":{"type":"object"}}
                        ],
                        "responses": {"200":{"description":"ok"}}
                    }
                }
            }
        });
        let _schema = server
            .mock("GET", "/v2/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(schema.to_string())
            .expect(2)
            .create_async()
            .await;

        let _pet = server
            .mock("POST", "/v2/pet")
            .match_body(mockito::Matcher::JsonString(
                r#"{"name":"doggie","photoUrls":["x"]}"#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .expect(1)
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new().with_schema_url_override(Some(schema_url.clone()));

        let mut args = HashMap::new();
        args.insert("name".to_string(), Value::String("doggie".to_string()));
        args.insert(
            "photoUrls".to_string(),
            Value::Array(vec![Value::String("x".to_string())]),
        );
        let result = adapter
            .execute(&schema_url, "post:/pet", args)
            .await
            .unwrap();
        assert_eq!(result.data["ok"], true);
    }

    #[tokio::test]
    async fn execute_swagger2_form_data_uses_urlencoded_body() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/v2/swagger.json", server.url());
        let host = server
            .url()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string();
        let schema = json!({
            "swagger": "2.0",
            "host": host,
            "basePath": "/v2",
            "schemes": ["http"],
            "consumes": ["application/x-www-form-urlencoded"],
            "info": {"title":"Petstore","version":"1.0.0"},
            "paths": {
                "/pet/{petId}": {
                    "post": {
                        "parameters": [
                            {"name":"petId","in":"path","required":true,"type":"integer","format":"int64"},
                            {"name":"additionalMetadata","in":"formData","required":true,"type":"string"}
                        ],
                        "responses": {"200":{"description":"ok"}}
                    }
                }
            }
        });
        let _schema = server
            .mock("GET", "/v2/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(schema.to_string())
            .expect(2)
            .create_async()
            .await;

        let _update = server
            .mock("POST", "/v2/pet/123")
            .match_body(mockito::Matcher::UrlEncoded(
                "additionalMetadata".to_string(),
                "meta".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .expect(1)
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new().with_schema_url_override(Some(schema_url.clone()));

        let mut args = HashMap::new();
        args.insert("petId".to_string(), Value::Number(123.into()));
        args.insert(
            "additionalMetadata".to_string(),
            Value::String("meta".to_string()),
        );
        let result = adapter
            .execute(&schema_url, "post:/pet/{petId}", args)
            .await
            .unwrap();
        assert_eq!(result.data["ok"], true);
    }

    #[tokio::test]
    async fn execute_swagger2_multipart_file_returns_unsupported_error() {
        let mut server = mockito::Server::new_async().await;
        let schema_url = format!("{}/v2/swagger.json", server.url());
        let _schema = server
            .mock("GET", "/v2/swagger.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                  "swagger": "2.0",
                  "host": "127.0.0.1:1234",
                  "basePath": "/v2",
                  "schemes": ["http"],
                  "paths": {
                    "/pet/{petId}/uploadImage": {
                      "post": {
                        "consumes": ["multipart/form-data"],
                        "parameters": [
                          {"name":"petId","in":"path","required":true,"type":"integer","format":"int64"},
                          {"name":"file","in":"formData","required":false,"type":"file"}
                        ],
                        "responses": {"200":{"description":"ok"}}
                      }
                    }
                  }
                }"#,
            )
            .expect(2)
            .create_async()
            .await;

        let adapter = OpenAPIAdapter::new().with_schema_url_override(Some(schema_url.clone()));

        let mut args = HashMap::new();
        args.insert("petId".to_string(), Value::Number(123.into()));
        args.insert("file".to_string(), Value::String("dummy.txt".to_string()));
        let err = adapter
            .execute(&schema_url, "post:/pet/{petId}/uploadImage", args)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("multipart/form-data"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn operation_base_url_uses_swagger_host_and_base_path() {
        let schema = json!({
            "swagger": "2.0",
            "host": "api.example.com:8443",
            "basePath": "/v2",
            "schemes": ["https"]
        });
        let base =
            OpenAPIAdapter::operation_base_url("https://ignored.example.com/swagger.json", &schema);
        assert_eq!(base, "https://api.example.com:8443/v2");
    }

    #[test]
    fn prepare_request_prefers_operation_parameters_over_path_item_parameters() {
        let root = json!({});
        let path_item = json!({
            "parameters": [
                {"name": "expand", "in": "query", "required": false}
            ]
        });
        let operation = json!({
            "parameters": [
                {"name": "expand", "in": "query", "required": true}
            ]
        });

        let err = OpenAPIAdapter::prepare_request(
            "get",
            "https://example.com",
            "/pets",
            &path_item,
            &operation,
            &root,
            &HashMap::new(),
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("Missing required parameter 'expand'"));
    }

    #[test]
    fn prepare_request_requires_and_encodes_path_parameters() {
        let root = json!({});
        let path_item = json!({});
        let operation = json!({
            "parameters": [
                {"name": "pet_id", "in": "path"}
            ]
        });

        let err = OpenAPIAdapter::prepare_request(
            "get",
            "https://example.com",
            "/pets/{pet_id}",
            &path_item,
            &operation,
            &root,
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("Missing required parameter 'pet_id'"));

        let mut args = HashMap::new();
        args.insert(
            "pet_id".to_string(),
            Value::String("cats/dogs?x#y".to_string()),
        );
        let prepared = OpenAPIAdapter::prepare_request(
            "get",
            "https://example.com",
            "/pets/{pet_id}",
            &path_item,
            &operation,
            &root,
            &args,
        )
        .unwrap();

        assert_eq!(prepared.url, "https://example.com/pets/cats%2Fdogs%3Fx%23y");
    }

    #[test]
    fn prepare_request_splits_query_params_and_json_body_by_schema() {
        let root = json!({});
        let path_item = json!({});
        let operation = json!({
            "parameters": [
                {"name": "symbol", "in": "query", "required": true},
                {"name": "x-trace-id", "in": "header"}
            ],
            "requestBody": {
                "content": {
                    "application/json": {
                        "schema": {"type": "object"}
                    }
                }
            }
        });

        let mut args = HashMap::new();
        args.insert("symbol".to_string(), Value::String("BTCUSDT".to_string()));
        args.insert(
            "x-trace-id".to_string(),
            Value::String("trace-1".to_string()),
        );
        args.insert("side".to_string(), Value::String("BUY".to_string()));
        args.insert("quantity".to_string(), Value::String("1".to_string()));

        let prepared = OpenAPIAdapter::prepare_request(
            "post",
            "https://example.com",
            "/orders",
            &path_item,
            &operation,
            &root,
            &args,
        )
        .unwrap();

        assert_eq!(prepared.url, "https://example.com/orders");
        assert_eq!(
            prepared.query_pairs,
            vec![("symbol".to_string(), "BTCUSDT".to_string())]
        );
        assert_eq!(
            prepared.headers,
            vec![("x-trace-id".to_string(), "trace-1".to_string())]
        );
        assert_eq!(
            prepared.json_body,
            Some(json!({"quantity": "1", "side": "BUY"}))
        );
        assert!(prepared.form_body.is_none());
    }

    #[test]
    fn prepare_request_uses_implicit_json_body_for_post_without_schema_hints() {
        let root = json!({});
        let path_item = json!({});
        let operation = json!({});

        let mut args = HashMap::new();
        args.insert("name".to_string(), Value::String("John".to_string()));
        args.insert(
            "email".to_string(),
            Value::String("john@example.com".to_string()),
        );

        let prepared = OpenAPIAdapter::prepare_request(
            "post",
            "https://example.com",
            "/users",
            &path_item,
            &operation,
            &root,
            &args,
        )
        .unwrap();

        assert_eq!(prepared.url, "https://example.com/users");
        assert!(prepared.query_pairs.is_empty());
        assert_eq!(
            prepared.json_body,
            Some(json!({"email": "john@example.com", "name": "John"}))
        );
    }
}
