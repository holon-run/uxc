use super::{McpClient, McpError, Session};
use crate::adapters::mcp::events::{client_capabilities, EventMethod};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use tokio::time::timeout;

/// Request a single page of the event catalog.
#[derive(Debug, Clone, Default)]
pub struct ListEventsParams {
    /// Opaque `next_cursor` from a previous page; `None` starts at the beginning.
    pub cursor: Option<String>,
}

/// Request a single poll. The caller owns cursor storage and scheduling.
#[derive(Clone)]
pub struct PollEventsParams {
    /// Name of the event descriptor to poll.
    pub name: String,
    /// Optional server-defined arguments.
    pub arguments: Map<String, Value>,
    /// Opaque checkpoint; `None` sends a null cursor to establish a starting point.
    pub cursor: Option<String>,
    /// Optional maximum age in milliseconds.
    pub max_age_ms: Option<u64>,
    /// Optional maximum number of events.
    pub max_events: Option<u64>,
}

impl PollEventsParams {
    /// Start with no cursor, arguments, or limits.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: Map::new(),
            cursor: None,
            max_age_ms: None,
            max_events: None,
        }
    }
}

/// One event advertised by the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct EventDescriptor {
    /// Name accepted by `events/poll`.
    pub name: String,
    /// Optional display title.
    pub title: Option<String>,
    /// Optional untrusted description.
    pub description: Option<String>,
    /// JSON Schema for arguments.
    pub input_schema: Option<Value>,
    /// JSON Schema for event data.
    pub payload_schema: Option<Value>,
    /// Advertised delivery modes, such as `poll` or `webhook`.
    #[serde(default)]
    pub delivery: Vec<String>,
}

/// One event catalog page, never automatically advanced.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ListEventsResult {
    /// Event descriptors in this page.
    pub events: Vec<EventDescriptor>,
    /// Opaque next page cursor, if there is another page.
    pub next_cursor: Option<String>,
}

/// One received event. Treat all server-supplied fields as untrusted.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct EventItem {
    /// Server-provided event ID.
    pub event_id: String,
    /// Event descriptor name.
    pub name: String,
    /// Optional server-provided timestamp.
    pub timestamp: Option<String>,
    /// Dynamic event payload.
    pub data: Value,
    /// Opaque cursor associated with this item, if supplied.
    pub cursor: Option<String>,
}

/// Result of one poll; none of these hints triggers additional polling.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PollEventsResult {
    /// Events returned by this poll.
    pub events: Vec<EventItem>,
    /// Opaque checkpoint to consider persisting after durable processing.
    pub cursor: String,
    /// Server reports a gap or omitted history.
    #[serde(default)]
    pub truncated: bool,
    /// Server reports more available events.
    #[serde(default)]
    pub has_more: bool,
    /// Suggested delay in milliseconds.
    pub next_poll_ms: Option<u64>,
}

impl McpClient {
    /// Fetch one event catalog page, if Events was negotiated.
    pub async fn list_events_page(
        &mut self,
        params: ListEventsParams,
    ) -> Result<ListEventsResult, McpError> {
        let mut args = HashMap::new();
        if let Some(cursor) = params.cursor {
            args.insert("cursor".to_owned(), Value::String(cursor));
        }
        serde_json::from_value(self.events_request(EventMethod::List, args).await?)
            .map_err(|_| McpError::protocol())
    }

    /// Send one poll, if Events was negotiated. Modern HTTP may first read
    /// descriptors to compute routing headers; it never resends the poll.
    pub async fn poll_events_once(
        &mut self,
        params: PollEventsParams,
    ) -> Result<PollEventsResult, McpError> {
        let mut args = HashMap::from([("name".to_owned(), Value::String(params.name))]);
        if let Some(cursor) = params.cursor {
            args.insert("cursor".to_owned(), Value::String(cursor));
        }
        if !params.arguments.is_empty() {
            args.insert("arguments".to_owned(), Value::Object(params.arguments));
        }
        if let Some(age) = params.max_age_ms {
            args.insert("maxAgeMs".to_owned(), Value::from(age));
        }
        if let Some(limit) = params.max_events {
            args.insert("maxEvents".to_owned(), Value::from(limit));
        }
        serde_json::from_value(self.events_request(EventMethod::Poll, args).await?)
            .map_err(|_| McpError::protocol())
    }

    async fn events_request(
        &mut self,
        method: EventMethod,
        args: HashMap<String, Value>,
    ) -> Result<Value, McpError> {
        if !self.discovery.capabilities.events {
            return Err(McpError::UnsupportedCapability);
        }
        let params = method.params(args).map_err(|_| McpError::Configuration)?;
        let capabilities = client_capabilities();
        let budget = self.start_request()?;
        let response = timeout(budget, async {
            match &mut self.session {
                Session::Stdio(transport) => {
                    transport
                        .call_events(method, params, &capabilities, budget)
                        .await
                }
                Session::Http(transport) => {
                    transport.call_events(method, params, &capabilities).await
                }
            }
        })
        .await;
        self.finish_request(response)
    }
}
