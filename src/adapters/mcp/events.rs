//! One-shot calls to the SEP-3415 Events extension.

use super::parse_schema_to_parameters_for_daemon;
use crate::adapters::{Operation, OperationDetail};
use crate::error::{structured_error_from_anyhow, StructuredError, UxcError};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;

pub(super) const EXTENSION: &str = "io.modelcontextprotocol/events";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventMethod {
    List,
    Poll,
}

impl EventMethod {
    pub(crate) const ALL: [Self; 2] = [Self::List, Self::Poll];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::List => "events/list",
            Self::Poll => "events/poll",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.name() == name)
    }

    pub(crate) fn operation(self) -> Operation {
        let detail = self.detail();
        Operation {
            operation_id: detail.operation_id,
            display_name: detail.display_name,
            description: detail.description,
            parameters: detail.parameters,
            return_type: detail.return_type,
        }
    }

    pub(crate) fn detail(self) -> OperationDetail {
        let (description, return_type, schema) = match self {
            Self::List => (
                "List one page of MCP event descriptors",
                "EventsListResult",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {"cursor": {"type": "string", "description": "List pagination cursor (nextCursor from the previous page)"}}
                }),
            ),
            Self::Poll => (
                "Poll once for events of the named type",
                "EventsPollResult",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["name"],
                    "properties": {
                        "name": {"type": "string", "minLength": 1, "description": "Event type from events/list"},
                        "arguments": {"type": "object", "description": "Event filter arguments"},
                        "cursor": {"type": ["string", "null"], "description": "Delivery cursor; null or omitted starts from now"},
                        "maxAgeMs": {"type": "integer"},
                        "maxEvents": {"type": "integer"}
                    }
                }),
            ),
        };
        OperationDetail {
            operation_id: self.name().to_string(),
            display_name: self.name().to_string(),
            description: Some(description.to_string()),
            parameters: parse_schema_to_parameters_for_daemon(&schema),
            return_type: Some(return_type.to_string()),
            input_schema: Some(schema),
        }
    }

    pub(super) fn params(self, args: HashMap<String, Value>) -> Result<Value> {
        let invalid = |message: &str| UxcError::InvalidArguments(message.to_string());
        for (key, value) in &args {
            let valid = match (self, key.as_str()) {
                (_, "cursor") => value.is_string() || self == Self::Poll && value.is_null(),
                (Self::Poll, "name") => value.as_str().is_some_and(|name| !name.is_empty()),
                (Self::Poll, "arguments") => value.is_object(),
                (Self::Poll, "maxAgeMs" | "maxEvents") => value.is_i64() || value.is_u64(),
                _ => {
                    return Err(
                        invalid(&format!("Unknown {} parameter '{key}'", self.name())).into(),
                    )
                }
            };
            if !valid {
                return Err(invalid(&format!("Invalid {} parameter '{key}'", self.name())).into());
            }
        }
        let mut params: serde_json::Map<_, _> = args.into_iter().collect();
        if self == Self::Poll {
            if !params.contains_key("name") {
                return Err(invalid("events/poll requires 'name'").into());
            }
            params.entry("cursor").or_insert(Value::Null);
        }
        Ok(Value::Object(params))
    }

    pub(crate) fn request(
        self,
        args: HashMap<String, Value>,
        options: super::McpExecutionOptions,
    ) -> Result<(Value, Value)> {
        if options.continuation.is_some() {
            return Err(UxcError::InvalidArguments(format!(
                "{} does not support --mcp-continuation",
                self.name()
            ))
            .into());
        }
        Ok((
            self.params(args)?,
            options.capabilities.unwrap_or_else(client_capabilities),
        ))
    }

    pub(crate) fn unsupported(self) -> anyhow::Error {
        StructuredError::new(
            "MCP_EVENTS_UNSUPPORTED",
            format!(
                "{} is not supported: MCP server does not advertise {EXTENSION}",
                self.name()
            ),
            None,
        )
        .into()
    }

    pub(crate) fn call_error(self, name: &str, err: anyhow::Error) -> anyhow::Error {
        if self != Self::Poll {
            return err;
        }
        if let Some(payload) = structured_error_from_anyhow(&err) {
            if payload
                .details
                .as_ref()
                .and_then(|details| details["jsonrpc_code"].as_i64())
                == Some(-32026)
            {
                return StructuredError::new(
                    payload.code,
                    format!(
                        "MCP event '{name}' does not support polling: {}",
                        payload.message
                    ),
                    payload.details,
                )
                .into();
            }
        }
        err
    }
}

pub(super) fn client_capabilities() -> Value {
    json!({"extensions": { (EXTENSION): {} }})
}
