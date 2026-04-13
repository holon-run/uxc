use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollSubscriptionConfig {
    pub interval_secs: u64,
    pub extract_items_pointer: String,
    #[serde(default)]
    pub missing_extract_items_pointer_as_empty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_cursor_arg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_cursor_pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_from_item_pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_transform: Option<PollCursorTransform>,
    pub checkpoint_strategy: PollCheckpointStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollCursorTransform {
    Increment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PollCheckpointStrategy {
    CursorOnly,
    ItemKey {
        item_key_pointer: String,
        #[serde(default)]
        seen_window: Option<usize>,
    },
    Watermark {
        item_watermark_pointer: String,
        #[serde(default)]
        item_tiebreaker_pointer: Option<String>,
        #[serde(default)]
        seen_window: Option<usize>,
    },
    ContentHash {
        #[serde(default)]
        seen_window: Option<usize>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PollCheckpointState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tie_breaker: Option<Value>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub seen_keys: VecDeque<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PollFetchResult {
    pub data: Value,
    pub duration_ms: Option<u64>,
    pub status_code: Option<u16>,
    pub response_headers: HashMap<String, String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PollCycleOutput {
    pub emitted_items: Vec<Value>,
    pub fetched_items: usize,
    pub skipped_items: usize,
    pub duration_ms: Option<u64>,
    pub checkpoint_meta: Option<Value>,
    pub not_modified: bool,
    pub poll_interval_secs: Option<u64>,
}

pub struct PollSubscriptionRunner {
    config: PollSubscriptionConfig,
    checkpoint: PollCheckpointState,
}

impl PollSubscriptionConfig {
    pub fn validate(&self) -> Result<()> {
        if self.interval_secs == 0 {
            bail!("poll interval_secs must be greater than 0");
        }
        if self
            .request_cursor_arg
            .as_deref()
            .is_some_and(|arg| arg.is_empty())
        {
            bail!("poll request_cursor_arg cannot be empty");
        }
        if self
            .response_cursor_pointer
            .as_deref()
            .is_some_and(|pointer| pointer.is_empty())
        {
            bail!("poll response_cursor_pointer cannot be empty");
        }
        if self
            .cursor_from_item_pointer
            .as_deref()
            .is_some_and(|pointer| pointer.is_empty())
        {
            bail!("poll cursor_from_item_pointer cannot be empty");
        }
        if self.cursor_from_item_pointer.is_some() && self.request_cursor_arg.is_none() {
            bail!("poll cursor_from_item_pointer requires request_cursor_arg");
        }
        if self.cursor_transform.is_some() && self.cursor_from_item_pointer.is_none() {
            bail!("poll cursor_transform requires cursor_from_item_pointer");
        }
        if self.response_cursor_pointer.is_some() && self.cursor_from_item_pointer.is_some() {
            bail!(
                "poll response_cursor_pointer and cursor_from_item_pointer are mutually exclusive"
            );
        }

        match &self.checkpoint_strategy {
            PollCheckpointStrategy::CursorOnly => {
                if self.request_cursor_arg.as_deref().is_none() {
                    bail!("cursor_only polling requires request_cursor_arg");
                }
                if self.response_cursor_pointer.is_none() && self.cursor_from_item_pointer.is_none()
                {
                    bail!(
                        "cursor_only polling requires response_cursor_pointer or cursor_from_item_pointer"
                    );
                }
            }
            PollCheckpointStrategy::ItemKey {
                item_key_pointer, ..
            } => {
                if item_key_pointer.is_empty() {
                    bail!("item_key polling requires item_key_pointer");
                }
            }
            PollCheckpointStrategy::Watermark {
                item_watermark_pointer,
                ..
            } => {
                if item_watermark_pointer.is_empty() {
                    bail!("watermark polling requires item_watermark_pointer");
                }
            }
            PollCheckpointStrategy::ContentHash { .. } => {}
        }

        Ok(())
    }
}

impl PollSubscriptionRunner {
    pub fn new(config: PollSubscriptionConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            checkpoint: PollCheckpointState::default(),
        })
    }

    pub fn checkpoint(&self) -> &PollCheckpointState {
        &self.checkpoint
    }

    pub fn restore_checkpoint(&mut self, checkpoint: PollCheckpointState) {
        self.checkpoint = checkpoint;
    }

    pub fn build_request_args(&self, base_args: &HashMap<String, Value>) -> HashMap<String, Value> {
        let mut args = base_args.clone();
        if let Some(request_cursor_arg) = self.config.request_cursor_arg.as_ref() {
            if let Some(cursor_value) = self.request_cursor_value() {
                args.insert(request_cursor_arg.clone(), cursor_value);
            }
        }
        args
    }

    pub fn process_response(
        &mut self,
        response: Value,
        duration_ms: Option<u64>,
    ) -> Result<PollCycleOutput> {
        let empty_items = Value::Array(Vec::new());
        let items_value = match response.pointer(&self.config.extract_items_pointer) {
            Some(value) => value,
            None if self.config.missing_extract_items_pointer_as_empty => &empty_items,
            None => {
                return Err(anyhow!(
                    "poll extract_items_pointer did not resolve: missing JSON pointer '{}'",
                    self.config.extract_items_pointer
                ));
            }
        };
        let items = items_value
            .as_array()
            .ok_or_else(|| anyhow!("poll extract_items_pointer must resolve to an array"))?;

        let fetched_items = items.len();
        let previous_checkpoint = self.checkpoint.clone();
        let strategy = self.config.checkpoint_strategy.clone();
        let emitted_items = match strategy {
            PollCheckpointStrategy::CursorOnly => items.to_vec(),
            PollCheckpointStrategy::ItemKey {
                item_key_pointer,
                seen_window,
            } => self.filter_by_item_key(items, &item_key_pointer, seen_window.unwrap_or(1024))?,
            PollCheckpointStrategy::Watermark {
                item_watermark_pointer,
                item_tiebreaker_pointer,
                seen_window,
            } => self.filter_by_watermark(
                items,
                &item_watermark_pointer,
                item_tiebreaker_pointer.as_deref(),
                seen_window.unwrap_or(1024),
            )?,
            PollCheckpointStrategy::ContentHash { seen_window } => {
                self.filter_by_content_hash(items, seen_window.unwrap_or(1024))?
            }
        };

        if let Some(pointer) = self.config.response_cursor_pointer.as_ref() {
            self.checkpoint.cursor = Some(
                pointer_required(&response, pointer)
                    .with_context(|| {
                        format!("poll response_cursor_pointer '{}' did not resolve", pointer)
                    })?
                    .clone(),
            );
        } else if let Some(pointer) = self.config.cursor_from_item_pointer.as_ref() {
            if let Some(cursor) = self.derive_cursor_from_items(items, pointer)? {
                self.checkpoint.cursor = Some(self.transform_cursor_value(cursor)?);
            }
        }

        let skipped_items = fetched_items.saturating_sub(emitted_items.len());
        let checkpoint_meta = (self.checkpoint != previous_checkpoint).then(|| {
            json!({
                "cursor": self.checkpoint.cursor.clone(),
                "watermark": self.checkpoint.watermark.clone(),
                "tie_breaker": self.checkpoint.tie_breaker.clone(),
                "seen_window_len": self.checkpoint.seen_keys.len(),
                "etag": self.checkpoint.etag.clone(),
            })
        });

        Ok(PollCycleOutput {
            emitted_items,
            fetched_items,
            skipped_items,
            duration_ms,
            checkpoint_meta,
            not_modified: false,
            poll_interval_secs: None,
        })
    }

    fn request_cursor_value(&self) -> Option<Value> {
        self.checkpoint
            .cursor
            .clone()
            .or_else(|| self.checkpoint.watermark.clone())
    }

    fn filter_by_item_key(
        &mut self,
        items: &[Value],
        pointer: &str,
        seen_window: usize,
    ) -> Result<Vec<Value>> {
        let mut emitted = Vec::new();
        for item in items {
            let key = canonical_key(pointer_required(item, pointer).with_context(|| {
                format!("poll item_key_pointer '{}' did not resolve", pointer)
            })?)?;
            if self.checkpoint.seen_keys.contains(&key) {
                continue;
            }
            push_seen_key(&mut self.checkpoint.seen_keys, key, seen_window);
            emitted.push(item.clone());
        }
        Ok(emitted)
    }

    fn filter_by_watermark(
        &mut self,
        items: &[Value],
        watermark_pointer: &str,
        tiebreaker_pointer: Option<&str>,
        seen_window: usize,
    ) -> Result<Vec<Value>> {
        let mut emitted = Vec::new();
        let mut max_watermark = self.checkpoint.watermark.clone();
        let mut max_tiebreaker = self.checkpoint.tie_breaker.clone();

        for item in items {
            let watermark = pointer_required(item, watermark_pointer)
                .cloned()
                .with_context(|| {
                    format!(
                        "poll item_watermark_pointer '{}' did not resolve",
                        watermark_pointer
                    )
                })?;
            let tiebreaker = match tiebreaker_pointer {
                Some(pointer) => {
                    Some(pointer_required(item, pointer).cloned().with_context(|| {
                        format!("poll item_tiebreaker_pointer '{}' did not resolve", pointer)
                    })?)
                }
                None => None,
            };
            let seen_key = match tiebreaker.as_ref() {
                Some(tie) => Some(canonical_pair(&watermark, tie)?),
                None => None,
            };
            if let Some(key) = seen_key.as_ref() {
                if self.checkpoint.seen_keys.contains(key) {
                    continue;
                }
            }

            if is_newer_item(
                &watermark,
                tiebreaker.as_ref(),
                self.checkpoint.watermark.as_ref(),
                self.checkpoint.tie_breaker.as_ref(),
            ) {
                emitted.push(item.clone());
                if let Some(key) = seen_key {
                    push_seen_key(&mut self.checkpoint.seen_keys, key, seen_window);
                }
            }

            if is_newer_item(
                &watermark,
                tiebreaker.as_ref(),
                max_watermark.as_ref(),
                max_tiebreaker.as_ref(),
            ) {
                max_watermark = Some(watermark);
                max_tiebreaker = tiebreaker;
            }
        }

        self.checkpoint.watermark = max_watermark;
        self.checkpoint.tie_breaker = max_tiebreaker;
        Ok(emitted)
    }

    fn filter_by_content_hash(
        &mut self,
        items: &[Value],
        seen_window: usize,
    ) -> Result<Vec<Value>> {
        let mut emitted = Vec::new();
        for item in items {
            let key = content_hash(item)?;
            if self.checkpoint.seen_keys.contains(&key) {
                continue;
            }
            push_seen_key(&mut self.checkpoint.seen_keys, key, seen_window);
            emitted.push(item.clone());
        }
        Ok(emitted)
    }

    fn derive_cursor_from_items(&self, items: &[Value], pointer: &str) -> Result<Option<Value>> {
        let mut cursor = None;
        for item in items {
            let candidate = pointer_required(item, pointer).cloned().with_context(|| {
                format!(
                    "poll cursor_from_item_pointer '{}' did not resolve",
                    pointer
                )
            })?;
            if cursor
                .as_ref()
                .is_none_or(|current| compare_json_values(&candidate, current) == Ordering::Greater)
            {
                cursor = Some(candidate);
            }
        }
        Ok(cursor)
    }

    fn transform_cursor_value(&self, value: Value) -> Result<Value> {
        match self.config.cursor_transform.as_ref() {
            None => Ok(value),
            Some(PollCursorTransform::Increment) => increment_cursor_value(value),
        }
    }
}

fn pointer_required<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value> {
    value
        .pointer(pointer)
        .ok_or_else(|| anyhow!("missing JSON pointer '{}'", pointer))
}

fn canonical_key(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn canonical_pair(left: &Value, right: &Value) -> Result<String> {
    Ok(format!(
        "{}:{}",
        serde_json::to_string(left)?,
        serde_json::to_string(right)?
    ))
}

fn content_hash(value: &Value) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(value)?);
    Ok(format!("{:x}", hasher.finalize()))
}

fn increment_cursor_value(value: Value) -> Result<Value> {
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(json!(value.checked_add(1).ok_or_else(|| anyhow!(
                    "poll increment cursor transform overflowed i64"
                ))?));
            }
            if let Some(value) = number.as_u64() {
                return Ok(json!(value.checked_add(1).ok_or_else(|| anyhow!(
                    "poll increment cursor transform overflowed u64"
                ))?));
            }
            bail!("poll increment cursor transform requires an integer number");
        }
        other => bail!(
            "poll increment cursor transform requires a numeric cursor, got {}",
            value_type_name(&other)
        ),
    }
}

fn push_seen_key(buffer: &mut VecDeque<String>, key: String, seen_window: usize) {
    buffer.push_back(key);
    while buffer.len() > seen_window {
        buffer.pop_front();
    }
}

fn compare_json_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => compare_json_numbers(a, b),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => serde_json::to_string(left)
            .unwrap_or_default()
            .cmp(&serde_json::to_string(right).unwrap_or_default()),
    }
}

fn compare_json_numbers(left: &serde_json::Number, right: &serde_json::Number) -> Ordering {
    match (left.as_i64(), right.as_i64()) {
        (Some(a), Some(b)) => return a.cmp(&b),
        (Some(a), None) if a < 0 => return Ordering::Less,
        (None, Some(b)) if b < 0 => return Ordering::Greater,
        _ => {}
    }

    if let (Some(a), Some(b)) = (left.as_u64(), right.as_u64()) {
        return a.cmp(&b);
    }

    left.as_f64()
        .zip(right.as_f64())
        .and_then(|(a, b)| a.partial_cmp(&b))
        .unwrap_or(Ordering::Equal)
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_newer_item(
    watermark: &Value,
    tie_breaker: Option<&Value>,
    current_watermark: Option<&Value>,
    current_tie_breaker: Option<&Value>,
) -> bool {
    match current_watermark {
        None => true,
        Some(current) => match compare_json_values(watermark, current) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => match (tie_breaker, current_tie_breaker) {
                (Some(next), Some(current)) => {
                    compare_json_values(next, current) == Ordering::Greater
                }
                _ => false,
            },
        },
    }
}

pub(crate) fn extract_header_value<'a>(
    headers: &'a HashMap<String, String>,
    name: &str,
) -> Option<&'a str> {
    headers
        .get(&name.to_ascii_lowercase())
        .or_else(|| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
        .map(String::as_str)
}

pub(crate) fn parse_poll_interval_secs(headers: &HashMap<String, String>) -> Option<u64> {
    let raw = extract_header_value(headers, "x-poll-interval")?;
    let parsed = raw.trim().parse::<u64>().ok()?;
    Some(parsed.clamp(1, 3600))
}

trait ResultExt<T> {
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String;
}

impl<T> ResultExt<T> for Result<T> {
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|err| anyhow!("{}: {}", f(), err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_key_config() -> PollSubscriptionConfig {
        PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: None,
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/id".to_string(),
                seen_window: Some(4),
            },
        }
    }

    #[test]
    fn cursor_only_requires_cursor_config() {
        let err = PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: None,
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::CursorOnly,
        }
        .validate()
        .unwrap_err();

        assert!(err.to_string().contains("cursor_only"));
    }

    #[test]
    fn item_key_strategy_filters_seen_items() {
        let mut runner = PollSubscriptionRunner::new(item_key_config()).unwrap();
        let first = runner
            .process_response(json!({"items":[{"id":1},{"id":2}]}), Some(5))
            .unwrap();
        assert_eq!(first.emitted_items.len(), 2);

        let second = runner
            .process_response(json!({"items":[{"id":2},{"id":3}]}), Some(7))
            .unwrap();
        assert_eq!(second.emitted_items, vec![json!({"id":3})]);
    }

    #[test]
    fn watermark_strategy_uses_tiebreaker() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("since".to_string()),
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::Watermark {
                item_watermark_pointer: "/updated_at".to_string(),
                item_tiebreaker_pointer: Some("/id".to_string()),
                seen_window: Some(4),
            },
        })
        .unwrap();

        let first = runner
            .process_response(
                json!({"items":[
                    {"id":"a","updated_at":"2025-01-01T00:00:00Z"},
                    {"id":"b","updated_at":"2025-01-01T00:00:00Z"}
                ]}),
                None,
            )
            .unwrap();
        assert_eq!(first.emitted_items.len(), 2);

        let second = runner
            .process_response(
                json!({"items":[
                    {"id":"a","updated_at":"2025-01-01T00:00:00Z"},
                    {"id":"c","updated_at":"2025-01-01T00:00:01Z"}
                ]}),
                None,
            )
            .unwrap();
        assert_eq!(
            second.emitted_items,
            vec![json!({"id":"c","updated_at":"2025-01-01T00:00:01Z"})]
        );
        assert_eq!(
            runner.build_request_args(&HashMap::new())["since"],
            "2025-01-01T00:00:01Z"
        );
    }

    #[test]
    fn content_hash_strategy_dedupes_equal_payloads() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: None,
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::ContentHash {
                seen_window: Some(4),
            },
        })
        .unwrap();

        let first = runner
            .process_response(json!({"items":[{"v":1},{"v":1}]}), None)
            .unwrap();
        assert_eq!(first.emitted_items.len(), 1);
    }

    #[test]
    fn cursor_only_requires_response_cursor_in_payload() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("cursor".to_string()),
            response_cursor_pointer: Some("/next_cursor".to_string()),
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::CursorOnly,
        })
        .unwrap();

        let err = runner
            .process_response(json!({"items":[{"id":1}]}), None)
            .unwrap_err();

        assert!(err.to_string().contains("response_cursor_pointer"));
    }

    #[test]
    fn watermark_strategy_dedupes_repeated_pair_within_seen_window() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: None,
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::Watermark {
                item_watermark_pointer: "/updated_at".to_string(),
                item_tiebreaker_pointer: Some("/id".to_string()),
                seen_window: Some(8),
            },
        })
        .unwrap();

        let first = runner
            .process_response(
                json!({"items":[
                    {"id":"a","updated_at":"2025-01-01T00:00:00Z"},
                    {"id":"a","updated_at":"2025-01-01T00:00:00Z"},
                    {"id":"b","updated_at":"2025-01-01T00:00:00Z"}
                ]}),
                None,
            )
            .unwrap();

        assert_eq!(
            first.emitted_items,
            vec![
                json!({"id":"a","updated_at":"2025-01-01T00:00:00Z"}),
                json!({"id":"b","updated_at":"2025-01-01T00:00:00Z"})
            ]
        );
    }

    #[test]
    fn cursor_transform_requires_item_derived_cursor() {
        let err = PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("offset".to_string()),
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: Some(PollCursorTransform::Increment),
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/id".to_string(),
                seen_window: Some(4),
            },
        }
        .validate()
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("cursor_transform requires cursor_from_item_pointer"));
    }

    #[test]
    fn poll_config_rejects_multiple_cursor_sources() {
        let err = PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("cursor".to_string()),
            response_cursor_pointer: Some("/next_cursor".to_string()),
            cursor_from_item_pointer: Some("/update_id".to_string()),
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::CursorOnly,
        }
        .validate()
        .unwrap_err();

        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn item_derived_cursor_updates_next_request_with_increment() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("offset".to_string()),
            response_cursor_pointer: None,
            cursor_from_item_pointer: Some("/update_id".to_string()),
            cursor_transform: Some(PollCursorTransform::Increment),
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/update_id".to_string(),
                seen_window: Some(8),
            },
        })
        .unwrap();

        runner
            .process_response(
                json!({"items":[
                    {"update_id": 1002, "message": "older"},
                    {"update_id": 1004, "message": "newest"},
                    {"update_id": 1003, "message": "middle"}
                ]}),
                None,
            )
            .unwrap();

        assert_eq!(runner.build_request_args(&HashMap::new())["offset"], 1005);
    }

    #[test]
    fn item_derived_cursor_preserves_large_integer_ordering() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("offset".to_string()),
            response_cursor_pointer: None,
            cursor_from_item_pointer: Some("/update_id".to_string()),
            cursor_transform: Some(PollCursorTransform::Increment),
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/update_id".to_string(),
                seen_window: Some(8),
            },
        })
        .unwrap();

        runner
            .process_response(
                json!({"items":[
                    {"update_id": 9007199254740992_u64},
                    {"update_id": 9007199254740994_u64}
                ]}),
                None,
            )
            .unwrap();

        assert_eq!(
            runner.build_request_args(&HashMap::new())["offset"],
            json!(9007199254740995_u64)
        );
    }

    #[test]
    fn item_derived_cursor_uses_last_seen_batch_even_when_items_are_deduped() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("offset".to_string()),
            response_cursor_pointer: None,
            cursor_from_item_pointer: Some("/update_id".to_string()),
            cursor_transform: Some(PollCursorTransform::Increment),
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/update_id".to_string(),
                seen_window: Some(8),
            },
        })
        .unwrap();

        runner
            .process_response(
                json!({"items":[
                    {"update_id": 10},
                    {"update_id": 11}
                ]}),
                None,
            )
            .unwrap();
        runner
            .process_response(
                json!({"items":[
                    {"update_id": 11},
                    {"update_id": 12}
                ]}),
                None,
            )
            .unwrap();

        assert_eq!(runner.build_request_args(&HashMap::new())["offset"], 13);
    }

    #[test]
    fn item_derived_cursor_increment_requires_integer_number() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/items".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("offset".to_string()),
            response_cursor_pointer: None,
            cursor_from_item_pointer: Some("/update_id".to_string()),
            cursor_transform: Some(PollCursorTransform::Increment),
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/update_id".to_string(),
                seen_window: Some(8),
            },
        })
        .unwrap();

        let err = runner
            .process_response(json!({"items":[{"update_id":"100"}]}), None)
            .unwrap_err();

        assert!(err.to_string().contains("requires a numeric cursor"));
    }

    #[test]
    fn missing_extract_items_pointer_can_be_treated_as_empty_array() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/rooms/join/!room:example.org/timeline/events".to_string(),
            missing_extract_items_pointer_as_empty: true,
            request_cursor_arg: Some("since".to_string()),
            response_cursor_pointer: Some("/next_batch".to_string()),
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::CursorOnly,
        })
        .unwrap();

        let output = runner
            .process_response(json!({"next_batch":"s123"}), Some(9))
            .unwrap();

        assert!(output.emitted_items.is_empty());
        assert_eq!(output.fetched_items, 0);
        assert_eq!(runner.build_request_args(&HashMap::new())["since"], "s123");
    }

    #[test]
    fn missing_extract_items_pointer_is_still_an_error_by_default() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: "/rooms/join/!room:example.org/timeline/events".to_string(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: Some("since".to_string()),
            response_cursor_pointer: Some("/next_batch".to_string()),
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::CursorOnly,
        })
        .unwrap();

        let err = runner
            .process_response(json!({"next_batch":"s123"}), Some(9))
            .unwrap_err();

        assert!(err
            .to_string()
            .contains("extract_items_pointer did not resolve"));
    }

    #[test]
    fn empty_extract_items_pointer_uses_response_root_array() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: String::new(),
            missing_extract_items_pointer_as_empty: false,
            request_cursor_arg: None,
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::ItemKey {
                item_key_pointer: "/id".to_string(),
                seen_window: Some(8),
            },
        })
        .unwrap();

        let first = runner
            .process_response(json!([{"id": 1}, {"id": 2}]), Some(3))
            .unwrap();
        assert_eq!(
            first.emitted_items,
            vec![json!({"id": 1}), json!({"id": 2})]
        );

        let second = runner
            .process_response(json!([{"id": 2}, {"id": 3}]), Some(2))
            .unwrap();
        assert_eq!(second.emitted_items, vec![json!({"id": 3})]);
    }

    #[test]
    fn empty_extract_items_pointer_with_missing_as_empty_still_treats_non_array_as_error() {
        let mut runner = PollSubscriptionRunner::new(PollSubscriptionConfig {
            interval_secs: 1,
            extract_items_pointer: String::new(),
            missing_extract_items_pointer_as_empty: true,
            request_cursor_arg: None,
            response_cursor_pointer: None,
            cursor_from_item_pointer: None,
            cursor_transform: None,
            checkpoint_strategy: PollCheckpointStrategy::ContentHash {
                seen_window: Some(8),
            },
        })
        .unwrap();

        let err = runner
            .process_response(json!({"items":[{"id":1}]}), None)
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("extract_items_pointer must resolve to an array"));
    }

    #[test]
    fn parse_poll_interval_from_headers_clamps_and_accepts_case_insensitive_name() {
        let mut headers = HashMap::new();
        headers.insert("X-Poll-Interval".to_string(), "0".to_string());
        assert_eq!(parse_poll_interval_secs(&headers), Some(1));

        headers.insert("x-poll-interval".to_string(), "7200".to_string());
        assert_eq!(parse_poll_interval_secs(&headers), Some(3600));
    }

    #[test]
    fn extract_header_value_matches_case_insensitive_keys() {
        let mut headers = HashMap::new();
        headers.insert("ETag".to_string(), "\"abc\"".to_string());
        assert_eq!(extract_header_value(&headers, "etag"), Some("\"abc\""));
    }
}
