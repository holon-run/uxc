//! GraphQL adapter with introspection support
//!
//! This adapter provides full GraphQL support including:
//! - Schema introspection and discovery
//! - Query and mutation execution
//! - Variable binding and serialization
//! - Comprehensive error handling

use super::{
    Adapter, ExecutionMetadata, ExecutionResult, Operation, OperationDetail, Parameter,
    ProtocolType,
};
use crate::auth::{oauth, AuthType, Profile};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info};

pub struct GraphQLAdapter {
    client: reqwest::Client,
    cache: Option<Arc<dyn crate::cache::Cache>>,
    auth_profile: Option<Profile>,
    runtime_auth_profile: Arc<Mutex<Option<Profile>>>,
    oauth_refresh_lock: Arc<Mutex<()>>,
    force_refresh_schema: bool,
}

impl GraphQLAdapter {
    const MAX_INPUT_SCHEMA_DEPTH: usize = 8;
    const MAX_SELECTION_DEPTH: usize = 3;
    const MAX_SELECTION_FIELDS_PER_OBJECT: usize = 12;

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            cache: None,
            auth_profile: None,
            runtime_auth_profile: Arc::new(Mutex::new(None)),
            oauth_refresh_lock: Arc::new(Mutex::new(())),
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

    fn apply_auth_profile_to_url(url: &str, profile: Option<&Profile>) -> Result<String> {
        match profile {
            Some(profile) => crate::auth::apply_profile_auth_to_url(url, profile),
            None => Ok(url.to_string()),
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

    async fn send_graphql_request(
        &self,
        url: &str,
        payload: &Value,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response> {
        let mut profile = self.refresh_effective_oauth_profile(false).await?;
        let authed_url = Self::apply_auth_profile_to_url(url, profile.as_ref())?;

        let mut req = self
            .client
            .post(&authed_url)
            .header("Content-Type", "application/json");
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        req = Self::apply_auth_profile(req, profile.as_ref())?;

        let mut resp = req.json(payload).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && profile
                .as_ref()
                .is_some_and(|active| active.auth_type == AuthType::OAuth)
        {
            profile = self.refresh_effective_oauth_profile(true).await?;

            let mut retry_req = self
                .client
                .post(&authed_url)
                .header("Content-Type", "application/json");
            if let Some(timeout) = timeout {
                retry_req = retry_req.timeout(timeout);
            }
            retry_req = Self::apply_auth_profile(retry_req, profile.as_ref())?;
            resp = retry_req.json(payload).send().await?;
        }

        Ok(resp)
    }

    /// Execute a GraphQL query/mutation with optional variables
    async fn execute_graphql(
        &self,
        url: &str,
        query: &str,
        variables: Option<Value>,
        operation_name: Option<&str>,
    ) -> Result<Value> {
        let mut payload = serde_json::json!({
            "query": query
        });

        if let Some(vars) = variables {
            payload["variables"] = vars;
        }

        if let Some(op_name) = operation_name {
            payload["operationName"] = serde_json::json!(op_name);
        }

        let resp = self.send_graphql_request(url, &payload, None).await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_default();
            bail!(
                "GraphQL request failed with status {}: {}",
                status,
                error_text
            );
        }

        let body: Value = resp.json().await?;

        // Check for GraphQL errors
        if let Some(errors) = body.get("errors") {
            if let Some(error_array) = errors.as_array() {
                let error_messages: Vec<String> = error_array
                    .iter()
                    .map(|e| {
                        let message = e
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("Unknown error");
                        let mut error_str = format!("- {}", message);

                        // Add location info if available
                        if let Some(locations) = e.get("locations").and_then(|l| l.as_array()) {
                            for loc in locations.iter().take(3) {
                                if let (Some(line), Some(col)) = (
                                    loc.get("line").and_then(|l| l.as_i64()),
                                    loc.get("column").and_then(|c| c.as_i64()),
                                ) {
                                    error_str
                                        .push_str(&format!(" [line {}, column {}]", line, col));
                                }
                            }
                        }

                        // Add path info if available
                        if let Some(path) = e.get("path") {
                            error_str.push_str(&format!(" (path: {})", path));
                        }

                        error_str
                    })
                    .collect();

                bail!("GraphQL errors:\n{}", error_messages.join("\n"));
            }
        }

        Ok(body)
    }

    /// Get the full introspection query
    fn get_introspection_query() -> &'static str {
        r#"
            query IntrospectionQuery {
                __schema {
                    queryType {
                        name
                        description
                        fields {
                            ...FieldInfo
                        }
                    }
                    mutationType {
                        name
                        description
                        fields {
                            ...FieldInfo
                        }
                    }
                    subscriptionType {
                        name
                        description
                        fields {
                            ...FieldInfo
                        }
                    }
                    types {
                        name
                        kind
                        description
                        enumValues {
                            name
                            description
                        }
                        inputFields {
                            name
                            description
                            type {
                                ...TypeRef
                            }
                        }
                    }
                }
            }

            fragment FieldInfo on __Field {
                name
                description
                args {
                    name
                    description
                    defaultValue
                    type {
                        ...TypeRef
                    }
                }
                type {
                    ...TypeRef
                }
            }

            fragment TypeRef on __Type {
                kind
                name
                ofType {
                    kind
                    name
                    ofType {
                        kind
                        name
                        ofType {
                            kind
                            name
                            ofType {
                                kind
                                name
                                ofType {
                                    kind
                                    name
                                    ofType {
                                        kind
                                        name
                                        ofType {
                                            kind
                                            name
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        "#
    }

    /// Extract type name from a GraphQL type structure
    #[allow(dead_code)]
    fn extract_type_name(type_info: &Value) -> Option<String> {
        let kind = type_info.get("kind")?.as_str()?;

        match kind {
            "NON_NULL" | "LIST" => Self::extract_type_name(type_info.get("ofType")?),
            _ => type_info.get("name")?.as_str().map(|s| s.to_string()),
        }
    }

    /// Convert GraphQL type to readable string representation
    fn type_to_string(type_info: &Value) -> String {
        let kind = type_info
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("UNKNOWN");

        match kind {
            "NON_NULL" => {
                let inner = type_info.get("ofType");
                format!("{}!", Self::type_to_string(inner.unwrap_or(&Value::Null)))
            }
            "LIST" => {
                let inner = type_info.get("ofType");
                format!("[{}]", Self::type_to_string(inner.unwrap_or(&Value::Null)))
            }
            _ => {
                let name = type_info
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("Unknown");
                name.to_string()
            }
        }
    }

    /// Parse introspection schema into operations
    fn parse_schema_to_operations(schema: &Value) -> Result<Vec<Operation>> {
        let mut operations = Vec::new();

        let data = schema
            .get("data")
            .ok_or_else(|| anyhow!("Invalid introspection response: missing data"))?;

        let schema_obj = data
            .get("__schema")
            .ok_or_else(|| anyhow!("Invalid introspection response: missing __schema"))?;

        // Parse queries
        if let Some(query_type) = schema_obj.get("queryType") {
            if let Some(fields) = query_type.get("fields").and_then(|f| f.as_array()) {
                for field in fields {
                    let name = field
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();

                    let description = field
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string());

                    let parameters = Self::parse_field_args(field);

                    let return_type = field.get("type").map(Self::type_to_string);
                    let operation_id = format!("query/{}", name);

                    operations.push(Operation {
                        operation_id: operation_id.clone(),
                        display_name: operation_id,
                        description,
                        parameters,
                        return_type,
                    });
                }
            }
        }

        // Parse mutations
        if let Some(mutation_type) = schema_obj.get("mutationType") {
            if let Some(fields) = mutation_type.get("fields").and_then(|f| f.as_array()) {
                for field in fields {
                    let name = field
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();

                    let description = field
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string());

                    let parameters = Self::parse_field_args(field);

                    let return_type = field.get("type").map(Self::type_to_string);
                    let operation_id = format!("mutation/{}", name);

                    operations.push(Operation {
                        operation_id: operation_id.clone(),
                        display_name: operation_id,
                        description,
                        parameters,
                        return_type,
                    });
                }
            }
        }

        // Parse subscriptions
        if let Some(subscription_type) = schema_obj.get("subscriptionType") {
            if let Some(fields) = subscription_type.get("fields").and_then(|f| f.as_array()) {
                for field in fields {
                    let name = field
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();

                    let description = field
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string());

                    let parameters = Self::parse_field_args(field);

                    let return_type = field.get("type").map(Self::type_to_string);
                    let operation_id = format!("subscription/{}", name);

                    operations.push(Operation {
                        operation_id: operation_id.clone(),
                        display_name: operation_id,
                        description,
                        parameters,
                        return_type,
                    });
                }
            }
        }

        Ok(operations)
    }

    /// Parse field arguments into parameters
    fn parse_field_args(field: &Value) -> Vec<Parameter> {
        field
            .get("args")
            .and_then(|args| args.as_array())
            .map(|args| {
                args.iter()
                    .filter_map(|arg| {
                        let name = arg.get("name")?.as_str()?;
                        let type_info = arg.get("type")?;

                        Some(Parameter {
                            name: name.to_string(),
                            param_type: Self::type_to_string(type_info),
                            required: type_info.get("kind").and_then(|k| k.as_str())
                                == Some("NON_NULL"),
                            description: arg
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn find_operation_field<'a>(schema: &'a Value, operation: &str) -> Option<&'a Value> {
        let (root_key, field_name) = if let Some(name) = operation.strip_prefix("query/") {
            ("queryType", name)
        } else if let Some(name) = operation.strip_prefix("mutation/") {
            ("mutationType", name)
        } else if let Some(name) = operation.strip_prefix("subscription/") {
            ("subscriptionType", name)
        } else {
            return None;
        };

        let fields = schema
            .get("data")?
            .get("__schema")?
            .get(root_key)?
            .get("fields")?
            .as_array()?;

        fields
            .iter()
            .find(|field| field.get("name").and_then(|n| n.as_str()) == Some(field_name))
    }

    fn build_type_index(schema: &Value) -> HashMap<String, &Value> {
        let mut type_index = HashMap::new();
        if let Some(types) = schema
            .get("data")
            .and_then(|d| d.get("__schema"))
            .and_then(|s| s.get("types"))
            .and_then(|t| t.as_array())
        {
            for type_def in types {
                if let Some(name) = type_def.get("name").and_then(|n| n.as_str()) {
                    type_index.insert(name.to_string(), type_def);
                }
            }
        }
        type_index
    }

    fn scalar_schema(type_name: Option<&str>) -> Value {
        match type_name.unwrap_or("String") {
            "String" | "ID" => serde_json::json!({ "type": "string" }),
            "Int" => serde_json::json!({ "type": "integer" }),
            "Float" => serde_json::json!({ "type": "number" }),
            "Boolean" => serde_json::json!({ "type": "boolean" }),
            other => serde_json::json!({
                "type": "string",
                "x-graphql-scalar": other
            }),
        }
    }

    fn graphql_type_to_input_schema(
        type_info: &Value,
        type_index: &HashMap<String, &Value>,
        visiting: &mut HashSet<String>,
        depth: usize,
    ) -> (Value, bool) {
        if depth == 0 {
            return (serde_json::json!({}), false);
        }

        let kind = type_info
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or("UNKNOWN");

        match kind {
            "NON_NULL" => {
                let inner = type_info.get("ofType").unwrap_or(&Value::Null);
                let (schema, _) =
                    Self::graphql_type_to_input_schema(inner, type_index, visiting, depth - 1);
                (schema, true)
            }
            "LIST" => {
                let inner = type_info.get("ofType").unwrap_or(&Value::Null);
                let (items, _) =
                    Self::graphql_type_to_input_schema(inner, type_index, visiting, depth - 1);
                (
                    serde_json::json!({ "type": "array", "items": items }),
                    false,
                )
            }
            "SCALAR" => (
                Self::scalar_schema(type_info.get("name").and_then(|n| n.as_str())),
                false,
            ),
            "ENUM" => {
                let type_name = type_info.get("name").and_then(|n| n.as_str());
                if let Some(enum_def) = type_name.and_then(|name| type_index.get(name).copied()) {
                    let values = enum_def
                        .get("enumValues")
                        .and_then(|v| v.as_array())
                        .map(|vals| {
                            vals.iter()
                                .filter_map(|item| item.get("name").and_then(|n| n.as_str()))
                                .map(|s| Value::String(s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    return (
                        serde_json::json!({
                            "type": "string",
                            "enum": values
                        }),
                        false,
                    );
                }
                (serde_json::json!({ "type": "string" }), false)
            }
            "INPUT_OBJECT" => {
                let Some(type_name) = type_info.get("name").and_then(|n| n.as_str()) else {
                    return (serde_json::json!({ "type": "object" }), false);
                };

                if !visiting.insert(type_name.to_string()) {
                    return (
                        serde_json::json!({
                            "$ref": format!("graphql://{}", type_name)
                        }),
                        false,
                    );
                }

                let mut properties = Map::new();
                let mut required = Vec::new();
                if let Some(type_def) = type_index.get(type_name).copied() {
                    if let Some(fields) = type_def.get("inputFields").and_then(|f| f.as_array()) {
                        for field in fields {
                            let Some(name) = field.get("name").and_then(|n| n.as_str()) else {
                                continue;
                            };
                            let Some(field_type) = field.get("type") else {
                                continue;
                            };

                            let (mut field_schema, is_required) =
                                Self::graphql_type_to_input_schema(
                                    field_type,
                                    type_index,
                                    visiting,
                                    depth - 1,
                                );
                            if let Some(description) =
                                field.get("description").and_then(|d| d.as_str())
                            {
                                if let Value::Object(ref mut obj) = field_schema {
                                    obj.insert(
                                        "description".to_string(),
                                        Value::String(description.to_string()),
                                    );
                                }
                            }

                            properties.insert(name.to_string(), field_schema);
                            if is_required {
                                required.push(Value::String(name.to_string()));
                            }
                        }
                    }
                }
                visiting.remove(type_name);

                let mut object_schema = Map::new();
                object_schema.insert("type".to_string(), Value::String("object".to_string()));
                object_schema.insert("properties".to_string(), Value::Object(properties));
                object_schema.insert("additionalProperties".to_string(), Value::Bool(false));
                if !required.is_empty() {
                    object_schema.insert("required".to_string(), Value::Array(required));
                }
                (Value::Object(object_schema), false)
            }
            _ => {
                if let Some(type_name) = type_info.get("name").and_then(|n| n.as_str()) {
                    return (Self::scalar_schema(Some(type_name)), false);
                }
                let mut fallback = Map::new();
                fallback.insert("type".to_string(), Value::String("object".to_string()));
                if let Some(type_name) = type_info.get("name").and_then(|n| n.as_str()) {
                    fallback.insert(
                        "x-graphql-type".to_string(),
                        Value::String(type_name.to_string()),
                    );
                }
                (Value::Object(fallback), false)
            }
        }
    }

    fn build_operation_input_schema(schema: &Value, operation: &str) -> Option<Value> {
        let field = Self::find_operation_field(schema, operation)?;
        let type_index = Self::build_type_index(schema);

        let mut properties = Map::new();
        let mut required = Vec::new();
        if let Some(args) = field.get("args").and_then(|a| a.as_array()) {
            for arg in args {
                let Some(name) = arg.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                let Some(type_info) = arg.get("type") else {
                    continue;
                };

                let (mut schema, is_required) = Self::graphql_type_to_input_schema(
                    type_info,
                    &type_index,
                    &mut HashSet::new(),
                    Self::MAX_INPUT_SCHEMA_DEPTH,
                );

                if let Some(description) = arg.get("description").and_then(|d| d.as_str()) {
                    if let Value::Object(ref mut obj) = schema {
                        obj.insert(
                            "description".to_string(),
                            Value::String(description.to_string()),
                        );
                    }
                }

                properties.insert(name.to_string(), schema);
                if is_required {
                    required.push(Value::String(name.to_string()));
                }
            }
        }

        let mut input = Map::new();
        input.insert(
            "kind".to_string(),
            Value::String("graphql_arguments".to_string()),
        );
        input.insert("type".to_string(), Value::String("object".to_string()));
        input.insert("properties".to_string(), Value::Object(properties));
        input.insert("additionalProperties".to_string(), Value::Bool(false));
        if !required.is_empty() {
            input.insert("required".to_string(), Value::Array(required));
        }
        Some(Value::Object(input))
    }

    fn is_scalar_or_enum(type_info: &Value) -> bool {
        let kind = type_info.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "SCALAR" | "ENUM" => true,
            "NON_NULL" | "LIST" => type_info
                .get("ofType")
                .map(Self::is_scalar_or_enum)
                .unwrap_or(false),
            _ => false,
        }
    }

    fn named_type_name(type_info: &Value) -> Option<String> {
        let kind = type_info.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "NON_NULL" | "LIST" => type_info.get("ofType").and_then(Self::named_type_name),
            _ => type_info
                .get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string()),
        }
    }

    fn field_priority(name: &str, is_leaf: bool) -> i32 {
        if name == "nodes" {
            return 0;
        }
        if is_leaf
            && matches!(
                name,
                "id" | "identifier"
                    | "key"
                    | "name"
                    | "title"
                    | "url"
                    | "state"
                    | "status"
                    | "createdAt"
                    | "updatedAt"
            )
        {
            return 1;
        }
        if is_leaf {
            return 2;
        }
        if matches!(name, "state" | "assignee" | "team" | "project" | "pageInfo") {
            return 3;
        }
        4
    }

    fn build_selection_set_for_type(
        type_info: &Value,
        type_index: &HashMap<String, &Value>,
        visiting: &mut HashSet<String>,
        depth: usize,
    ) -> Option<String> {
        if depth == 0 {
            return Some("__typename".to_string());
        }

        let Some(type_name) = Self::named_type_name(type_info) else {
            return Some("__typename".to_string());
        };
        let Some(type_def) = type_index.get(&type_name).copied() else {
            return Some("__typename".to_string());
        };
        let fields = type_def.get("fields").and_then(|f| f.as_array())?;

        if !visiting.insert(type_name.clone()) {
            return Some("__typename".to_string());
        }

        let mut ranked: Vec<(usize, i32)> = fields
            .iter()
            .enumerate()
            .filter_map(|(idx, field)| {
                let name = field.get("name").and_then(|n| n.as_str())?;
                let field_type = field.get("type")?;
                let is_leaf = Self::is_scalar_or_enum(field_type);
                Some((idx, Self::field_priority(name, is_leaf)))
            })
            .collect();
        ranked.sort_by_key(|(idx, priority)| (*priority, *idx));

        let mut selections = Vec::new();
        for (idx, _) in ranked {
            if selections.len() >= Self::MAX_SELECTION_FIELDS_PER_OBJECT {
                break;
            }
            let field = &fields[idx];
            let Some(name) = field.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let Some(field_type) = field.get("type") else {
                continue;
            };

            if Self::is_scalar_or_enum(field_type) {
                selections.push(name.to_string());
                continue;
            }

            let child = Self::build_selection_set_for_type(
                field_type,
                type_index,
                visiting,
                depth.saturating_sub(1),
            );
            if let Some(child_selection) = child {
                selections.push(format!("{} {{ {} }}", name, child_selection));
            }
        }

        visiting.remove(&type_name);

        if selections.is_empty() {
            Some("__typename".to_string())
        } else {
            Some(selections.join(" "))
        }
    }

    fn default_selection_set(schema: &Value, operation: &str) -> String {
        let Some(field) = Self::find_operation_field(schema, operation) else {
            return "__typename".to_string();
        };
        let Some(return_type) = field.get("type") else {
            return "__typename".to_string();
        };
        let type_index = Self::build_type_index(schema);
        Self::build_selection_set_for_type(
            return_type,
            &type_index,
            &mut HashSet::new(),
            Self::MAX_SELECTION_DEPTH,
        )
        .unwrap_or_else(|| "__typename".to_string())
    }

    /// Find operation details from parsed operations
    fn find_operation(schema: &Value, operation: &str) -> Option<Operation> {
        let operations = Self::parse_schema_to_operations(schema).ok()?;
        operations
            .into_iter()
            .find(|op| op.operation_id == operation)
    }

    /// Determine operation type and name from operation string
    fn parse_operation_name(operation: &str) -> Result<(OperationType, String)> {
        if let Some(rest) = operation.strip_prefix("query/") {
            Ok((OperationType::Query, rest.to_string()))
        } else if let Some(rest) = operation.strip_prefix("mutation/") {
            Ok((OperationType::Mutation, rest.to_string()))
        } else if let Some(rest) = operation.strip_prefix("subscription/") {
            Ok((OperationType::Subscription, rest.to_string()))
        } else {
            bail!(
                "Invalid GraphQL operation ID '{}'. Use query/<field>, mutation/<field>, or subscription/<field>",
                operation
            )
        }
    }

    /// Build a GraphQL query string from operation name and selection set
    #[allow(dead_code)]
    fn build_query(
        op_type: OperationType,
        field_name: &str,
        selection_set: Option<&str>,
    ) -> String {
        let keyword = match op_type {
            OperationType::Query => "query",
            OperationType::Mutation => "mutation",
            OperationType::Subscription => "subscription",
        };

        if let Some(selection) = selection_set {
            format!("{} {{ {} {{ {} }} }}", keyword, field_name, selection)
        } else {
            format!("{} {{ {} }}", keyword, field_name)
        }
    }
}

impl Default for GraphQLAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
enum OperationType {
    Query,
    Mutation,
    Subscription,
}

#[async_trait]
impl Adapter for GraphQLAdapter {
    fn protocol_type(&self) -> ProtocolType {
        ProtocolType::GraphQL
    }

    async fn can_handle(&self, url: &str) -> Result<bool> {
        // Try GraphQL introspection with timeout
        let query = r#"
            {
                __schema {
                    queryType {
                        name
                    }
                }
            }
        "#;

        let payload = serde_json::json!({ "query": query });
        let resp = match self
            .send_graphql_request(url, &payload, Some(std::time::Duration::from_secs(2)))
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(false),
        };

        if !resp.status().is_success() {
            return Ok(false);
        }

        // Check if response is valid GraphQL with __schema
        if let Ok(body) = resp.json::<Value>().await {
            // A valid GraphQL introspection response should have data.__schema
            // or errors (which still indicates GraphQL)
            if body.get("data").is_some()
                || body.get("errors").is_some()
                || body.get("__schema").is_some()
            {
                return Ok(true);
            }
        }

        Ok(false)
    }

    async fn fetch_schema(&self, url: &str) -> Result<Value> {
        // Try cache first if available
        if !self.force_refresh_schema {
            if let Some(cache) = &self.cache {
                match cache.get(url)? {
                    crate::cache::CacheResult::Hit(schema) => {
                        debug!("GraphQL cache hit for: {}", url);
                        return Ok(schema);
                    }
                    crate::cache::CacheResult::Bypassed => {
                        debug!("GraphQL cache bypassed for: {}", url);
                    }
                    crate::cache::CacheResult::Miss => {
                        debug!("GraphQL cache miss for: {}", url);
                    }
                }
            }
        }

        // Fetch from remote
        let introspection_query = Self::get_introspection_query();

        let payload = serde_json::json!({ "query": introspection_query });
        let resp = self.send_graphql_request(url, &payload, None).await?;

        if !resp.status().is_success() {
            bail!("Failed to fetch GraphQL schema: HTTP {}", resp.status());
        }

        let body: Value = resp.json().await?;

        // Check for GraphQL errors in introspection
        if let Some(errors) = body.get("errors") {
            bail!(
                "GraphQL introspection failed: {}",
                serde_json::to_string_pretty(errors)?
            );
        }

        // Store in cache if available
        if let Some(cache) = &self.cache {
            if let Err(e) = cache.put(url, &body) {
                debug!("Failed to cache GraphQL schema: {}", e);
            } else {
                info!("Cached GraphQL schema for: {}", url);
            }
        }

        Ok(body)
    }

    async fn list_operations(&self, url: &str) -> Result<Vec<Operation>> {
        let schema = self.fetch_schema(url).await?;
        Self::parse_schema_to_operations(&schema)
    }

    async fn describe_operation(&self, url: &str, operation: &str) -> Result<OperationDetail> {
        let schema = self.fetch_schema(url).await?;

        let op = Self::find_operation(&schema, operation)
            .ok_or_else(|| anyhow!("Operation '{}' not found", operation))?;
        let input_schema = Self::build_operation_input_schema(&schema, operation);

        Ok(OperationDetail {
            operation_id: op.operation_id,
            display_name: op.display_name,
            description: op.description,
            parameters: op.parameters,
            return_type: op.return_type,
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
        let mut args = args;

        // Parse operation name to determine type
        let (op_type, field_name) = Self::parse_operation_name(operation)?;
        let selection_override = args.remove("_select");
        let selection_override = match selection_override {
            Some(Value::String(s)) => Some(s),
            Some(other) => {
                bail!(
                    "GraphQL reserved argument '_select' must be a string, got {}",
                    other
                );
            }
            None => None,
        };

        let schema = self.fetch_schema(url).await?;
        let field = Self::find_operation_field(&schema, operation)
            .ok_or_else(|| anyhow!("Operation '{}' not found", operation))?;

        let declared_args = field
            .get("args")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();

        let known_arg_names: HashSet<String> = declared_args
            .iter()
            .filter_map(|arg| {
                arg.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_string())
            })
            .collect();
        let unknown_args: Vec<String> = args
            .keys()
            .filter(|k| !known_arg_names.contains(*k))
            .cloned()
            .collect();
        if !unknown_args.is_empty() {
            bail!(
                "Unknown argument(s) for GraphQL operation '{}': {}",
                operation,
                unknown_args.join(", ")
            );
        }

        let missing_required: Vec<String> = declared_args
            .iter()
            .filter_map(|arg| {
                let name = arg.get("name").and_then(|n| n.as_str())?;
                let is_non_null = arg
                    .get("type")
                    .and_then(|t| t.get("kind"))
                    .and_then(|k| k.as_str())
                    == Some("NON_NULL");
                let required = if !is_non_null {
                    false
                } else {
                    match arg.get("defaultValue") {
                        // defaultValue explicitly null => no default => required
                        Some(v) => v.is_null(),
                        // defaultValue missing from schema => avoid false positives
                        None => false,
                    }
                };
                if required && !args.contains_key(name) {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect();
        if !missing_required.is_empty() {
            bail!(
                "Missing required GraphQL argument(s) for '{}': {}",
                operation,
                missing_required.join(", ")
            );
        }

        let var_defs: Vec<String> = declared_args
            .iter()
            .filter_map(|arg| {
                let name = arg.get("name").and_then(|n| n.as_str())?;
                if !args.contains_key(name) {
                    return None;
                }
                let type_info = arg.get("type")?;
                let type_name = Self::type_to_string(type_info);
                Some(format!("${}: {}", name, type_name))
            })
            .collect();

        let arg_bindings: Vec<String> = declared_args
            .iter()
            .filter_map(|arg| {
                let name = arg.get("name").and_then(|n| n.as_str())?;
                if args.contains_key(name) {
                    Some(format!("{}: ${}", name, name))
                } else {
                    None
                }
            })
            .collect();

        let var_defs_str = if var_defs.is_empty() {
            String::new()
        } else {
            format!(" ({})", var_defs.join(", "))
        };
        let args_str = if arg_bindings.is_empty() {
            String::new()
        } else {
            format!("({})", arg_bindings.join(", "))
        };

        let selection_set =
            selection_override.unwrap_or_else(|| Self::default_selection_set(&schema, operation));

        let query_string = format!(
            "{}{} {{ {}{} {{ {} }} }}",
            match op_type {
                OperationType::Query => "query",
                OperationType::Mutation => "mutation",
                OperationType::Subscription => "subscription",
            },
            var_defs_str,
            field_name,
            args_str,
            selection_set
        );

        let variables = if args.is_empty() {
            None
        } else {
            Some(Value::Object(args.into_iter().collect()))
        };

        let result = self
            .execute_graphql(url, &query_string, variables, None)
            .await?;

        // Extract data from response
        let data = result.get("data").cloned().unwrap_or(result);

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

    #[test]
    fn test_parse_operation_name() {
        let (op_type, name) = GraphQLAdapter::parse_operation_name("query/viewer").unwrap();
        assert!(matches!(op_type, OperationType::Query));
        assert_eq!(name, "viewer");

        let (op_type, name) = GraphQLAdapter::parse_operation_name("mutation/addStar").unwrap();
        assert!(matches!(op_type, OperationType::Mutation));
        assert_eq!(name, "addStar");

        let err = GraphQLAdapter::parse_operation_name("viewer").unwrap_err();
        assert!(
            err.to_string().contains("Invalid GraphQL operation ID"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_type_to_string() {
        let scalar_type = serde_json::json!({
            "kind": "SCALAR",
            "name": "String"
        });
        assert_eq!(GraphQLAdapter::type_to_string(&scalar_type), "String");

        let non_null_type = serde_json::json!({
            "kind": "NON_NULL",
            "ofType": {
                "kind": "SCALAR",
                "name": "String"
            }
        });
        assert_eq!(GraphQLAdapter::type_to_string(&non_null_type), "String!");

        let list_type = serde_json::json!({
            "kind": "LIST",
            "ofType": {
                "kind": "SCALAR",
                "name": "String"
            }
        });
        assert_eq!(GraphQLAdapter::type_to_string(&list_type), "[String]");

        let list_of_non_null = serde_json::json!({
            "kind": "LIST",
            "ofType": {
                "kind": "NON_NULL",
                "ofType": {
                    "kind": "SCALAR",
                    "name": "String"
                }
            }
        });
        assert_eq!(
            GraphQLAdapter::type_to_string(&list_of_non_null),
            "[String!]"
        );
    }

    #[test]
    fn test_build_query() {
        let query = GraphQLAdapter::build_query(OperationType::Query, "viewer", None);
        assert_eq!(query, "query { viewer }");

        let query = GraphQLAdapter::build_query(OperationType::Query, "viewer", Some("id login"));
        assert_eq!(query, "query { viewer { id login } }");

        let mutation = GraphQLAdapter::build_query(OperationType::Mutation, "addStar", None);
        assert_eq!(mutation, "mutation { addStar }");
    }

    #[test]
    fn test_introspection_query_includes_deep_type_ref_fragment() {
        let query = GraphQLAdapter::get_introspection_query();
        assert!(query.contains("fragment TypeRef on __Type"));
        assert!(query.matches("ofType").count() >= 6);
    }

    #[test]
    fn test_build_operation_input_schema_expands_input_objects_and_enums() {
        let schema = serde_json::json!({
            "data": {
                "__schema": {
                    "queryType": {
                        "name": "Query",
                        "fields": [
                            {
                                "name": "user",
                                "args": [
                                    {
                                        "name": "id",
                                        "description": "User id",
                                        "type": {
                                            "kind": "NON_NULL",
                                            "ofType": {
                                                "kind": "SCALAR",
                                                "name": "ID"
                                            }
                                        }
                                    },
                                    {
                                        "name": "filter",
                                        "type": {
                                            "kind": "INPUT_OBJECT",
                                            "name": "UserFilter"
                                        }
                                    }
                                ]
                            }
                        ]
                    },
                    "mutationType": null,
                    "subscriptionType": null,
                    "types": [
                        {
                            "name": "UserFilter",
                            "kind": "INPUT_OBJECT",
                            "inputFields": [
                                {
                                    "name": "status",
                                    "type": {
                                        "kind": "ENUM",
                                        "name": "UserStatus"
                                    }
                                },
                                {
                                    "name": "tags",
                                    "type": {
                                        "kind": "LIST",
                                        "ofType": {
                                            "kind": "SCALAR",
                                            "name": "String"
                                        }
                                    }
                                }
                            ]
                        },
                        {
                            "name": "UserStatus",
                            "kind": "ENUM",
                            "enumValues": [
                                { "name": "ACTIVE" },
                                { "name": "INACTIVE" }
                            ]
                        }
                    ]
                }
            }
        });

        let input_schema =
            GraphQLAdapter::build_operation_input_schema(&schema, "query/user").unwrap();
        assert_eq!(input_schema["kind"], "graphql_arguments");
        assert_eq!(input_schema["type"], "object");
        assert_eq!(input_schema["properties"]["id"]["type"], "string");
        assert_eq!(
            input_schema["properties"]["filter"]["properties"]["status"]["enum"][0],
            "ACTIVE"
        );
        assert_eq!(
            input_schema["properties"]["filter"]["properties"]["tags"]["type"],
            "array"
        );
        assert_eq!(input_schema["required"][0], "id");
    }

    #[tokio::test]
    async fn fetch_schema_with_oauth_refreshes_before_request() {
        let mut server = mockito::Server::new_async().await;
        let token_endpoint = format!("{}/token", server.url());

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

        let _schema = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer new-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "__schema": {
                            "queryType": {"name":"Query","fields":[]},
                            "mutationType": null,
                            "subscriptionType": null,
                            "types": []
                        }
                    }
                }"#,
            )
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
            expires_at: Some(i64::MAX - 1),
            oauth_flow: Some(crate::auth::OAuthFlow::AuthorizationCode),
            ..Default::default()
        });
        if let Some(oauth) = profile.oauth.as_mut() {
            oauth.expires_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
                    + 30,
            );
        }

        let adapter = GraphQLAdapter::new().with_auth(profile);
        let schema = adapter.fetch_schema(&server.url()).await.unwrap();
        assert!(schema.get("data").is_some());
    }
}
