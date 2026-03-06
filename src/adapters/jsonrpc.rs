//! JSON-RPC adapter with OpenRPC-based discovery.
//!
//! This adapter intentionally prioritizes discoverable JSON-RPC services:
//! - `rpc.discover` (OpenRPC service discovery)
//! - static OpenRPC documents (`openrpc.json`)

use super::{
    Adapter, ExecutionMetadata, ExecutionResult, Operation, OperationDetail, Parameter,
    ProtocolType,
};
use crate::auth::{oauth, AuthType, Profile};
use crate::error::UxcError;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};

#[derive(Clone)]
struct ResolvedOpenRpc {
    rpc_url: String,
    schema: Value,
}

pub struct JsonRpcAdapter {
    client: reqwest::Client,
    cache: Option<Arc<dyn crate::cache::Cache>>,
    auth_profile: Option<Profile>,
    runtime_auth_profile: Arc<Mutex<Option<Profile>>>,
    oauth_refresh_lock: Arc<Mutex<()>>,
    force_refresh_schema: bool,
    discovered: Arc<RwLock<HashMap<String, ResolvedOpenRpc>>>,
    next_id: Arc<Mutex<i64>>,
}

impl JsonRpcAdapter {
    const OPENRPC_DOC_SUFFIX: &'static str = "/openrpc.json";
    const OPENRPC_WELL_KNOWN_SUFFIX: &'static str = "/.well-known/openrpc.json";

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            cache: None,
            auth_profile: None,
            runtime_auth_profile: Arc::new(Mutex::new(None)),
            oauth_refresh_lock: Arc::new(Mutex::new(())),
            force_refresh_schema: false,
            discovered: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(1)),
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

    fn is_http_url(url: &str) -> bool {
        let lower = url.to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    }

    fn is_openrpc_document(body: &Value) -> bool {
        body.get("openrpc").and_then(|v| v.as_str()).is_some()
            && body.get("methods").and_then(|v| v.as_array()).is_some()
    }

    fn schema_candidates(url: &str) -> Vec<String> {
        let normalized = Self::normalized_url(url);
        if normalized.ends_with(Self::OPENRPC_DOC_SUFFIX)
            || normalized.ends_with(Self::OPENRPC_WELL_KNOWN_SUFFIX)
        {
            return vec![normalized];
        }

        let mut candidates = vec![
            format!("{}{}", normalized, Self::OPENRPC_DOC_SUFFIX),
            format!("{}{}", normalized, Self::OPENRPC_WELL_KNOWN_SUFFIX),
        ];
        candidates.sort();
        candidates.dedup();
        candidates
    }

    fn schema_type_hint(schema: &Value) -> String {
        if let Some(type_name) = schema.get("type").and_then(|t| t.as_str()) {
            return type_name.to_string();
        }
        if schema.get("properties").is_some()
            || schema.get("allOf").is_some()
            || schema.get("oneOf").is_some()
            || schema.get("anyOf").is_some()
        {
            return "object".to_string();
        }
        if schema.get("items").is_some() {
            return "array".to_string();
        }
        "unknown".to_string()
    }

    fn parse_parameters(method: &Value) -> Vec<Parameter> {
        method
            .get("params")
            .and_then(|v| v.as_array())
            .map(|params| {
                params
                    .iter()
                    .filter_map(|param| {
                        let name = param.get("name").and_then(|v| v.as_str())?;
                        let param_type = param
                            .get("schema")
                            .map(Self::schema_type_hint)
                            .unwrap_or_else(|| "unknown".to_string());
                        let required = param
                            .get("required")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let description = param
                            .get("description")
                            .or_else(|| param.get("summary"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        Some(Parameter {
                            name: name.to_string(),
                            param_type,
                            required,
                            description,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn parse_return_type(method: &Value) -> Option<String> {
        let result = method.get("result")?;

        if let Some(schema) = result.get("schema") {
            return Some(Self::schema_type_hint(schema));
        }

        if let Some(name) = result.get("name").and_then(|v| v.as_str()) {
            return Some(name.to_string());
        }

        Some("unknown".to_string())
    }

    fn method_description(method: &Value) -> Option<String> {
        method
            .get("description")
            .or_else(|| method.get("summary"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn method_to_operation(method: &Value) -> Option<Operation> {
        let method_name = method.get("name").and_then(|v| v.as_str())?;
        Some(Operation {
            operation_id: method_name.to_string(),
            display_name: method_name.to_string(),
            description: Self::method_description(method),
            parameters: Self::parse_parameters(method),
            return_type: Self::parse_return_type(method),
        })
    }

    fn find_method<'a>(schema: &'a Value, operation: &str) -> Option<&'a Value> {
        schema
            .get("methods")
            .and_then(|v| v.as_array())
            .and_then(|methods| {
                methods.iter().find(|method| {
                    method
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|name| name == operation)
                        .unwrap_or(false)
                })
            })
    }

    fn build_operation_input_schema(method: &Value) -> Value {
        let mut schema = Map::new();
        schema.insert(
            "kind".to_string(),
            Value::String("openrpc_method".to_string()),
        );
        schema.insert(
            "paramStructure".to_string(),
            method
                .get("paramStructure")
                .cloned()
                .unwrap_or_else(|| Value::String("either".to_string())),
        );
        schema.insert(
            "params".to_string(),
            method
                .get("params")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
        schema.insert(
            "result".to_string(),
            method.get("result").cloned().unwrap_or(Value::Null),
        );
        Value::Object(schema)
    }

    fn ordered_parameter_specs(method: &Value) -> Vec<(String, bool)> {
        method
            .get("params")
            .and_then(|v| v.as_array())
            .map(|params| {
                params
                    .iter()
                    .filter_map(|param| {
                        let name = param.get("name").and_then(|v| v.as_str())?;
                        let required = param
                            .get("required")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        Some((name.to_string(), required))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn build_positional_params(
        ordered_specs: &[(String, bool)],
        args: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        if ordered_specs.is_empty() {
            return Ok(None);
        }

        for (name, required) in ordered_specs {
            if *required && !args.contains_key(name) {
                return Err(UxcError::InvalidArguments(format!(
                    "Missing required parameter: {}",
                    name
                ))
                .into());
            }
        }

        let mut last_used_index = None;
        for (idx, (name, _)) in ordered_specs.iter().enumerate() {
            if args.contains_key(name) {
                last_used_index = Some(idx);
            }
        }

        let Some(last_used_index) = last_used_index else {
            return Ok(None);
        };

        let mut items = Vec::new();
        for (name, required) in ordered_specs.iter().take(last_used_index + 1) {
            match args.get(name) {
                Some(value) => items.push(value.clone()),
                None if *required => {
                    return Err(UxcError::InvalidArguments(format!(
                        "Missing required parameter: {}",
                        name
                    ))
                    .into())
                }
                None => items.push(Value::Null),
            }
        }

        Ok(Some(Value::Array(items)))
    }

    fn build_params(method: &Value, args: &HashMap<String, Value>) -> Result<Option<Value>> {
        let ordered_specs = Self::ordered_parameter_specs(method);

        if !ordered_specs.is_empty() {
            let known: HashSet<&str> = ordered_specs
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            let unknown = args
                .keys()
                .filter(|key| !known.contains(key.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !unknown.is_empty() {
                return Err(UxcError::InvalidArguments(format!(
                    "Unknown parameter(s): {}",
                    unknown.join(", ")
                ))
                .into());
            }
        }

        if args.is_empty() {
            for (name, required) in &ordered_specs {
                if *required {
                    return Err(UxcError::InvalidArguments(format!(
                        "Missing required parameter: {}",
                        name
                    ))
                    .into());
                }
            }
            return Ok(None);
        }

        let structure = method
            .get("paramStructure")
            .and_then(|v| v.as_str())
            .unwrap_or("either");

        match structure {
            "by-name" => Ok(Some(Value::Object(args.clone().into_iter().collect()))),
            "by-position" => {
                if let Some(positional) = Self::build_positional_params(&ordered_specs, args)? {
                    Ok(Some(positional))
                } else {
                    Ok(Some(Value::Object(args.clone().into_iter().collect())))
                }
            }
            _ => {
                if let Some(positional) = Self::build_positional_params(&ordered_specs, args)? {
                    return Ok(Some(positional));
                }
                Ok(Some(Value::Object(args.clone().into_iter().collect())))
            }
        }
    }

    fn strip_suffix(url: &str, suffix: &str) -> Option<String> {
        url.strip_suffix(suffix)
            .map(|stripped| stripped.trim_end_matches('/').to_string())
    }

    fn default_rpc_url(input_url: &str, schema_url: &str) -> String {
        let normalized_input = Self::normalized_url(input_url);

        if let Some(base) = Self::strip_suffix(&normalized_input, Self::OPENRPC_WELL_KNOWN_SUFFIX) {
            if !base.is_empty() {
                return base;
            }
        }

        if let Some(base) = Self::strip_suffix(&normalized_input, Self::OPENRPC_DOC_SUFFIX) {
            if !base.is_empty() {
                return base;
            }
        }

        if let Some(base) = Self::strip_suffix(schema_url, Self::OPENRPC_WELL_KNOWN_SUFFIX) {
            if !base.is_empty() {
                return base;
            }
        }

        if let Some(base) = Self::strip_suffix(schema_url, Self::OPENRPC_DOC_SUFFIX) {
            if !base.is_empty() {
                return base;
            }
        }

        normalized_input
    }

    fn resolve_rpc_url_from_openrpc(schema: &Value, schema_url: &str, input_url: &str) -> String {
        if let Some(server_url) = schema
            .get("servers")
            .and_then(|v| v.as_array())
            .and_then(|servers| servers.first())
            .and_then(|server| server.get("url"))
            .and_then(|url| url.as_str())
        {
            if let Ok(parsed) = url::Url::parse(server_url) {
                if parsed.scheme() == "http" || parsed.scheme() == "https" {
                    return server_url.to_string();
                }
            }

            if let Ok(base) = url::Url::parse(schema_url) {
                if let Ok(joined) = base.join(server_url) {
                    return joined.to_string();
                }
            }

            if let Ok(base) = url::Url::parse(input_url) {
                if let Ok(joined) = base.join(server_url) {
                    return joined.to_string();
                }
            }
        }

        Self::default_rpc_url(input_url, schema_url)
    }

    async fn discover_via_rpc_discover(&self, url: &str) -> Result<Option<ResolvedOpenRpc>> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "rpc.discover",
            "params": []
        });

        let response = match self
            .send_with_oauth_retry(|profile| {
                let req = self
                    .client
                    .post(url)
                    .timeout(std::time::Duration::from_secs(3))
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&request);
                Self::apply_auth_profile(req, profile)
            })
            .await
        {
            Ok(response) => response,
            Err(_) => return Ok(None),
        };

        if !response.status().is_success() {
            return Ok(None);
        }

        let body = match response.json::<Value>().await {
            Ok(body) => body,
            Err(_) => return Ok(None),
        };

        let Some(result) = body.get("result") else {
            return Ok(None);
        };

        if body.get("error").is_some() || !Self::is_openrpc_document(result) {
            return Ok(None);
        }

        Ok(Some(ResolvedOpenRpc {
            rpc_url: url.to_string(),
            schema: result.clone(),
        }))
    }

    async fn discover_via_schema_urls(&self, url: &str) -> Result<Option<ResolvedOpenRpc>> {
        for schema_url in Self::schema_candidates(url) {
            let response = match self
                .send_with_oauth_retry(|profile| {
                    let req = self
                        .client
                        .get(&schema_url)
                        .timeout(std::time::Duration::from_secs(3))
                        .header("Accept", "application/json");
                    Self::apply_auth_profile(req, profile)
                })
                .await
            {
                Ok(response) => response,
                Err(_) => continue,
            };

            if !response.status().is_success() {
                continue;
            }

            let body = match response.json::<Value>().await {
                Ok(body) => body,
                Err(_) => continue,
            };

            if !Self::is_openrpc_document(&body) {
                continue;
            }

            let rpc_url = Self::resolve_rpc_url_from_openrpc(&body, &schema_url, url);
            return Ok(Some(ResolvedOpenRpc {
                rpc_url,
                schema: body,
            }));
        }

        Ok(None)
    }

    async fn discover_openrpc(&self, url: &str) -> Result<Option<ResolvedOpenRpc>> {
        if !Self::is_http_url(url) {
            return Ok(None);
        }

        let normalized = Self::normalized_url(url);

        {
            let cache = self.discovered.read().await;
            if let Some(found) = cache.get(&normalized) {
                return Ok(Some(found.clone()));
            }
        }

        let discovered = if let Some(found) = self.discover_via_rpc_discover(&normalized).await? {
            Some(found)
        } else {
            self.discover_via_schema_urls(&normalized).await?
        };

        if let Some(found) = discovered {
            let mut cache = self.discovered.write().await;
            cache.insert(normalized, found.clone());
            return Ok(Some(found));
        }

        Ok(None)
    }

    async fn resolve_rpc_url(&self, url: &str) -> Result<String> {
        let normalized = Self::normalized_url(url);

        {
            let cache = self.discovered.read().await;
            if let Some(found) = cache.get(&normalized) {
                return Ok(found.rpc_url.clone());
            }
        }

        if let Some(found) = self.discover_openrpc(&normalized).await? {
            return Ok(found.rpc_url);
        }

        Ok(Self::default_rpc_url(&normalized, &normalized))
    }

    async fn next_request_id(&self) -> i64 {
        let mut next = self.next_id.lock().await;
        let id = *next;
        *next += 1;
        id
    }

    async fn execute_jsonrpc(
        &self,
        rpc_url: &str,
        operation: &str,
        params: Option<Value>,
    ) -> Result<Value> {
        let mut request = Map::new();
        request.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
        request.insert(
            "id".to_string(),
            Value::Number(serde_json::Number::from(self.next_request_id().await)),
        );
        request.insert("method".to_string(), Value::String(operation.to_string()));
        if let Some(params) = params {
            request.insert("params".to_string(), params);
        }

        let response = self
            .send_with_oauth_retry(|profile| {
                let req = self
                    .client
                    .post(rpc_url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .json(&request);
                Self::apply_auth_profile(req, profile)
            })
            .await
            .context("Failed to send JSON-RPC request")?;

        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            bail!(
                "JSON-RPC server returned HTTP error: {} - {}",
                status,
                body_text
            );
        }

        let body: Value = serde_json::from_str(&body_text)
            .with_context(|| format!("Failed to parse JSON-RPC response: {}", body_text))?;

        if body.is_array() {
            bail!("JSON-RPC batch responses are not supported in this command");
        }

        let Some(obj) = body.as_object() else {
            bail!("Invalid JSON-RPC response: expected object");
        };

        if let Some(err) = obj.get("error").and_then(|v| v.as_object()) {
            let code = err
                .get("code")
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let message = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown JSON-RPC error");
            let data = err.get("data").cloned().unwrap_or(Value::Null);

            if data.is_null() {
                bail!("JSON-RPC error {}: {}", code, message);
            }
            bail!("JSON-RPC error {}: {} (data: {})", code, message, data);
        }

        obj.get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Invalid JSON-RPC response: missing result field"))
    }
}

impl Default for JsonRpcAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for JsonRpcAdapter {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::JsonRpc
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        Ok(self.discover_openrpc(url).await?.is_some())
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        let normalized = Self::normalized_url(url);

        {
            let discovered = self.discovered.read().await;
            if let Some(found) = discovered.get(&normalized) {
                return Ok(found.schema.clone());
            }
        }

        if !self.force_refresh_schema {
            if let Some(cache) = &self.cache {
                match cache.get(url)? {
                    crate::cache::CacheResult::Hit(schema) => {
                        debug!("JSON-RPC cache hit for: {}", url);
                        return Ok(schema);
                    }
                    crate::cache::CacheResult::Bypassed => {
                        debug!("JSON-RPC cache bypassed for: {}", url);
                    }
                    crate::cache::CacheResult::Miss => {
                        debug!("JSON-RPC cache miss for: {}", url);
                    }
                }
            }
        }

        let discovered = self.discover_openrpc(url).await?.ok_or_else(|| {
            UxcError::SchemaRetrievalFailed(format!("OpenRPC schema not found for {}", url))
        })?;

        if let Some(cache) = &self.cache {
            if let Err(e) = cache.put(url, &discovered.schema) {
                debug!("Failed to cache JSON-RPC schema: {}", e);
            } else {
                info!("Cached JSON-RPC schema for: {}", url);
            }
        }

        Ok(discovered.schema)
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        let schema = self.fetch_schema(url).await?;

        let methods = schema
            .get("methods")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                UxcError::SchemaRetrievalFailed("OpenRPC schema missing methods".to_string())
            })?;

        Ok(methods
            .iter()
            .filter_map(Self::method_to_operation)
            .collect::<Vec<_>>())
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        let schema = self.fetch_schema(url).await?;
        let method = Self::find_method(&schema, operation)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;

        Ok(OperationDetail {
            operation_id: operation.to_string(),
            display_name: operation.to_string(),
            description: Self::method_description(method),
            parameters: Self::parse_parameters(method),
            return_type: Self::parse_return_type(method),
            input_schema: Some(Self::build_operation_input_schema(method)),
        })
    }

    async fn execute(
        &self,
        url: &str,
        operation: &str,
        args: HashMap<String, Value>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let schema = self.fetch_schema(url).await?;
        let method = Self::find_method(&schema, operation)
            .ok_or_else(|| UxcError::OperationNotFound(operation.to_string()))?;

        let params = Self::build_params(method, &args)?;
        let rpc_url = self.resolve_rpc_url(url).await?;
        let data = self.execute_jsonrpc(&rpc_url, operation, params).await?;

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

    fn openrpc_schema_with_structure(structure: &str) -> Value {
        json!({
            "openrpc": "1.3.2",
            "methods": [
                {
                    "name": "subtract",
                    "paramStructure": structure,
                    "params": [
                        {
                            "name": "minuend",
                            "required": true,
                            "schema": { "type": "number" }
                        },
                        {
                            "name": "subtrahend",
                            "required": true,
                            "schema": { "type": "number" }
                        }
                    ],
                    "result": {
                        "name": "difference",
                        "schema": { "type": "number" }
                    }
                }
            ]
        })
    }

    #[test]
    fn build_params_prefers_positional_for_either() {
        let schema = openrpc_schema_with_structure("either");
        let method = JsonRpcAdapter::find_method(&schema, "subtract").unwrap();

        let mut args = HashMap::new();
        args.insert("minuend".to_string(), json!(42));
        args.insert("subtrahend".to_string(), json!(23));

        let params = JsonRpcAdapter::build_params(method, &args).unwrap();
        assert_eq!(params, Some(json!([42, 23])));
    }

    #[test]
    fn build_params_uses_object_for_by_name() {
        let schema = openrpc_schema_with_structure("by-name");
        let method = JsonRpcAdapter::find_method(&schema, "subtract").unwrap();

        let mut args = HashMap::new();
        args.insert("minuend".to_string(), json!(42));
        args.insert("subtrahend".to_string(), json!(23));

        let params = JsonRpcAdapter::build_params(method, &args).unwrap();
        assert_eq!(params, Some(json!({"minuend": 42, "subtrahend": 23})));
    }

    #[test]
    fn default_rpc_url_trims_openrpc_suffix() {
        assert_eq!(
            JsonRpcAdapter::default_rpc_url(
                "https://example.com/openrpc.json",
                "https://example.com/openrpc.json"
            ),
            "https://example.com"
        );
        assert_eq!(
            JsonRpcAdapter::default_rpc_url(
                "https://example.com/.well-known/openrpc.json",
                "https://example.com/.well-known/openrpc.json"
            ),
            "https://example.com"
        );
    }

    #[tokio::test]
    async fn execute_with_oauth_401_refreshes_and_retries() {
        let mut server = mockito::Server::new_async().await;
        let token_endpoint = format!("{}/token", server.url());

        let _discover = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer old-token")
            .match_body(mockito::Matcher::Regex(
                "\"method\":\"rpc.discover\"".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "jsonrpc":"2.0",
                    "id":1,
                    "result":{
                        "openrpc":"1.3.2",
                        "methods":[{"name":"health","params":[],"result":{"name":"result"}}]
                    }
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let _unauthorized = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer old-token")
            .match_body(mockito::Matcher::Regex("\"method\":\"health\"".to_string()))
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
            .mock("POST", "/")
            .match_header("authorization", "Bearer new-token")
            .match_body(mockito::Matcher::Regex("\"method\":\"health\"".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":2,"result":{"status":"ok"}}"#)
            .expect(1)
            .create_async()
            .await;

        let mut profile =
            crate::auth::Profile::new("old-token".to_string(), crate::auth::AuthType::OAuth);
        profile.oauth = Some(crate::auth::OAuthProfile {
            token_endpoint: Some(token_endpoint),
            refresh_token: Some("refresh-1".to_string()),
            access_token: Some("old-token".to_string()),
            token_type: Some("Bearer".to_string()),
            oauth_flow: Some(crate::auth::OAuthFlow::AuthorizationCode),
            ..Default::default()
        });

        let adapter = JsonRpcAdapter::new().with_auth(profile);
        let result = adapter
            .execute(&server.url(), "health", HashMap::new())
            .await
            .unwrap();
        assert_eq!(result.data["status"], "ok");
    }
}
