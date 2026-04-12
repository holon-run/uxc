use crate::adapters::{Adapter, AdapterEnum, OperationDetail, ProtocolType};
use crate::error::UxcError;
use anyhow::Result;
use serde_json::{Map, Number, Value};
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct NormalizedSchema {
    root: Value,
    strict_unknown_fields: bool,
    enforce_required_fields: bool,
}

pub async fn prepare_execute_args(
    adapter: &AdapterEnum,
    endpoint: &str,
    operation_id: &str,
    raw_args: HashMap<String, Value>,
) -> Result<HashMap<String, Value>> {
    prepare_execute_args_with_adapter(adapter, endpoint, operation_id, raw_args).await
}

pub fn prepare_execute_args_from_detail(
    protocol: ProtocolType,
    operation_id: &str,
    detail: &OperationDetail,
    raw_args: HashMap<String, Value>,
) -> Result<HashMap<String, Value>> {
    if raw_args.is_empty() {
        return Ok(raw_args);
    }

    let Some(schema) = normalize_operation_schema(protocol, detail) else {
        return Ok(raw_args);
    };

    coerce_execute_args(operation_id, raw_args, schema)
}

async fn prepare_execute_args_with_adapter<A: Adapter + Sync>(
    adapter: &A,
    endpoint: &str,
    operation_id: &str,
    raw_args: HashMap<String, Value>,
) -> Result<HashMap<String, Value>> {
    if raw_args.is_empty() {
        return Ok(raw_args);
    }

    let detail = match adapter.describe_operation(endpoint, operation_id).await {
        Ok(detail) => detail,
        Err(_) => return Ok(raw_args),
    };
    prepare_execute_args_from_detail(adapter.protocol_type(), operation_id, &detail, raw_args)
}

fn coerce_execute_args(
    operation_id: &str,
    raw_args: HashMap<String, Value>,
    schema: NormalizedSchema,
) -> Result<HashMap<String, Value>> {
    let value = coerce_value(
        &Value::Object(raw_args.into_iter().collect()),
        &schema.root,
        "$",
        schema.strict_unknown_fields,
        schema.enforce_required_fields,
    )?;

    match value {
        Value::Object(map) => Ok(map.into_iter().collect()),
        _ => Err(UxcError::InvalidArguments(format!(
            "Schema normalization for operation '{}' did not produce an object input",
            operation_id
        ))
        .into()),
    }
}

fn normalize_operation_schema(
    protocol: ProtocolType,
    detail: &OperationDetail,
) -> Option<NormalizedSchema> {
    match protocol {
        ProtocolType::GraphQL => detail.input_schema.as_ref().map(|schema| NormalizedSchema {
            root: schema.clone(),
            strict_unknown_fields: false,
            enforce_required_fields: false,
        }),
        ProtocolType::Mcp => detail.input_schema.as_ref().map(|schema| NormalizedSchema {
            root: schema.clone(),
            strict_unknown_fields: false,
            enforce_required_fields: true,
        }),
        ProtocolType::GRpc => detail
            .input_schema
            .as_ref()
            .and_then(|schema| schema.get("schema"))
            .cloned()
            .map(|schema| NormalizedSchema {
                root: schema,
                strict_unknown_fields: false,
                enforce_required_fields: true,
            }),
        ProtocolType::JsonRpc => normalize_jsonrpc_schema(detail),
        ProtocolType::OpenAPI => normalize_openapi_schema(detail),
    }
}

fn normalize_jsonrpc_schema(detail: &OperationDetail) -> Option<NormalizedSchema> {
    let schema = detail.input_schema.as_ref()?;
    let params = schema.get("params")?.as_array()?;

    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in params {
        let Some(name) = param.get("name").and_then(Value::as_str) else {
            continue;
        };
        let param_schema = param
            .get("schema")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        if param
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            required.push(Value::String(name.to_string()));
        }
        properties.insert(name.to_string(), param_schema);
    }

    Some(NormalizedSchema {
        root: Value::Object(Map::from_iter([
            ("type".to_string(), Value::String("object".to_string())),
            ("properties".to_string(), Value::Object(properties)),
            ("required".to_string(), Value::Array(required)),
        ])),
        strict_unknown_fields: false,
        enforce_required_fields: true,
    })
}

fn normalize_openapi_schema(detail: &OperationDetail) -> Option<NormalizedSchema> {
    if let Some(schema) = detail.input_schema.as_ref() {
        if let Some(content) = schema.get("content").and_then(Value::as_object) {
            for media_type in [
                "application/json",
                "application/x-www-form-urlencoded",
                "multipart/form-data",
            ] {
                let Some(json_schema) = content
                    .get(media_type)
                    .and_then(|entry| entry.get("schema"))
                else {
                    continue;
                };
                return Some(NormalizedSchema {
                    root: json_schema.clone(),
                    strict_unknown_fields: false,
                    enforce_required_fields: true,
                });
            }
        }
    }

    if detail.parameters.is_empty() {
        return None;
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in &detail.parameters {
        properties.insert(
            param.name.clone(),
            Value::Object(Map::from_iter([(
                "type".to_string(),
                Value::String(param.param_type.clone()),
            )])),
        );
        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }

    Some(NormalizedSchema {
        root: Value::Object(Map::from_iter([
            ("type".to_string(), Value::String("object".to_string())),
            ("properties".to_string(), Value::Object(properties)),
            ("required".to_string(), Value::Array(required)),
        ])),
        strict_unknown_fields: false,
        enforce_required_fields: true,
    })
}

fn coerce_value(
    value: &Value,
    schema: &Value,
    path: &str,
    strict_unknown_fields: bool,
    enforce_required_fields: bool,
) -> Result<Value> {
    if let Some(branches) = schema.get("oneOf").and_then(Value::as_array) {
        return coerce_union(
            value,
            branches,
            path,
            strict_unknown_fields,
            enforce_required_fields,
            "oneOf",
        );
    }
    if let Some(branches) = schema.get("anyOf").and_then(Value::as_array) {
        return coerce_union(
            value,
            branches,
            path,
            strict_unknown_fields,
            enforce_required_fields,
            "anyOf",
        );
    }

    let inferred_type = schema_type(schema);
    let coerced = match inferred_type.as_deref() {
        Some("object") => coerce_object(
            value,
            schema,
            path,
            strict_unknown_fields,
            enforce_required_fields,
        )?,
        Some("array") => coerce_array(
            value,
            schema,
            path,
            strict_unknown_fields,
            enforce_required_fields,
        )?,
        Some("integer") => coerce_integer(value, path)?,
        Some("number") => coerce_number(value, path)?,
        Some("boolean") => coerce_boolean(value, path)?,
        Some("null") => coerce_null(value, path)?,
        Some("string") => coerce_string(value, path)?,
        _ => value.clone(),
    };

    validate_const_and_enum(&coerced, schema, path)?;
    Ok(coerced)
}

fn coerce_union(
    value: &Value,
    branches: &[Value],
    path: &str,
    strict_unknown_fields: bool,
    enforce_required_fields: bool,
    label: &str,
) -> Result<Value> {
    let mut errors = Vec::new();
    for branch in branches {
        match coerce_value(
            value,
            branch,
            path,
            strict_unknown_fields,
            enforce_required_fields,
        ) {
            Ok(v) => return Ok(v),
            Err(err) => errors.push(err.to_string()),
        }
    }

    Err(UxcError::InvalidArguments(format!(
        "Invalid value at {}: does not match any {} branch ({})",
        path,
        label,
        errors.join("; ")
    ))
    .into())
}

fn coerce_object(
    value: &Value,
    schema: &Value,
    path: &str,
    strict_unknown_fields: bool,
    enforce_required_fields: bool,
) -> Result<Value> {
    let value = match value {
        Value::Object(map) => Value::Object(map.clone()),
        Value::String(raw) => serde_json::from_str::<Value>(raw).map_err(|_| {
            UxcError::InvalidArguments(format!(
                "Invalid value at {}: expected object, got string {}. Use nested path keys (for example a.b=c) or per-field JSON assignment (a:='{{\"k\":\"v\"}}').",
                path,
                render_value(value)
            ))
        })?,
        _ => {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid value at {}: expected object, got {}",
                path,
                type_name(value)
            ))
            .into())
        }
    };

    let obj = value.as_object().ok_or_else(|| {
        UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected object, got {}",
            path,
            type_name(&value)
        ))
    })?;

    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut output = Map::new();

    if enforce_required_fields {
        for key in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(key) {
                return Err(UxcError::InvalidArguments(format!(
                    "Invalid value at {}.{}: missing required field",
                    path, key
                ))
                .into());
            }
        }
    }

    for (key, raw) in obj {
        if let Some(prop_schema) = properties.get(key) {
            let child_path = format!("{}.{}", path, key);
            output.insert(
                key.clone(),
                coerce_value(
                    raw,
                    prop_schema,
                    &child_path,
                    strict_unknown_fields,
                    enforce_required_fields,
                )?,
            );
            continue;
        }

        if strict_unknown_fields {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid value at {}.{}: unknown field",
                path, key
            ))
            .into());
        }

        output.insert(key.clone(), raw.clone());
    }

    Ok(Value::Object(output))
}

fn coerce_array(
    value: &Value,
    schema: &Value,
    path: &str,
    strict_unknown_fields: bool,
    enforce_required_fields: bool,
) -> Result<Value> {
    let value = match value {
        Value::Array(items) => Value::Array(items.clone()),
        Value::String(raw) => serde_json::from_str::<Value>(raw).map_err(|_| {
            UxcError::InvalidArguments(format!(
                "Invalid value at {}: expected array, got string {}. Use indexed path keys (for example items[0]=x) or per-field JSON assignment (items:='[\"x\"]').",
                path,
                render_value(value)
            ))
        })?,
        _ => {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid value at {}: expected array, got {}",
                path,
                type_name(value)
            ))
            .into())
        }
    };

    let items = value.as_array().ok_or_else(|| {
        UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected array, got {}",
            path,
            type_name(&value)
        ))
    })?;

    let item_schema = schema.get("items");
    let mut coerced_items = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let child_path = format!("{}[{}]", path, idx);
        let coerced = match item_schema {
            Some(child_schema) => coerce_value(
                item,
                child_schema,
                &child_path,
                strict_unknown_fields,
                enforce_required_fields,
            )?,
            None => item.clone(),
        };
        coerced_items.push(coerced);
    }

    Ok(Value::Array(coerced_items))
}

fn coerce_integer(value: &Value, path: &str) -> Result<Value> {
    match value {
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(Value::Number(n.clone())),
        Value::Number(n) => Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected integer, got {}",
            path, n
        ))
        .into()),
        Value::String(raw) => {
            let parsed = raw.parse::<i64>().map_err(|_| {
                UxcError::InvalidArguments(format!(
                    "Invalid value at {}: expected integer, got {}",
                    path,
                    render_value(value)
                ))
            })?;
            Ok(Value::Number(Number::from(parsed)))
        }
        _ => Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected integer, got {}",
            path,
            type_name(value)
        ))
        .into()),
    }
}

fn coerce_number(value: &Value, path: &str) -> Result<Value> {
    match value {
        Value::Number(n) => Ok(Value::Number(n.clone())),
        Value::String(raw) => {
            let parsed = raw.parse::<f64>().map_err(|_| {
                UxcError::InvalidArguments(format!(
                    "Invalid value at {}: expected number, got {}",
                    path,
                    render_value(value)
                ))
            })?;
            let number = Number::from_f64(parsed).ok_or_else(|| {
                UxcError::InvalidArguments(format!(
                    "Invalid value at {}: expected finite number, got {}",
                    path, parsed
                ))
            })?;
            Ok(Value::Number(number))
        }
        _ => Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected number, got {}",
            path,
            type_name(value)
        ))
        .into()),
    }
}

fn coerce_boolean(value: &Value, path: &str) -> Result<Value> {
    match value {
        Value::Bool(v) => Ok(Value::Bool(*v)),
        Value::String(raw) if raw.eq_ignore_ascii_case("true") => Ok(Value::Bool(true)),
        Value::String(raw) if raw.eq_ignore_ascii_case("false") => Ok(Value::Bool(false)),
        _ => Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected boolean, got {}",
            path,
            render_value(value)
        ))
        .into()),
    }
}

fn coerce_null(value: &Value, path: &str) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::String(raw) if raw.eq_ignore_ascii_case("null") => Ok(Value::Null),
        _ => Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected null, got {}",
            path,
            render_value(value)
        ))
        .into()),
    }
}

fn coerce_string(value: &Value, path: &str) -> Result<Value> {
    match value {
        Value::String(v) => Ok(Value::String(v.clone())),
        Value::Number(v) => Ok(Value::String(v.to_string())),
        Value::Bool(v) => Ok(Value::String(v.to_string())),
        Value::Null => Ok(Value::String("null".to_string())),
        _ => Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected string, got {}",
            path,
            type_name(value)
        ))
        .into()),
    }
}

fn validate_const_and_enum(value: &Value, schema: &Value, path: &str) -> Result<()> {
    if let Some(const_value) = schema.get("const") {
        if value != const_value {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid value at {}: expected {}, got {}",
                path,
                render_value(const_value),
                render_value(value)
            ))
            .into());
        }
    }

    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if enum_values.iter().any(|candidate| candidate == value) {
            return Ok(());
        }

        if let Value::String(raw) = value {
            for candidate in enum_values {
                if let Ok(coerced) = coerce_literal_to_match(raw, candidate) {
                    if &coerced == candidate {
                        return Ok(());
                    }
                }
            }
        }

        return Err(UxcError::InvalidArguments(format!(
            "Invalid value at {}: expected one of {}, got {}",
            path,
            render_value(&Value::Array(enum_values.clone())),
            render_value(value)
        ))
        .into());
    }

    Ok(())
}

fn coerce_literal_to_match(raw: &str, sample: &Value) -> Result<Value> {
    match sample {
        Value::String(_) => Ok(Value::String(raw.to_string())),
        Value::Bool(_) => {
            if raw.eq_ignore_ascii_case("true") {
                Ok(Value::Bool(true))
            } else if raw.eq_ignore_ascii_case("false") {
                Ok(Value::Bool(false))
            } else {
                Err(UxcError::InvalidArguments("enum coercion failed".to_string()).into())
            }
        }
        Value::Null => {
            if raw.eq_ignore_ascii_case("null") {
                Ok(Value::Null)
            } else {
                Err(UxcError::InvalidArguments("enum coercion failed".to_string()).into())
            }
        }
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            Ok(Value::Number(Number::from(raw.parse::<i64>().map_err(
                |_| UxcError::InvalidArguments("enum coercion failed".to_string()),
            )?)))
        }
        Value::Number(_) => Ok(Value::Number(
            Number::from_f64(
                raw.parse::<f64>()
                    .map_err(|_| UxcError::InvalidArguments("enum coercion failed".to_string()))?,
            )
            .ok_or_else(|| UxcError::InvalidArguments("enum coercion failed".to_string()))?,
        )),
        _ => Err(UxcError::InvalidArguments("enum coercion failed".to_string()).into()),
    }
}

fn schema_type(schema: &Value) -> Option<String> {
    if let Some(type_name) = schema.get("type").and_then(Value::as_str) {
        return Some(type_name.to_string());
    }
    if schema.get("properties").is_some() {
        return Some("object".to_string());
    }
    if schema.get("items").is_some() {
        return Some("array".to_string());
    }
    None
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(v) => format!("{:?}", v),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::Parameter;
    use crate::adapters::{Adapter, ExecutionResult};
    use async_trait::async_trait;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn graphql_detail() -> OperationDetail {
        OperationDetail {
            operation_id: "query/test".to_string(),
            display_name: "query/test".to_string(),
            description: None,
            parameters: vec![],
            return_type: None,
            input_schema: Some(serde_json::json!({
                "kind": "graphql_arguments",
                "type": "object",
                "properties": {
                    "count": { "type": "integer" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["count"],
                "additionalProperties": false
            })),
        }
    }

    #[test]
    fn normalize_graphql_schema_marks_unknown_fields_strict() {
        let schema = normalize_operation_schema(ProtocolType::GraphQL, &graphql_detail()).unwrap();
        assert!(!schema.strict_unknown_fields);
        assert!(!schema.enforce_required_fields);
    }

    #[test]
    fn coerce_scalars_from_strings() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" },
                "enabled": { "type": "boolean" },
                "name": { "type": "string" }
            },
            "required": ["count", "enabled"]
        });
        let input = serde_json::json!({
            "count": "42",
            "enabled": "true",
            "name": 7
        });

        let value = coerce_value(&input, &schema, "$", false, true).unwrap();
        assert_eq!(value["count"], 42);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["name"], "7");
    }

    #[test]
    fn coerce_array_and_object_from_json_string() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array", "items": { "type": "string" } },
                "filter": {
                    "type": "object",
                    "properties": { "limit": { "type": "integer" } },
                    "required": ["limit"]
                }
            }
        });
        let input = serde_json::json!({
            "tags": "[\"a\", \"b\"]",
            "filter": "{\"limit\":\"2\"}"
        });

        let value = coerce_value(&input, &schema, "$", false, true).unwrap();
        assert_eq!(value["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(value["filter"]["limit"], 2);
    }

    #[test]
    fn strict_unknown_fields_error() {
        let err = coerce_value(
            &serde_json::json!({"count": "1", "extra": "2"}),
            &graphql_detail().input_schema.unwrap(),
            "$",
            true,
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("$.extra"));
    }

    #[test]
    fn graphql_schema_allows_unknown_and_missing_fields_for_adapter_validation() {
        let schema = normalize_operation_schema(ProtocolType::GraphQL, &graphql_detail()).unwrap();
        let value = coerce_value(
            &serde_json::json!({"extra":"2"}),
            &schema.root,
            "$",
            schema.strict_unknown_fields,
            schema.enforce_required_fields,
        )
        .unwrap();
        assert_eq!(value["extra"], "2");
    }

    #[test]
    fn openapi_parameter_fallback_schema_is_built() {
        let detail = OperationDetail {
            operation_id: "get:/pets".to_string(),
            display_name: "get:/pets".to_string(),
            description: None,
            parameters: vec![Parameter {
                name: "limit".to_string(),
                param_type: "integer".to_string(),
                required: true,
                description: None,
            }],
            return_type: None,
            input_schema: None,
        };

        let schema = normalize_openapi_schema(&detail).unwrap();
        assert_eq!(schema.root["properties"]["limit"]["type"], "integer");
    }

    struct StubAdapter {
        describe_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Adapter for StubAdapter {
        fn protocol_type(&self) -> ProtocolType {
            ProtocolType::Mcp
        }

        async fn can_handle(&self, _url: &str) -> Result<bool> {
            Ok(true)
        }

        async fn fetch_schema(&self, _url: &str) -> Result<Value> {
            Ok(serde_json::json!({}))
        }

        async fn list_operations(&self, _url: &str) -> Result<Vec<crate::adapters::Operation>> {
            Ok(Vec::new())
        }

        async fn describe_operation(
            &self,
            _url: &str,
            _operation: &str,
        ) -> Result<OperationDetail> {
            self.describe_calls.fetch_add(1, Ordering::SeqCst);
            Ok(OperationDetail {
                operation_id: "noop".to_string(),
                display_name: "noop".to_string(),
                description: None,
                parameters: Vec::new(),
                return_type: None,
                input_schema: None,
            })
        }

        async fn execute(
            &self,
            _url: &str,
            _operation: &str,
            _args: HashMap<String, Value>,
        ) -> Result<ExecutionResult> {
            Ok(ExecutionResult {
                data: Value::Null,
                metadata: crate::adapters::ExecutionMetadata {
                    duration_ms: 0,
                    operation: "noop".to_string(),
                    response_status_code: None,
                    response_headers: HashMap::new(),
                },
            })
        }
    }

    #[tokio::test]
    async fn empty_args_skip_describe_operation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let stub = StubAdapter {
            describe_calls: calls.clone(),
        };

        let result = prepare_execute_args_with_adapter(&stub, "ignored", "noop", HashMap::new())
            .await
            .unwrap();

        assert!(result.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
