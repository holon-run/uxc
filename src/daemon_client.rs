use crate::auth::injected_env::InjectEnvSpec;
use crate::daemon::{
    self, DaemonSessionView, DaemonStatus, RuntimeAction, RuntimeInvokeOptions,
    RuntimeInvokeRequest, RuntimeInvokeResponse, SubscribeStartRequest, SubscribeStartResponse,
    SubscribeStopResponse, SubscriptionEventsRequest, SubscriptionEventsResponse,
    SubscriptionJobView, SubscriptionMode, SubscriptionTransportHint,
};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct DaemonClientOptions {
    pub auth: Option<String>,
    pub inject_env: Vec<InjectEnvSpec>,
    pub no_cache: bool,
    pub cache_ttl: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub refresh_schema: bool,
    pub schema_url: Option<String>,
    pub link_name: Option<String>,
    pub link_skill: Option<String>,
    pub link_skill_doc: Option<String>,
    pub link_skill_path: Option<String>,
    pub schema_mapping_file: Option<String>,
    pub daemon_exclusive: Vec<String>,
    pub daemon_idle_ttl: Option<u64>,
}

impl From<DaemonClientOptions> for RuntimeInvokeOptions {
    fn from(value: DaemonClientOptions) -> Self {
        Self {
            auth: value.auth,
            inject_env: value.inject_env,
            no_cache: value.no_cache,
            cache_ttl: value.cache_ttl,
            timeout_ms: value.timeout_ms,
            refresh_schema: value.refresh_schema,
            schema_url: value.schema_url,
            link_name: value.link_name,
            link_skill: value.link_skill,
            link_skill_doc: value.link_skill_doc,
            link_skill_path: value.link_skill_path,
            schema_mapping_file: value.schema_mapping_file,
            daemon_exclusive: value.daemon_exclusive,
            daemon_idle_ttl: value.daemon_idle_ttl,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DaemonClient;

#[derive(Debug, Clone)]
pub struct DaemonSubscribeRequest {
    pub endpoint: String,
    pub sink: String,
    pub operation_id: Option<String>,
    pub args: Option<HashMap<String, Value>>,
    pub resource_uri: Option<String>,
    pub mode: SubscriptionMode,
    pub transport_hint: Option<SubscriptionTransportHint>,
    pub options: DaemonClientOptions,
    pub read_resource: bool,
    pub ephemeral: bool,
}

impl DaemonClient {
    pub fn new() -> Self {
        Self
    }

    pub fn socket_path(&self) -> std::path::PathBuf {
        daemon::socket_path()
    }

    pub async fn daemon_status(&self) -> Result<DaemonStatus> {
        daemon::daemon_status_client().await
    }

    pub async fn daemon_sessions(&self) -> Result<Vec<DaemonSessionView>> {
        daemon::daemon_sessions_client().await
    }

    pub async fn call(
        &self,
        endpoint: impl Into<String>,
        operation: impl Into<String>,
        payload: Option<HashMap<String, Value>>,
        options: DaemonClientOptions,
    ) -> Result<RuntimeInvokeResponse> {
        let request = RuntimeInvokeRequest {
            request_id: default_request_id("call"),
            endpoint: endpoint.into(),
            action: RuntimeAction::Execute,
            operation_id: Some(operation.into()),
            args: payload,
            options: options.into(),
        };
        daemon::runtime_invoke_client(&request).await
    }

    pub async fn subscribe_start(
        &self,
        request: DaemonSubscribeRequest,
    ) -> Result<SubscribeStartResponse> {
        let request = SubscribeStartRequest {
            request_id: default_request_id("subscribe"),
            endpoint: request.endpoint,
            sink: request.sink,
            operation_id: request.operation_id,
            args: request.args,
            resource_uri: request.resource_uri,
            read_resource: request.read_resource,
            transport_hint: request.transport_hint,
            subprotocols: Vec::new(),
            initial_text_frames: Vec::new(),
            mode: request.mode,
            poll_config: None,
            ephemeral: request.ephemeral,
            options: request.options.into(),
        };
        daemon::subscribe_start_client(&request).await
    }

    pub async fn subscribe_list(&self) -> Result<Vec<SubscriptionJobView>> {
        daemon::subscribe_list_client().await
    }

    pub async fn subscribe_status(&self, job_id: &str) -> Result<SubscriptionJobView> {
        daemon::subscribe_status_client(job_id).await
    }

    pub async fn subscribe_stop(&self, job_id: &str) -> Result<SubscribeStopResponse> {
        daemon::subscribe_stop_client(job_id).await
    }

    pub async fn subscribe_events(
        &self,
        job_id: impl Into<String>,
        after_seq: u64,
        limit: usize,
        wait_ms: u64,
    ) -> Result<SubscriptionEventsResponse> {
        daemon::subscribe_events_client(&SubscriptionEventsRequest {
            job_id: job_id.into(),
            after_seq,
            limit,
            wait_ms,
        })
        .await
    }
}

fn default_request_id(prefix: &str) -> String {
    format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
