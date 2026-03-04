//! OpenAPI/Swagger adapter

use super::{
    Adapter, ExecutionMetadata, ExecutionResult, Operation, OperationDetail, Parameter,
    ProtocolType,
};
use crate::auth::Profile;
use crate::error::UxcError;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

pub struct OpenAPIAdapter {
    client: reqwest::Client,
    cache: Option<Arc<dyn crate::cache::Cache>>,
    auth_profile: Option<Profile>,
    discovered_schema_urls: Arc<RwLock<HashMap<String, String>>>,
    schema_url_override: Option<String>,
    force_refresh_schema: bool,
}

impl OpenAPIAdapter {
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
            .client
            .get(schema_url)
            .timeout(std::time::Duration::from_secs(10))
            .header("Accept", "application/json")
            .send()
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
                .client
                .get(&full_url)
                .timeout(std::time::Duration::from_secs(2))
                .header("Accept", "application/json")
                .send()
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
            path_item.get("parameters").and_then(|p| p.as_array()),
            operation_spec.get("parameters").and_then(|p| p.as_array()),
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

    fn extract_request_body_input_schema(operation_spec: &Value, root: &Value) -> Option<Value> {
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
        let resp = self.client.get(&schema_url).send().await?;
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
        let input_schema = Self::extract_request_body_input_schema(operation_spec, &schema);

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

        let full_url = format!("{}{}", url.trim_end_matches('/'), path);

        // Determine if the method supports request body
        // GET, HEAD, OPTIONS, TRACE typically don't send request bodies
        // DELETE can but rarely does; POST, PUT, PATCH commonly do
        let has_body_support = matches!(method.as_str(), "post" | "put" | "patch");

        let req = match method.as_str() {
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

        // Apply authentication if profile is set
        let req = if let Some(profile) = &self.auth_profile {
            crate::auth::apply_profile_auth_to_request(req, profile)?
        } else {
            req
        };

        // Apply arguments based on HTTP method semantics
        let req = if has_body_support {
            // For POST/PUT/PATCH, send JSON body
            req.json(&args)
        } else {
            // For GET/DELETE/HEAD/OPTIONS/TRACE, use query parameters
            req.query(&args)
        };

        let resp = req.send().await?;
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
                        full_url
                    )
                })?;

                if bytes.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_slice::<Value>(&bytes).with_context(|| {
                        format!(
                            "error decoding response body: HTTP {} from {}",
                            status.as_u16(),
                            full_url
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
}
