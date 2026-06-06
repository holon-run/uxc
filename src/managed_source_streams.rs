use crate::error::StructuredError;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const DEFAULT_RETENTION_MAX_ROWS: u64 = 10_000;
const DEFAULT_RETENTION_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;
const MANAGED_SOURCE_SCHEMA_VERSION: i64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedSourceRecord {
    pub namespace: String,
    pub source_key: String,
    pub spec_json: Value,
    pub spec_key: String,
    pub run_id: String,
    pub stream_id: String,
    pub status: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub stopped_at_unix: Option<u64>,
    pub last_error: Option<String>,
    pub last_success_at_unix: Option<u64>,
    pub last_event_at_unix: Option<u64>,
    pub reconnect_count: u64,
    pub written_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStreamRecord {
    pub stream_id: String,
    pub namespace: String,
    pub source_key: String,
    pub created_at_unix: u64,
    pub retention_max_rows: u64,
    pub retention_max_age_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEventRecord {
    pub stream_id: String,
    pub offset: u64,
    pub ingested_at_unix: u64,
    pub raw_payload: Value,
}

#[derive(Debug, Clone)]
pub struct StreamReadPage {
    pub events: Vec<StreamEventRecord>,
    pub next_after_offset: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub struct StreamInfoRecord {
    pub stream_id: String,
    pub namespace: String,
    pub source_key: String,
    pub created_at_unix: u64,
    pub earliest_offset: Option<u64>,
    pub latest_offset: Option<u64>,
    pub latest_event_at_unix: Option<u64>,
    pub event_count: u64,
    pub retention_max_rows: u64,
    pub retention_max_age_secs: u64,
}

#[derive(Debug, Clone)]
pub struct ManagedSourceStoreSummary {
    pub source_count: usize,
    pub running_source_count: usize,
    pub stream_count: usize,
}

#[derive(Debug, Clone)]
pub struct ManagedSourceListRecord {
    pub namespace: String,
    pub source_key: String,
    pub status: String,
    pub run_id: String,
    pub stream_id: String,
    pub updated_at_unix: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SourceRuntimeUpdate {
    pub status: String,
    pub updated_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub stopped_at_unix: Option<u64>,
    pub last_error: Option<String>,
    pub last_success_at_unix: Option<u64>,
    pub last_event_at_unix: Option<u64>,
    pub reconnect_count: u64,
    pub written_events: u64,
}

#[derive(Debug, Clone)]
pub struct PendingStreamEvent {
    pub ingested_at_unix: u64,
    pub payload: Value,
}

#[derive(Clone)]
pub struct ManagedSourceStore {
    path: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl ManagedSourceStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let store = Self {
            path,
            gate: Arc::new(Mutex::new(())),
        };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<()> {
        let mut conn = Connection::open(&self.path)
            .with_context(|| format!("Failed to open {}", self.path.display()))?;
        conn.execute_batch(
            r#"
            pragma journal_mode = wal;
            pragma foreign_keys = on;

            create table if not exists managed_sources (
                namespace text not null,
                source_key text not null,
                spec_json text not null,
                spec_key text not null,
                run_id text not null,
                stream_id text not null,
                status text not null,
                created_at_unix integer not null,
                updated_at_unix integer not null,
                started_at_unix integer,
                stopped_at_unix integer,
                last_error text,
                last_success_at_unix integer,
                last_event_at_unix integer,
                reconnect_count integer not null default 0,
                written_events integer not null default 0,
                poll_checkpoint_json text,
                underlying_job_id text,
                mirrored_after_seq integer not null default 0,
                primary key (namespace, source_key)
            );

            create table if not exists event_streams (
                stream_id text primary key,
                namespace text not null,
                source_key text not null,
                created_at_unix integer not null,
                retention_max_rows integer not null,
                retention_max_age_secs integer not null,
                next_event_offset integer not null default 1
            );

            create table if not exists stream_events (
                stream_id text not null,
                offset integer not null,
                ingested_at_unix integer not null,
                raw_payload_json text not null,
                primary key (stream_id, offset),
                foreign key (stream_id) references event_streams(stream_id)
            );

            create index if not exists idx_stream_events_stream_offset
                on stream_events(stream_id, offset);
            create index if not exists idx_stream_events_stream_ingested_at
                on stream_events(stream_id, ingested_at_unix);
            "#,
        )?;
        migrate_schema(&mut conn)?;
        Ok(())
    }

    pub async fn load_sources(&self) -> Result<Vec<ManagedSourceRecord>> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let mut stmt = conn.prepare(
                r#"
                select
                    namespace,
                    source_key,
                    spec_json,
                    spec_key,
                    run_id,
                    stream_id,
                    status,
                    created_at_unix,
                    updated_at_unix,
                    started_at_unix,
                    stopped_at_unix,
                    last_error,
                    last_success_at_unix,
                    last_event_at_unix,
                    reconnect_count,
                    written_events
                from managed_sources
                order by namespace, source_key
                "#,
            )?;
            let rows = stmt.query_map([], row_to_managed_source_record)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
        .await?
    }

    pub async fn load_source_list_records(&self) -> Result<Vec<ManagedSourceListRecord>> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let mut stmt = conn.prepare(
                r#"
                select
                    namespace,
                    source_key,
                    status,
                    run_id,
                    stream_id,
                    updated_at_unix,
                    last_error
                from managed_sources
                order by updated_at_unix desc, namespace asc, source_key asc
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ManagedSourceListRecord {
                    namespace: row.get(0)?,
                    source_key: row.get(1)?,
                    status: row.get(2)?,
                    run_id: row.get(3)?,
                    stream_id: row.get(4)?,
                    updated_at_unix: row.get(5)?,
                    last_error: row.get(6)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
        .await?
    }

    pub async fn summary(&self) -> Result<ManagedSourceStoreSummary> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let source_count: usize =
                conn.query_row("select count(*) from managed_sources", [], |row| row.get(0))?;
            let running_source_count: usize = conn.query_row(
                "select count(*) from managed_sources where status = 'running'",
                [],
                |row| row.get(0),
            )?;
            let stream_count: usize =
                conn.query_row("select count(*) from event_streams", [], |row| row.get(0))?;
            Ok(ManagedSourceStoreSummary {
                source_count,
                running_source_count,
                stream_count,
            })
        })
        .await?
    }

    pub async fn get_source(
        &self,
        namespace: &str,
        source_key: &str,
    ) -> Result<Option<ManagedSourceRecord>> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            conn.query_row(
                r#"
                select
                    namespace,
                    source_key,
                    spec_json,
                    spec_key,
                    run_id,
                    stream_id,
                    status,
                    created_at_unix,
                    updated_at_unix,
                    started_at_unix,
                    stopped_at_unix,
                    last_error,
                    last_success_at_unix,
                    last_event_at_unix,
                    reconnect_count,
                    written_events
                from managed_sources
                where namespace = ?1 and source_key = ?2
                "#,
                params![namespace, source_key],
                row_to_managed_source_record,
            )
            .optional()
            .map_err(Into::into)
        })
        .await?
    }

    pub async fn upsert_source(
        &self,
        record: &ManagedSourceRecord,
        create_stream: bool,
    ) -> Result<()> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            upsert_source_tx(&tx, &record, create_stream)?;
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn update_source_runtime(
        &self,
        namespace: &str,
        source_key: &str,
        update: SourceRuntimeUpdate,
    ) -> Result<()> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            conn.execute(
                r#"
                update managed_sources
                set status = ?3,
                    updated_at_unix = ?4,
                    started_at_unix = ?5,
                    stopped_at_unix = ?6,
                    last_error = ?7,
                    last_success_at_unix = ?8,
                    last_event_at_unix = ?9,
                    reconnect_count = ?10,
                    written_events = ?11
                where namespace = ?1 and source_key = ?2
                "#,
                params![
                    namespace,
                    source_key,
                    update.status,
                    update.updated_at_unix,
                    update.started_at_unix,
                    update.stopped_at_unix,
                    update.last_error,
                    update.last_success_at_unix,
                    update.last_event_at_unix,
                    update.reconnect_count,
                    update.written_events
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn clear_source_job(
        &self,
        namespace: &str,
        source_key: &str,
        status: &str,
        updated_at_unix: u64,
        stopped_at_unix: Option<u64>,
        last_error: Option<String>,
    ) -> Result<()> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            conn.execute(
                r#"
                update managed_sources
                set status = ?3,
                    updated_at_unix = ?4,
                    stopped_at_unix = ?5,
                    last_error = ?6
                where namespace = ?1 and source_key = ?2
                "#,
                params![
                    namespace,
                    source_key,
                    status,
                    updated_at_unix,
                    stopped_at_unix,
                    last_error
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn delete_source(&self, namespace: &str, source_key: &str) -> Result<()> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            conn.execute(
                "delete from managed_sources where namespace = ?1 and source_key = ?2",
                params![namespace, source_key],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn load_poll_checkpoint(
        &self,
        namespace: &str,
        source_key: &str,
        run_id: &str,
    ) -> Result<Option<crate::subscription_poll::PollCheckpointState>> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        let run_id = run_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let json = conn
                .query_row(
                    r#"
                select poll_checkpoint_json
                from managed_sources
                where namespace = ?1 and source_key = ?2 and run_id = ?3
                "#,
                    params![namespace, source_key, run_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            json.map(|raw| {
                serde_json::from_str::<crate::subscription_poll::PollCheckpointState>(&raw)
                    .map_err(Into::into)
            })
            .transpose()
        })
        .await?
    }

    pub async fn store_poll_checkpoint_if_missing(
        &self,
        namespace: &str,
        source_key: &str,
        run_id: &str,
        checkpoint: &crate::subscription_poll::PollCheckpointState,
    ) -> Result<bool> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        let run_id = run_id.to_string();
        let checkpoint_json = serde_json::to_string(checkpoint)?;
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let changed = conn.execute(
                r#"
                update managed_sources
                set poll_checkpoint_json = ?4
                where namespace = ?1
                  and source_key = ?2
                  and run_id = ?3
                  and poll_checkpoint_json is null
                "#,
                params![namespace, source_key, run_id, checkpoint_json],
            )?;
            Ok(changed > 0)
        })
        .await?
    }

    pub async fn clear_poll_checkpoint(
        &self,
        namespace: &str,
        source_key: &str,
        run_id: &str,
    ) -> Result<()> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        let run_id = run_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            conn.execute(
                r#"
                update managed_sources
                set poll_checkpoint_json = null
                where namespace = ?1 and source_key = ?2 and run_id = ?3
                "#,
                params![namespace, source_key, run_id],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn append_events_and_store_poll_checkpoint(
        &self,
        namespace: &str,
        source_key: &str,
        run_id: &str,
        stream_id: &str,
        events: &[PendingStreamEvent],
        checkpoint: &crate::subscription_poll::PollCheckpointState,
    ) -> Result<Vec<u64>> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let namespace = namespace.to_string();
        let source_key = source_key.to_string();
        let run_id = run_id.to_string();
        let stream_id = stream_id.to_string();
        let events = events.to_vec();
        let checkpoint_json = serde_json::to_string(checkpoint)?;
        tokio::task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current_stream_id: Option<String> = tx
                .query_row(
                    r#"
                    select stream_id
                    from managed_sources
                    where namespace = ?1 and source_key = ?2 and run_id = ?3
                    "#,
                    params![namespace, source_key, run_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current_stream_id) = current_stream_id else {
                anyhow::bail!("managed source row not found while storing poll checkpoint");
            };
            if current_stream_id != stream_id {
                anyhow::bail!("managed source stream mismatch while storing poll checkpoint");
            }

            let mut next_offset: u64 = if events.is_empty() {
                0
            } else {
                tx.query_row(
                    "select next_event_offset from event_streams where stream_id = ?1",
                    params![stream_id],
                    |row| row.get(0),
                )?
            };
            let mut offsets = Vec::with_capacity(events.len());
            let mut latest_ingested_at_unix = None;
            for event in &events {
                tx.execute(
                    r#"
                    insert into stream_events(stream_id, offset, ingested_at_unix, raw_payload_json)
                    values (?1, ?2, ?3, ?4)
                    "#,
                    params![
                        stream_id,
                        next_offset,
                        event.ingested_at_unix,
                        serde_json::to_string(&event.payload)?
                    ],
                )?;
                latest_ingested_at_unix = Some(event.ingested_at_unix);
                offsets.push(next_offset);
                next_offset = next_offset.saturating_add(1);
            }

            let changed = tx.execute(
                r#"
                update managed_sources
                set poll_checkpoint_json = ?4
                where namespace = ?1 and source_key = ?2 and run_id = ?3
                "#,
                params![namespace, source_key, run_id, checkpoint_json],
            )?;
            if changed != 1 {
                anyhow::bail!("managed source row changed while storing poll checkpoint");
            }

            if let Some(now_unix) = latest_ingested_at_unix {
                tx.execute(
                    "update event_streams set next_event_offset = ?2 where stream_id = ?1",
                    params![stream_id, next_offset],
                )?;
                apply_retention_tx(&tx, &stream_id, now_unix)?;
            }
            tx.commit()?;
            Ok(offsets)
        })
        .await?
    }

    pub async fn append_event(
        &self,
        stream_id: &str,
        ingested_at_unix: u64,
        payload: &Value,
    ) -> Result<u64> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let stream_id = stream_id.to_string();
        let payload_json = serde_json::to_string(payload)?;
        tokio::task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let next_offset: u64 = tx.query_row(
                "select next_event_offset from event_streams where stream_id = ?1",
                params![stream_id],
                |row| row.get(0),
            )?;
            tx.execute(
                r#"
                insert into stream_events(stream_id, offset, ingested_at_unix, raw_payload_json)
                values (?1, ?2, ?3, ?4)
                "#,
                params![stream_id, next_offset, ingested_at_unix, payload_json],
            )?;
            tx.execute(
                "update event_streams set next_event_offset = ?2 where stream_id = ?1",
                params![stream_id, next_offset.saturating_add(1)],
            )?;
            apply_retention_tx(&tx, &stream_id, ingested_at_unix)?;
            tx.commit()?;
            Ok(next_offset)
        })
        .await?
    }

    pub async fn read_stream(
        &self,
        stream_id: &str,
        after_offset: u64,
        limit: usize,
    ) -> Result<StreamReadPage> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let stream_id = stream_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let latest_offset = conn
                .query_row(
                    "select max(offset) from stream_events where stream_id = ?1",
                    params![stream_id],
                    |row| row.get::<_, Option<u64>>(0),
                )
                .optional()?
                .flatten();
            if let Some(latest_offset) = latest_offset {
                if after_offset > latest_offset {
                    return Err(StructuredError::new(
                        "cursor_out_of_range",
                        format!(
                            "stream read cursor is out of range: after_offset {} is greater than latest_offset {} for stream {}",
                            after_offset, latest_offset, stream_id
                        ),
                        Some(json!({
                            "stream_id": stream_id,
                            "after_offset": after_offset,
                            "latest_offset": latest_offset,
                        })),
                    )
                    .into());
                }
            }
            if limit == 0 {
                return Ok(StreamReadPage {
                    events: Vec::new(),
                    next_after_offset: after_offset,
                    has_more: false,
                });
            }
            let query_limit = limit.saturating_add(1) as u64;
            let mut stmt = conn.prepare(
                r#"
                select stream_id, offset, ingested_at_unix, raw_payload_json
                from stream_events
                where stream_id = ?1 and offset > ?2
                order by offset asc
                limit ?3
                "#,
            )?;
            let mut rows = stmt.query(params![stream_id, after_offset, query_limit])?;
            let mut events = Vec::new();
            while let Some(row) = rows.next()? {
                events.push(StreamEventRecord {
                    stream_id: row.get(0)?,
                    offset: row.get(1)?,
                    ingested_at_unix: row.get(2)?,
                    raw_payload: serde_json::from_str::<Value>(&row.get::<_, String>(3)?)?,
                });
            }
            let has_more = events.len() > limit;
            if has_more {
                events.truncate(limit);
            }
            let next_after_offset = events
                .last()
                .map(|event| event.offset)
                .unwrap_or(after_offset);
            Ok(StreamReadPage {
                events,
                next_after_offset,
                has_more,
            })
        })
        .await?
    }

    pub async fn stream_info(&self, stream_id: &str) -> Result<Option<StreamInfoRecord>> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let stream_id = stream_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let stream = conn
                .query_row(
                    r#"
                    select stream_id, namespace, source_key, created_at_unix, retention_max_rows, retention_max_age_secs
                    from event_streams
                    where stream_id = ?1
                    "#,
                    params![stream_id],
                    |row| {
                        Ok(EventStreamRecord {
                            stream_id: row.get(0)?,
                            namespace: row.get(1)?,
                            source_key: row.get(2)?,
                            created_at_unix: row.get(3)?,
                            retention_max_rows: row.get(4)?,
                            retention_max_age_secs: row.get(5)?,
                        })
                    },
                )
                .optional()?;
            let Some(stream) = stream else {
                return Ok(None);
            };
            let (earliest_offset, latest_offset, latest_event_at_unix, event_count): (
                Option<u64>,
                Option<u64>,
                Option<u64>,
                u64,
            ) =
                conn.query_row(
                    r#"
                    select min(offset), max(offset), max(ingested_at_unix), count(*)
                    from stream_events
                    where stream_id = ?1
                    "#,
                    params![stream.stream_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?;
            Ok(Some(StreamInfoRecord {
                stream_id: stream.stream_id,
                namespace: stream.namespace,
                source_key: stream.source_key,
                created_at_unix: stream.created_at_unix,
                earliest_offset,
                latest_offset,
                latest_event_at_unix,
                event_count,
                retention_max_rows: stream.retention_max_rows,
                retention_max_age_secs: stream.retention_max_age_secs,
            }))
        })
        .await?
    }

    pub async fn trim_stream_before(&self, stream_id: &str, before_offset: u64) -> Result<u64> {
        let _guard = self.gate.lock().await;
        let path = self.path.clone();
        let stream_id = stream_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let changed = conn.execute(
                "delete from stream_events where stream_id = ?1 and offset < ?2",
                params![stream_id, before_offset],
            )?;
            Ok(changed as u64)
        })
        .await?
    }
}

fn row_to_managed_source_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManagedSourceRecord> {
    Ok(ManagedSourceRecord {
        namespace: row.get(0)?,
        source_key: row.get(1)?,
        spec_json: serde_json::from_str::<Value>(&row.get::<_, String>(2)?)
            .map_err(to_sqlite_error)?,
        spec_key: row.get(3)?,
        run_id: row.get(4)?,
        stream_id: row.get(5)?,
        status: row.get(6)?,
        created_at_unix: row.get(7)?,
        updated_at_unix: row.get(8)?,
        started_at_unix: row.get(9)?,
        stopped_at_unix: row.get(10)?,
        last_error: row.get(11)?,
        last_success_at_unix: row.get(12)?,
        last_event_at_unix: row.get(13)?,
        reconnect_count: row.get(14)?,
        written_events: row.get(15)?,
    })
}

fn upsert_source_tx(
    tx: &rusqlite::Transaction<'_>,
    record: &ManagedSourceRecord,
    create_stream: bool,
) -> Result<()> {
    tx.execute(
        r#"
        insert into managed_sources(
            namespace,
            source_key,
            spec_json,
            spec_key,
            run_id,
            stream_id,
            status,
            created_at_unix,
            updated_at_unix,
            started_at_unix,
            stopped_at_unix,
            last_error,
            last_success_at_unix,
            last_event_at_unix,
            reconnect_count,
            written_events,
            poll_checkpoint_json
        )
        values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        on conflict(namespace, source_key) do update set
            spec_json = excluded.spec_json,
            spec_key = excluded.spec_key,
            run_id = excluded.run_id,
            stream_id = excluded.stream_id,
            status = excluded.status,
            updated_at_unix = excluded.updated_at_unix,
            started_at_unix = excluded.started_at_unix,
            stopped_at_unix = excluded.stopped_at_unix,
            last_error = excluded.last_error,
            last_success_at_unix = excluded.last_success_at_unix,
            last_event_at_unix = excluded.last_event_at_unix,
            reconnect_count = excluded.reconnect_count,
            written_events = excluded.written_events,
            poll_checkpoint_json = excluded.poll_checkpoint_json
        "#,
        params![
            record.namespace,
            record.source_key,
            serde_json::to_string(&record.spec_json)?,
            record.spec_key,
            record.run_id,
            record.stream_id,
            record.status,
            record.created_at_unix,
            record.updated_at_unix,
            record.started_at_unix,
            record.stopped_at_unix,
            record.last_error,
            record.last_success_at_unix,
            record.last_event_at_unix,
            record.reconnect_count,
            record.written_events,
            Option::<String>::None
        ],
    )?;
    if create_stream {
        tx.execute(
            r#"
            insert or ignore into event_streams(
                stream_id,
                namespace,
                source_key,
                created_at_unix,
                retention_max_rows,
                retention_max_age_secs,
                next_event_offset
            )
            values (?1, ?2, ?3, ?4, ?5, ?6, 1)
            "#,
            params![
                record.stream_id,
                record.namespace,
                record.source_key,
                record.created_at_unix,
                DEFAULT_RETENTION_MAX_ROWS,
                DEFAULT_RETENTION_MAX_AGE_SECS
            ],
        )?;
    }
    Ok(())
}

fn apply_retention_tx(
    tx: &rusqlite::Transaction<'_>,
    stream_id: &str,
    now_unix: u64,
) -> Result<()> {
    let (retention_max_rows, retention_max_age_secs): (u64, u64) = tx.query_row(
        "select retention_max_rows, retention_max_age_secs from event_streams where stream_id = ?1",
        params![stream_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let cutoff = now_unix.saturating_sub(retention_max_age_secs);
    tx.execute(
        "delete from stream_events where stream_id = ?1 and ingested_at_unix < ?2",
        params![stream_id, cutoff],
    )?;
    let count: u64 = tx.query_row(
        "select count(*) from stream_events where stream_id = ?1",
        params![stream_id],
        |row| row.get(0),
    )?;
    if count > retention_max_rows {
        let overflow = count - retention_max_rows;
        tx.execute(
            r#"
            delete from stream_events
            where rowid in (
                select rowid
                from stream_events
                where stream_id = ?1
                order by offset asc
                limit ?2
            )
            "#,
            params![stream_id, overflow],
        )?;
    }
    Ok(())
}

fn to_sqlite_error(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
}

fn migrate_schema(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("pragma user_version", [], |row| row.get(0))?;
    if version < 1 {
        migrate_v0_to_v1(conn)?;
    }
    if version < 2 {
        migrate_v1_to_v2(conn)?;
    }
    if version < 3 {
        migrate_v2_to_v3(conn)?;
    }
    conn.pragma_update(None, "user_version", MANAGED_SOURCE_SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_v0_to_v1(conn: &mut Connection) -> Result<()> {
    if !table_has_column(conn, "managed_sources", "poll_checkpoint_json")? {
        conn.execute(
            "alter table managed_sources add column poll_checkpoint_json text",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v1_to_v2(conn: &mut Connection) -> Result<()> {
    if !table_has_column(conn, "managed_sources", "last_success_at_unix")? {
        conn.execute(
            "alter table managed_sources add column last_success_at_unix integer",
            [],
        )?;
    }
    if !table_has_column(conn, "managed_sources", "last_event_at_unix")? {
        conn.execute(
            "alter table managed_sources add column last_event_at_unix integer",
            [],
        )?;
    }
    if !table_has_column(conn, "managed_sources", "reconnect_count")? {
        conn.execute(
            "alter table managed_sources add column reconnect_count integer not null default 0",
            [],
        )?;
    }
    if !table_has_column(conn, "managed_sources", "written_events")? {
        conn.execute(
            "alter table managed_sources add column written_events integer not null default 0",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v2_to_v3(conn: &mut Connection) -> Result<()> {
    if !table_has_column(conn, "event_streams", "next_event_offset")? {
        conn.execute(
            "alter table event_streams add column next_event_offset integer not null default 1",
            [],
        )?;
    }
    conn.execute(
        r#"
        update event_streams
        set next_event_offset = coalesce(
            (
                select max(offset) + 1
                from stream_events
                where stream_events.stream_id = event_streams.stream_id
            ),
            1
        )
        "#,
        [],
    )?;
    Ok(())
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("pragma table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
