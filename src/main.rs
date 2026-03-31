use anyhow::Result;
use clap::{error::ErrorKind, Parser, Subcommand, ValueEnum};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::IpAddr;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tracing::info;

mod adapters;
mod arg_coercion;
mod auth;
mod cache;
pub mod cli;
mod daemon;
mod daemon_log;
mod error;
mod http_client;
mod output;
mod schema_mapping;
mod subscription_discord;
mod subscription_feishu;
mod subscription_graphql;
mod subscription_jsonrpc;
mod subscription_poll;
mod subscription_slack;
mod subscription_websocket;

use adapters::OperationDetail;
use auth::injected_env::{parse_inject_env_specs, InjectEnvSpec};
use auth::{AuthBindingRule, AuthBindings, AuthHeader, AuthType, OAuthFlow, Profile, Profiles};
use cache::CacheConfig;
use error::{structured_error_from_anyhow, UxcError};
use http_client::build_resilient_http_client;
use output::OutputEnvelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Json,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SubscribeModeArg {
    Stream,
    Poll,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum SubscribeTransportArg {
    Websocket,
    DiscordGateway,
    SlackSocketMode,
    FeishuLongConnection,
}

#[derive(Parser)]
#[command(name = "uxc")]
#[command(about = "Universal X-Protocol CLI", long_about = None)]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(disable_help_flag = true)]
#[command(disable_help_subcommand = true)]
struct Cli {
    /// Show help
    #[arg(short = 'h', long = "help", global = true)]
    help: bool,

    /// Exclusive daemon state keys for MCP stdio session eviction/hand-off (can be repeated).
    ///
    /// Use this when the stdio server process locks a shared state directory/file and you want
    /// seamless switching between different endpoints that share the same state path.
    #[arg(long = "daemon-exclusive", global = true, value_name = "KEY")]
    daemon_exclusive: Vec<String>,

    /// Idle TTL in seconds for reused MCP stdio daemon sessions (0 disables idle reaping).
    #[arg(long = "daemon-idle-ttl", global = true, value_name = "SECONDS")]
    daemon_idle_ttl: Option<u64>,

    /// Explicit credential ID for this request (overrides endpoint binding auto-match)
    #[arg(long, global = true)]
    auth: Option<String>,

    /// Inject resolved credential secret into stdio child env using NAME={{secret}}
    #[arg(long = "inject-env", global = true, value_name = "NAME={{secret}}")]
    inject_env: Vec<String>,

    /// Disable cache for this operation
    #[arg(long, global = true)]
    no_cache: bool,

    /// Cache TTL in seconds
    #[arg(long, global = true)]
    cache_ttl: Option<u64>,

    /// Per-request timeout in milliseconds
    #[arg(long = "timeout-ms", global = true, value_name = "MILLISECONDS")]
    timeout_ms: Option<u64>,

    /// Force online schema discovery and refresh cache.
    #[arg(long, global = true, conflicts_with = "no_cache")]
    refresh_schema: bool,

    /// Explicit OpenAPI schema URL (for schema-discovery separated services)
    #[arg(long, global = true)]
    schema_url: Option<String>,

    /// Output format (default: json)
    #[arg(long, value_enum, global = true)]
    format: Option<OutputFormat>,

    /// Use human-readable text output
    #[arg(long, global = true, conflicts_with = "format")]
    text: bool,

    /// Remote endpoint URL (not used with 'cache'/'auth' subcommands)
    #[arg(value_name = "URL")]
    url: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Commands {
    /// Manage schema cache
    Cache {
        #[command(subcommand)]
        cache_command: CacheCommands,
    },

    /// Manage authentication credentials and bindings
    Auth {
        #[command(subcommand)]
        auth_command: AuthCommands,
    },

    /// Create a host-bound shortcut command
    Link {
        /// Shortcut command name (file name)
        #[arg(value_name = "NAME")]
        name: String,

        /// Host/endpoint bound to this shortcut
        #[arg(value_name = "HOST")]
        host: String,

        /// Directory to write the shortcut file (default: ~/.local/bin on Unix, ~/.uxc/bin on Windows)
        #[arg(long, value_name = "DIR")]
        dir: Option<String>,

        /// Default OpenAPI schema URL persisted in the generated shortcut
        #[arg(long)]
        schema_url: Option<String>,

        /// Credential ID persisted into the generated shortcut as --auth <credential_id>
        #[arg(long = "credential")]
        credential: Option<String>,

        /// Overwrite existing shortcut file
        #[arg(long)]
        force: bool,
    },

    /// Manage local runtime daemon
    Daemon {
        #[command(subcommand)]
        daemon_command: DaemonCommands,
    },

    /// Manage background subscriptions via daemon
    Subscribe {
        #[command(subcommand)]
        subscribe_command: SubscribeCommands,
    },

    /// Dynamic operation execution: `uxc <url> <operation_id> [key=value ...] ['{...}']`
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start daemon process
    Start,
    /// Stop daemon process
    Stop,
    /// Show daemon status
    Status,
    /// List MCP daemon sessions
    Sessions,
    /// Restart daemon process (stop if running, then start)
    Restart,
    /// Internal daemon server entrypoint
    #[command(name = "_serve", hide = true)]
    Serve,
}

#[derive(Subcommand)]
enum SubscribeCommands {
    /// Start a background subscription job
    Start {
        /// HTTP stream URL, WebSocket JSON-RPC endpoint, GraphQL endpoint, or MCP endpoint/stdio command
        #[arg(value_name = "ENDPOINT")]
        endpoint: String,

        /// Optional operation ID for protocol-aware subscriptions (for example subscription/messageAdded or eth_subscribe)
        #[arg(value_name = "OPERATION_ID")]
        operation_id: Option<String>,

        /// Operation arguments as key=value pairs or one positional JSON object
        #[arg(value_name = "ARG")]
        args: Vec<String>,

        /// JSON object payload for operation arguments
        #[arg(long = "input-json", value_name = "JSON")]
        input_json: Option<String>,

        /// File sink spec, for example file:/tmp/events.ndjson
        #[arg(long, value_name = "file:/path.ndjson")]
        sink: String,

        /// MCP resource URI to subscribe to
        #[arg(long = "resource-uri", value_name = "URI")]
        resource_uri: Option<String>,

        /// For MCP resource subscriptions, emit resource snapshots by calling resources/read
        #[arg(long = "read-resource")]
        read_resource: bool,

        /// Explicit stream transport hint
        #[arg(long, value_enum)]
        transport: Option<SubscribeTransportArg>,

        /// WebSocket subprotocol to advertise during handshake (repeatable)
        #[arg(long = "subprotocol", value_name = "VALUE")]
        subprotocols: Vec<String>,

        /// Initial WebSocket text frame to send after connect (repeatable)
        #[arg(long = "init-frame", value_name = "TEXT_OR_JSON")]
        init_frames: Vec<String>,

        /// Event acquisition mode
        #[arg(long, value_enum, default_value = "stream")]
        mode: SubscribeModeArg,

        /// JSON object describing poll interval, extraction, and checkpoint strategy
        #[arg(long = "poll-config", value_name = "JSON")]
        poll_config: Option<String>,

        /// Do not auto-resume this subscription after daemon restart
        #[arg(long)]
        ephemeral: bool,
    },
    /// List background subscription jobs
    List,
    /// Show a background subscription job
    Status {
        /// Subscription job ID
        #[arg(value_name = "JOB_ID")]
        job_id: String,
    },
    /// Stop a background subscription job
    Stop {
        /// Subscription job ID
        #[arg(value_name = "JOB_ID")]
        job_id: String,
    },
}

#[derive(Subcommand)]
enum CacheCommands {
    /// List cache entries
    List,

    /// Show cache statistics
    Stats,

    /// Clear cache entries
    Clear {
        /// Optional URL to clear specific cache entry
        #[arg(conflicts_with_all = ["all", "key"])]
        url: Option<String>,

        /// Clear all cached entries
        #[arg(long, conflicts_with_all = ["key", "url"])]
        all: bool,

        /// Cache key to clear
        #[arg(long, conflicts_with = "url")]
        key: Option<String>,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Manage credentials
    Credential {
        #[command(subcommand)]
        credential_command: AuthCredentialCommands,
    },

    /// Alias for `auth credential info`
    Info {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },

    /// Manage endpoint auth bindings
    Binding {
        #[command(subcommand)]
        binding_command: AuthBindingCommands,
    },

    /// Manage app-credential token bootstrap
    Bootstrap {
        #[command(subcommand)]
        bootstrap_command: AuthBootstrapCommands,
    },

    /// Manage OAuth credentials
    Oauth {
        #[command(subcommand)]
        oauth_command: AuthOauthCommands,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum AuthCredentialCommands {
    /// List all credentials
    List,

    /// Show information about a specific credential
    Info {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },

    /// Set or update a credential
    Set {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,

        /// Authentication type (bearer, api_key, basic, oauth)
        #[arg(short = 't', long)]
        auth_type: Option<String>,

        /// Literal secret value
        #[arg(long, conflicts_with_all = ["secret_env", "secret_op"])]
        secret: Option<String>,

        /// Environment variable key containing secret
        #[arg(long, conflicts_with_all = ["secret", "secret_op"])]
        secret_env: Option<String>,

        /// 1Password secret reference (op://...)
        #[arg(long, conflicts_with_all = ["secret", "secret_env"])]
        secret_op: Option<String>,

        /// API key header name shortcut (equivalent to --header "<name>={{secret}}")
        #[arg(long, conflicts_with = "header")]
        api_key_header: Option<String>,

        /// Custom auth header template (repeatable): <name>=<template>
        /// Template supports {{secret}}, {{env:VAR}}, {{op://...}}
        #[arg(long, conflicts_with = "api_key_header")]
        header: Vec<String>,

        /// Custom auth query param template (repeatable): <name>=<template>
        /// Template supports {{secret}}, {{env:VAR}}, {{op://...}}
        #[arg(long)]
        query_param: Vec<String>,

        /// Request path prefix template: <template>
        /// Template supports {{secret}}, {{field:name}}, {{env:VAR}}, {{op://...}}
        #[arg(long)]
        path_prefix_template: Option<String>,

        /// Named auth field source (repeatable): <field-name>=<source>
        /// Source supports literal:<value>, env:<VAR>, op://...
        #[arg(long)]
        field: Vec<String>,

        /// Credential description
        #[arg(long)]
        description: Option<String>,
    },

    /// Remove a credential
    Remove {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },
}

#[derive(Subcommand)]
enum AuthBindingCommands {
    /// List all endpoint auth bindings
    List,

    /// Add a binding rule
    Add {
        /// Binding ID
        #[arg(long, value_name = "BINDING_ID")]
        id: String,

        /// Endpoint host (exact match)
        #[arg(long)]
        host: String,

        /// Optional path prefix
        #[arg(long)]
        path_prefix: Option<String>,

        /// Optional URL scheme (http/https)
        #[arg(long)]
        scheme: Option<String>,

        /// Credential ID to bind
        #[arg(long)]
        credential: String,

        /// Structured signer config JSON
        #[arg(long)]
        signer_json: Option<String>,

        /// Priority (higher wins)
        #[arg(long, default_value_t = 0)]
        priority: i32,

        /// Disable binding
        #[arg(long)]
        disabled: bool,
    },

    /// Remove a binding rule
    Remove {
        /// Binding ID
        #[arg(value_name = "BINDING_ID")]
        binding_id: String,
    },

    /// Match endpoint against bindings
    Match {
        /// Endpoint URL
        #[arg(value_name = "ENDPOINT")]
        endpoint: String,
    },
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum AuthOauthCommands {
    /// List OAuth credentials
    List,

    /// Start non-interactive OAuth authorization_code login
    Start {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,

        /// Service endpoint URL used for OAuth discovery
        #[arg(long)]
        endpoint: String,

        /// OAuth scope (can be repeated)
        #[arg(long)]
        scope: Vec<String>,

        /// OAuth client ID
        #[arg(long)]
        client_id: Option<String>,

        /// OAuth client secret
        #[arg(long)]
        client_secret: Option<String>,

        /// Redirect URI for authorization_code flow
        #[arg(long)]
        redirect_uri: String,

        /// OAuth issuer URL (overrides auto-discovery)
        #[arg(long)]
        issuer: Option<String>,

        /// OAuth authorization endpoint URL (overrides auto-discovery)
        #[arg(long)]
        authorization_endpoint: Option<String>,

        /// OAuth token endpoint URL (overrides auto-discovery)
        #[arg(long)]
        token_endpoint: Option<String>,

        /// OAuth dynamic client registration endpoint URL (overrides auto-discovery)
        #[arg(long)]
        registration_endpoint: Option<String>,

        /// OAuth resource metadata URL (overrides auto-discovery)
        #[arg(long)]
        resource_metadata_url: Option<String>,
    },

    /// Complete non-interactive OAuth authorization_code login
    Complete {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,

        /// OAuth session ID
        #[arg(long)]
        session_id: String,

        /// Authorization response, callback URL, or plain authorization code
        #[arg(
            long = "authorization-response",
            visible_alias = "callback-url",
            visible_alias = "authorization-code"
        )]
        authorization_response: String,
    },

    /// Login with OAuth and save tokens
    Login {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,

        /// Service endpoint URL used for OAuth discovery
        #[arg(long)]
        endpoint: String,

        /// OAuth flow type
        #[arg(long, default_value = "device_code")]
        flow: String,

        /// OAuth scope (can be repeated)
        #[arg(long)]
        scope: Vec<String>,

        /// OAuth client ID
        #[arg(long)]
        client_id: Option<String>,

        /// OAuth client secret
        #[arg(long)]
        client_secret: Option<String>,

        /// Redirect URI for authorization_code flow
        #[arg(long)]
        redirect_uri: Option<String>,

        /// Authorization code or callback URL for authorization_code flow
        #[arg(long)]
        authorization_code: Option<String>,

        /// OAuth issuer URL (overrides auto-discovery)
        #[arg(long)]
        issuer: Option<String>,

        /// OAuth authorization endpoint URL (overrides auto-discovery)
        #[arg(long)]
        authorization_endpoint: Option<String>,

        /// OAuth token endpoint URL (overrides auto-discovery)
        #[arg(long)]
        token_endpoint: Option<String>,

        /// OAuth device authorization endpoint URL (overrides auto-discovery)
        #[arg(long)]
        device_authorization_endpoint: Option<String>,

        /// OAuth dynamic client registration endpoint URL (overrides auto-discovery)
        #[arg(long)]
        registration_endpoint: Option<String>,

        /// OAuth resource metadata URL (overrides auto-discovery)
        #[arg(long)]
        resource_metadata_url: Option<String>,
    },

    /// Refresh OAuth token
    Refresh {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },

    /// Show OAuth credential information
    Info {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },

    /// Remove OAuth token data from credential
    Logout {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },
}

#[derive(Subcommand)]
enum AuthBootstrapCommands {
    /// Configure token bootstrap for a credential
    Set {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,

        /// Token endpoint URL
        #[arg(long)]
        token_endpoint: String,

        /// JSON request body template
        #[arg(long)]
        request_json: String,

        /// Optional bootstrap request header template (repeatable): <name>=<template>
        #[arg(long)]
        header: Vec<String>,

        /// JSON pointer for the access token in the response
        #[arg(long)]
        access_token_pointer: String,

        /// JSON pointer for expires_in seconds in the response
        #[arg(long)]
        expires_in_pointer: String,

        /// Optional JSON pointer for token_type in the response
        #[arg(long)]
        token_type_pointer: Option<String>,

        /// Optional JSON pointer for success-code validation
        #[arg(long)]
        success_code_pointer: Option<String>,

        /// Expected JSON literal at success-code pointer
        #[arg(long)]
        success_code_value: Option<String>,

        /// Refresh skew in seconds
        #[arg(long, default_value_t = 60)]
        refresh_skew_seconds: i64,
    },

    /// Show token bootstrap configuration and state
    Info {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },

    /// Force refresh a bootstrap-backed token
    Refresh {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },

    /// Remove token bootstrap configuration and state
    Remove {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,
    },
}

enum EndpointCommand {
    HostHelp,
    Describe {
        operation_id: String,
    },
    Execute {
        operation_id: String,
        args: Vec<String>,
        input_json: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct OperationSummary {
    operation_id: String,
    display_name: String,
    summary: Option<String>,
    required: Vec<String>,
    input_shape_hint: String,
    protocol_kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HostHelpData {
    operations: Vec<OperationSummary>,
    count: usize,
    examples: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service: Option<ServiceSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ServiceSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelpData {
    path: String,
    about: String,
    usage: String,
    commands: Vec<HelpCommand>,
    notes: Vec<String>,
    examples: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelpCommand {
    name: String,
    about: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheClearData {
    scope: String,
    url: Option<String>,
    key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheListData {
    entries: Vec<cache::CacheListEntry>,
    count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthProfileView {
    name: String,
    auth_type: String,
    api_key_masked: String,
    secret_source: Option<AuthSecretSourceView>,
    fields: Option<Vec<AuthFieldView>>,
    auth_headers: Option<Vec<AuthHeaderView>>,
    auth_query_params: Option<Vec<AuthQueryParamView>>,
    auth_path_prefix: Option<AuthPathPrefixView>,
    description: Option<String>,
    oauth: Option<AuthOAuthView>,
    bootstrap: Option<AuthBootstrapView>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthSecretSourceView {
    kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthHeaderView {
    name: String,
    value_masked: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthFieldView {
    name: String,
    source_kind: String,
    value_masked: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthQueryParamView {
    name: String,
    value_masked: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthPathPrefixView {
    value_masked: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthOAuthView {
    flow: Option<String>,
    provider_issuer: Option<String>,
    resource_metadata_url: Option<String>,
    scopes: Vec<String>,
    expires_at: Option<i64>,
    has_refresh_token: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBootstrapView {
    token_endpoint: String,
    request_json_masked: String,
    headers: Option<Vec<AuthHeaderView>>,
    access_token_pointer: String,
    expires_in_pointer: String,
    token_type_pointer: Option<String>,
    success_code_pointer: Option<String>,
    success_code_value: Option<String>,
    refresh_skew_seconds: i64,
    token_present: bool,
    expires_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBootstrapInfoData {
    credential: String,
    auth_type: String,
    fields: Option<Vec<AuthFieldView>>,
    bootstrap: AuthBootstrapView,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthListData {
    credentials: Vec<AuthProfileView>,
    count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthRemoveData {
    credential: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthOAuthStartData {
    credential: String,
    flow: String,
    session_id: String,
    authorization_url: String,
    redirect_uri: String,
    expires_at: i64,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBindingListData {
    bindings: Vec<AuthBindingRule>,
    count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBindingMatchData {
    endpoint: String,
    matched: bool,
    binding: Option<AuthBindingRule>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBindingSetData {
    id: String,
    credential: String,
    host: String,
    path_prefix: Option<String>,
    scheme: Option<String>,
    signer: Option<auth::AuthSignerConfig>,
    priority: i32,
    enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBindingRemoveData {
    binding_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SubscribeListData {
    jobs: Vec<daemon::SubscriptionJobView>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LinkCreateData {
    name: String,
    host: String,
    path: String,
    overwritten: bool,
    dir_in_path: bool,
    schema_url: Option<String>,
    credential: Option<String>,
    inject_env: Vec<String>,
    daemon_idle_ttl: Option<u64>,
}

struct LinkCommandOptions<'a> {
    dir: Option<&'a str>,
    schema_url: Option<&'a str>,
    credential: Option<&'a str>,
    explicit_auth: Option<&'a str>,
    inject_env: &'a [InjectEnvSpec],
    force: bool,
    daemon_exclusive: &'a [String],
    daemon_idle_ttl: Option<u64>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let raw_args: Vec<String> = std::env::args().collect();
    if is_version_shortcut(&raw_args) {
        print_version();
        return;
    }

    let normalized_args = normalize_global_args(raw_args);
    let fallback_output_mode = output_mode_from_args(&normalized_args);

    if let Err(err) = run(normalized_args, fallback_output_mode).await {
        render_error(&err, fallback_output_mode);
        std::process::exit(1);
    }
}

fn is_version_shortcut(args: &[String]) -> bool {
    args.len() == 2 && matches!(args[1].as_str(), "-v" | "version")
}

fn print_version() {
    println!("uxc {}", env!("CARGO_PKG_VERSION"));
}

fn render_error(err: &anyhow::Error, output_mode: OutputMode) {
    if output_mode == OutputMode::Text {
        eprintln!("{}", err);
        return;
    }

    let structured = structured_error_from_anyhow(err);
    let code = structured
        .as_ref()
        .map(|payload| payload.code.as_str())
        .unwrap_or_else(|| error_code(err));
    let message = structured
        .as_ref()
        .map(|payload| payload.message.clone())
        .unwrap_or_else(|| err.to_string());
    let details = structured
        .as_ref()
        .and_then(|payload| payload.details.clone());
    let envelope = OutputEnvelope::error_with_details(code, &message, details);
    match envelope.to_json() {
        Ok(json) => println!("{}", json),
        Err(ser_err) => {
            eprintln!("failed to serialize error output: {}", ser_err);
            eprintln!("{}", err);
        }
    }
}

async fn run(args: Vec<String>, fallback_output_mode: OutputMode) -> Result<()> {
    let parse_result = Cli::try_parse_from(args.clone());
    let cli = match parse_result {
        Ok(cli) => cli,
        Err(parse_err) => {
            if matches!(parse_err.kind(), ErrorKind::DisplayVersion) {
                print_version();
                return Ok(());
            }
            if let Some(help_path) = help_path_from_parse_error(&args, &parse_err) {
                let envelope = if help_path.is_empty() {
                    global_help_envelope()?
                } else {
                    let help_path_refs = help_path.iter().map(String::as_str).collect::<Vec<_>>();
                    subcommand_help_envelope(&help_path_refs)?
                };
                return render_output(&envelope, fallback_output_mode);
            }
            return Err(UxcError::InvalidArguments(parse_err.to_string()).into());
        }
    };

    let output_mode = resolve_output_mode(&cli);
    let envelope = execute_cli(&cli).await?;
    render_output(&envelope, output_mode)
}

fn resolve_output_mode(cli: &Cli) -> OutputMode {
    if cli.text || cli.format == Some(OutputFormat::Text) {
        OutputMode::Text
    } else {
        OutputMode::Json
    }
}

fn output_mode_from_args(args: &[String]) -> OutputMode {
    if args.iter().any(|arg| arg == "--text") {
        return OutputMode::Text;
    }

    for (idx, arg) in args.iter().enumerate() {
        if arg == "--format" {
            if let Some(value) = args.get(idx + 1) {
                if value == "text" {
                    return OutputMode::Text;
                }
            }
        } else if arg == "--format=text" {
            return OutputMode::Text;
        }
    }

    OutputMode::Json
}

fn normalize_global_args(raw_args: Vec<String>) -> Vec<String> {
    if raw_args.len() <= 1 {
        return raw_args;
    }

    let mut normalized = vec![raw_args[0].clone()];
    let mut global_args = Vec::new();
    let mut rest_args = Vec::new();
    let mut idx = 1;

    while idx < raw_args.len() {
        let arg = &raw_args[idx];
        let is_global_bool = matches!(arg.as_str(), "--text" | "--no-cache" | "--refresh-schema");
        let is_global_kv = matches!(
            arg.as_str(),
            "--format"
                | "--auth"
                | "--cache-ttl"
                | "--schema-url"
                | "--daemon-exclusive"
                | "--daemon-idle-ttl"
                | "--inject-env"
        );
        let is_global_inline = arg.starts_with("--format=")
            || arg.starts_with("--auth=")
            || arg.starts_with("--cache-ttl=")
            || arg.starts_with("--schema-url=")
            || arg.starts_with("--daemon-exclusive=")
            || arg.starts_with("--daemon-idle-ttl=")
            || arg.starts_with("--inject-env=");

        if is_global_bool || is_global_inline {
            global_args.push(arg.clone());
            idx += 1;
            continue;
        }

        if is_global_kv {
            global_args.push(arg.clone());
            if let Some(value) = raw_args.get(idx + 1) {
                if !value.starts_with("--") {
                    global_args.push(value.clone());
                    idx += 2;
                } else {
                    idx += 1;
                }
            } else {
                idx += 1;
            }
            continue;
        }

        rest_args.push(arg.clone());
        idx += 1;
    }

    normalized.extend(global_args);
    normalized.extend(rest_args);
    normalized
}

fn is_global_bool_arg(arg: &str) -> bool {
    matches!(
        arg,
        "--text" | "--no-cache" | "--refresh-schema" | "-h" | "--help"
    )
}

fn is_global_kv_arg(arg: &str) -> bool {
    matches!(
        arg,
        "--format"
            | "--auth"
            | "--cache-ttl"
            | "--schema-url"
            | "--daemon-exclusive"
            | "--daemon-idle-ttl"
            | "--inject-env"
    )
}

fn is_global_inline_arg(arg: &str) -> bool {
    arg.starts_with("--format=")
        || arg.starts_with("--auth=")
        || arg.starts_with("--cache-ttl=")
        || arg.starts_with("--schema-url=")
        || arg.starts_with("--daemon-exclusive=")
        || arg.starts_with("--daemon-idle-ttl=")
        || arg.starts_with("--inject-env=")
}

fn non_global_tokens(raw_args: &[String]) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut idx = 1;

    while idx < raw_args.len() {
        let arg = &raw_args[idx];

        if is_global_bool_arg(arg) || is_global_inline_arg(arg) {
            idx += 1;
            continue;
        }

        if is_global_kv_arg(arg) {
            idx += 1;
            if idx < raw_args.len() && !raw_args[idx].starts_with("--") {
                idx += 1;
            }
            continue;
        }

        tokens.push(arg.clone());
        idx += 1;
    }

    tokens
}

fn is_help_token(arg: &str) -> bool {
    matches!(arg, "-h" | "--help" | "help")
}

fn raw_has_help_token(raw_args: &[String]) -> bool {
    raw_args.iter().skip(1).any(|arg| is_help_token(arg))
}

fn is_top_level_command_token(token: &str) -> bool {
    matches!(
        token,
        "help" | "cache" | "auth" | "link" | "daemon" | "subscribe"
    )
}

fn infer_help_path_from_tokens(tokens: &[String]) -> Option<Vec<String>> {
    if tokens.is_empty() {
        return Some(vec![]);
    }

    if tokens[0] == "help" {
        return Some(vec![]);
    }

    let mut idx = 0usize;
    if !is_top_level_command_token(&tokens[idx]) {
        if tokens
            .get(idx + 1)
            .is_some_and(|next| is_top_level_command_token(next))
        {
            idx += 1;
        } else {
            return None;
        }
    }

    let mut path = vec![tokens[idx].clone()];
    idx += 1;

    match path[0].as_str() {
        "cache" => {
            if let Some(level1) = tokens.get(idx).map(|s| s.as_str()) {
                if matches!(level1, "clear" | "stats") {
                    path.push(level1.to_string());
                }
            }
        }
        "auth" => {
            if let Some(level1) = tokens.get(idx).map(|s| s.as_str()) {
                match level1 {
                    "info" => {
                        path.push("info".to_string());
                    }
                    "credential" => {
                        path.push("credential".to_string());
                        if let Some(level2) = tokens.get(idx + 1).map(|s| s.as_str()) {
                            if matches!(level2, "list" | "info" | "set" | "remove") {
                                path.push(level2.to_string());
                            }
                        }
                    }
                    "binding" => {
                        path.push("binding".to_string());
                        if let Some(level2) = tokens.get(idx + 1).map(|s| s.as_str()) {
                            if matches!(level2, "list" | "add" | "remove" | "match") {
                                path.push(level2.to_string());
                            }
                        }
                    }
                    "oauth" => {
                        path.push("oauth".to_string());
                        if let Some(level2) = tokens.get(idx + 1).map(|s| s.as_str()) {
                            if matches!(level2, "list" | "login" | "refresh" | "info" | "logout") {
                                path.push(level2.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "daemon" => {
            if let Some(level1) = tokens.get(idx).map(|s| s.as_str()) {
                if matches!(level1, "start" | "stop" | "status" | "restart" | "_serve") {
                    path.push(level1.to_string());
                }
            }
        }
        "subscribe" => {
            if let Some(level1) = tokens.get(idx).map(|s| s.as_str()) {
                if matches!(level1, "start" | "list" | "status" | "stop") {
                    path.push(level1.to_string());
                }
            }
        }
        _ => {}
    }

    Some(path)
}

fn help_path_from_parse_error(raw_args: &[String], parse_err: &clap::Error) -> Option<Vec<String>> {
    let kind = parse_err.kind();
    let is_missing_subcommand = matches!(
        kind,
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    let is_help_like_error = matches!(
        kind,
        ErrorKind::MissingRequiredArgument
            | ErrorKind::InvalidSubcommand
            | ErrorKind::UnknownArgument
            | ErrorKind::DisplayHelp
    );

    if !is_missing_subcommand && !is_help_like_error {
        return None;
    }

    if !is_missing_subcommand && !raw_has_help_token(raw_args) {
        return None;
    }

    let tokens = non_global_tokens(raw_args);
    infer_help_path_from_tokens(&tokens).or(Some(vec![]))
}

fn static_help_path_from_cli(cli: &Cli) -> Option<Vec<&'static str>> {
    if matches!(cli.command, Some(Commands::External(_))) {
        return None;
    }

    if cli.command.is_none() {
        if cli.url.is_none() || cli.url.as_deref() == Some("help") {
            return Some(vec![]);
        }
        return None;
    }

    if !cli.help {
        return None;
    }

    match &cli.command {
        Some(Commands::Cache { cache_command }) => match cache_command {
            CacheCommands::List => Some(vec!["cache", "list"]),
            CacheCommands::Clear { .. } => Some(vec!["cache", "clear"]),
            CacheCommands::Stats => Some(vec!["cache", "stats"]),
        },
        Some(Commands::Auth { auth_command }) => match auth_command {
            AuthCommands::Credential { credential_command } => match credential_command {
                AuthCredentialCommands::List => Some(vec!["auth", "credential", "list"]),
                AuthCredentialCommands::Info { .. } => Some(vec!["auth", "credential", "info"]),
                AuthCredentialCommands::Set { .. } => Some(vec!["auth", "credential", "set"]),
                AuthCredentialCommands::Remove { .. } => Some(vec!["auth", "credential", "remove"]),
            },
            AuthCommands::Info { .. } => Some(vec!["auth", "info"]),
            AuthCommands::Binding { binding_command } => match binding_command {
                AuthBindingCommands::List => Some(vec!["auth", "binding", "list"]),
                AuthBindingCommands::Add { .. } => Some(vec!["auth", "binding", "add"]),
                AuthBindingCommands::Remove { .. } => Some(vec!["auth", "binding", "remove"]),
                AuthBindingCommands::Match { .. } => Some(vec!["auth", "binding", "match"]),
            },
            AuthCommands::Bootstrap { bootstrap_command } => match bootstrap_command {
                AuthBootstrapCommands::Set { .. } => Some(vec!["auth", "bootstrap", "set"]),
                AuthBootstrapCommands::Info { .. } => Some(vec!["auth", "bootstrap", "info"]),
                AuthBootstrapCommands::Refresh { .. } => Some(vec!["auth", "bootstrap", "refresh"]),
                AuthBootstrapCommands::Remove { .. } => Some(vec!["auth", "bootstrap", "remove"]),
            },
            AuthCommands::Oauth { oauth_command } => match oauth_command {
                AuthOauthCommands::List => Some(vec!["auth", "oauth", "list"]),
                AuthOauthCommands::Start { .. } => Some(vec!["auth", "oauth", "start"]),
                AuthOauthCommands::Complete { .. } => Some(vec!["auth", "oauth", "complete"]),
                AuthOauthCommands::Login { .. } => Some(vec!["auth", "oauth", "login"]),
                AuthOauthCommands::Refresh { .. } => Some(vec!["auth", "oauth", "refresh"]),
                AuthOauthCommands::Info { .. } => Some(vec!["auth", "oauth", "info"]),
                AuthOauthCommands::Logout { .. } => Some(vec!["auth", "oauth", "logout"]),
            },
        },
        Some(Commands::Link { .. }) => Some(vec!["link"]),
        Some(Commands::Daemon { daemon_command }) => match daemon_command {
            DaemonCommands::Start => Some(vec!["daemon", "start"]),
            DaemonCommands::Stop => Some(vec!["daemon", "stop"]),
            DaemonCommands::Status => Some(vec!["daemon", "status"]),
            DaemonCommands::Sessions => Some(vec!["daemon", "sessions"]),
            DaemonCommands::Restart => Some(vec!["daemon", "restart"]),
            DaemonCommands::Serve => Some(vec!["daemon", "_serve"]),
        },
        Some(Commands::Subscribe { subscribe_command }) => match subscribe_command {
            SubscribeCommands::Start { .. } => Some(vec!["subscribe", "start"]),
            SubscribeCommands::List => Some(vec!["subscribe", "list"]),
            SubscribeCommands::Status { .. } => Some(vec!["subscribe", "status"]),
            SubscribeCommands::Stop { .. } => Some(vec!["subscribe", "stop"]),
        },
        Some(Commands::External(_)) | None => None,
    }
}

fn normalize_endpoint_url(input: &str) -> String {
    let normalized = match infer_scheme_for_endpoint(input) {
        Some(scheme) => format!("{}://{}", scheme, input),
        None => input.to_string(),
    };
    absolutize_stdio_command_endpoint(&normalized)
}

fn absolutize_stdio_command_endpoint(input: &str) -> String {
    if !adapters::mcp::McpAdapter::is_stdio_command(input) {
        return input.to_string();
    }

    let parts = adapters::mcp::transport::parse_command(input);
    let Some((command, args)) = parts.split_first() else {
        return input.to_string();
    };

    let Some(absolute_command) = absolutize_stdio_command_path(command) else {
        return input.to_string();
    };

    let mut rebuilt = Vec::with_capacity(parts.len());
    rebuilt.push(absolute_command);
    rebuilt.extend(args.iter().cloned());
    rebuilt
        .into_iter()
        .map(|part| quote_stdio_command_part(&part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn absolutize_stdio_command_path(command: &str) -> Option<String> {
    if command.contains("://") {
        return None;
    }

    let expanded = if command == "~" || command.starts_with("~/") || command.starts_with("~\\") {
        let home = resolve_home_dir()?;
        if command == "~" {
            home
        } else {
            home.join(command[2..].replace('\\', "/"))
        }
    } else {
        let path = PathBuf::from(command);
        if path.is_absolute() {
            return None;
        }
        let looks_like_path = command.starts_with("./")
            || command.starts_with("../")
            || command.contains('/')
            || command.contains('\\');
        if !looks_like_path {
            return None;
        }
        std::env::current_dir().ok()?.join(path)
    };

    Some(
        fs::canonicalize(&expanded)
            .unwrap_or(expanded)
            .to_string_lossy()
            .into_owned(),
    )
}

fn quote_stdio_command_part(part: &str) -> String {
    if part.is_empty()
        || part.chars().any(char::is_whitespace)
        || part.contains('\'')
        || part.contains('"')
    {
        shell_single_quote(part)
    } else {
        part.to_string()
    }
}

fn infer_scheme_for_endpoint(input: &str) -> Option<&'static str> {
    if input.is_empty()
        || input.contains("://")
        || input.chars().any(char::is_whitespace)
        || input.starts_with('-')
        || input.starts_with('/')
        || input.starts_with("./")
        || input.starts_with("../")
        || input.starts_with('~')
        || input.contains('\\')
        || looks_like_operation_id(input)
    {
        return None;
    }

    let parsed = url::Url::parse(&format!("http://{}", input)).ok()?;
    let host = parsed.host_str()?;
    let is_ip = host.parse::<IpAddr>().is_ok();
    let is_local = host.eq_ignore_ascii_case("localhost") || host.ends_with(".local");
    let has_dot = host.contains('.');

    // Keep short single-segment tokens unchanged (e.g. operation IDs or aliases).
    if !(has_dot || is_local || is_ip) {
        return None;
    }

    let has_non_root_path = parsed.path() != "/";
    let has_explicit_port = parsed.port().is_some();

    // host:port without path is ambiguous (could be gRPC/MCP/http); require explicit scheme.
    if has_explicit_port && !has_non_root_path && !is_local && !is_ip {
        return None;
    }

    if is_local || is_ip {
        Some("http")
    } else {
        Some("https")
    }
}

fn looks_like_operation_id(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    [
        "get:/",
        "post:/",
        "put:/",
        "patch:/",
        "delete:/",
        "head:/",
        "options:/",
        "trace:/",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.starts_with("query/")
        || lower.starts_with("mutation/")
        || lower.starts_with("subscription/")
}

async fn execute_cli(cli: &Cli) -> Result<OutputEnvelope> {
    if let Some(help_path) = static_help_path_from_cli(cli) {
        if help_path.is_empty() {
            return global_help_envelope();
        }
        return subcommand_help_envelope(&help_path);
    }

    let cache_config = if cli.no_cache {
        CacheConfig {
            enabled: false,
            ..Default::default()
        }
    } else if let Some(ttl) = cli.cache_ttl {
        CacheConfig {
            ttl,
            ..Default::default()
        }
    } else {
        CacheConfig::load_from_file().unwrap_or_default()
    };

    if let Some(Commands::Cache { cache_command }) = &cli.command {
        return handle_cache_command(cache_command, cache_config).await;
    }

    if let Some(Commands::Auth { auth_command }) = &cli.command {
        return handle_auth_command(auth_command).await;
    }

    if let Some(Commands::Link {
        name,
        host,
        dir,
        schema_url,
        credential,
        force,
    }) = &cli.command
    {
        let exclusive = collect_daemon_exclusive_keys(cli)?;
        let inject_env = collect_inject_env_specs(cli)?;
        let options = LinkCommandOptions {
            dir: dir.as_deref(),
            schema_url: schema_url.as_deref(),
            credential: credential.as_deref(),
            explicit_auth: cli.auth.as_deref(),
            inject_env: &inject_env,
            force: *force,
            daemon_exclusive: &exclusive,
            daemon_idle_ttl: collect_daemon_idle_ttl(cli)?,
        };
        return handle_link_command(name, host, options).await;
    }

    if let Some(Commands::Daemon { daemon_command }) = &cli.command {
        return handle_daemon_command(daemon_command).await;
    }

    if let Some(Commands::Subscribe { subscribe_command }) = &cli.command {
        return handle_subscribe_command(subscribe_command, cli).await;
    }

    let url = cli
        .url
        .clone()
        .ok_or_else(|| UxcError::InvalidArguments("URL is required".to_string()))
        .map(|raw| normalize_endpoint_url(&raw))?;

    let endpoint_command = resolve_endpoint_command(cli)?;
    execute_endpoint_via_daemon(&url, &endpoint_command, cli).await
}

async fn execute_endpoint_via_daemon(
    url: &str,
    endpoint_command: &EndpointCommand,
    cli: &Cli,
) -> Result<OutputEnvelope> {
    info!("UXC v{} - connecting to {}", env!("CARGO_PKG_VERSION"), url);

    let daemon_used = daemon::daemon_supported();
    let daemon_ensure = if daemon_used {
        Some(daemon::ensure_compatible_daemon_running().await?)
    } else {
        None
    };
    let daemon_autostarted = daemon_ensure
        .as_ref()
        .map(|outcome| outcome.started_now && !outcome.restarted_for_version_mismatch);
    let daemon_restarted_for_version_mismatch = daemon_ensure
        .as_ref()
        .map(|outcome| outcome.restarted_for_version_mismatch);
    let (action, operation_id, args_map) = match endpoint_command {
        EndpointCommand::HostHelp => (daemon::RuntimeAction::HostHelp, None, None),
        EndpointCommand::Describe { operation_id } => (
            daemon::RuntimeAction::OperationHelp,
            Some(operation_id.clone()),
            None,
        ),
        EndpointCommand::Execute {
            operation_id,
            args,
            input_json,
        } => (
            daemon::RuntimeAction::Execute,
            Some(operation_id.clone()),
            Some(parse_arguments(args.clone(), input_json.clone())?),
        ),
    };

    let request = daemon::RuntimeInvokeRequest {
        request_id: format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ),
        endpoint: url.to_string(),
        action,
        operation_id,
        args: args_map,
        options: daemon::RuntimeInvokeOptions {
            auth: cli.auth.clone(),
            inject_env: collect_inject_env_specs(cli)?,
            no_cache: cli.no_cache,
            cache_ttl: cli.cache_ttl,
            timeout_ms: cli.timeout_ms,
            refresh_schema: cli.refresh_schema,
            schema_url: cli.schema_url.as_deref().map(normalize_endpoint_url),
            link_name: std::env::var("UXC_LINK_NAME").ok(),
            schema_mapping_file: std::env::var("UXC_SCHEMA_MAPPINGS_FILE").ok(),
            daemon_exclusive: collect_daemon_exclusive_keys(cli)?,
            daemon_idle_ttl: collect_daemon_idle_ttl(cli)?,
        },
    };

    let response = daemon::runtime_invoke_client(&request).await?;
    let command_head = request
        .options
        .link_name
        .as_deref()
        .unwrap_or("uxc <host>")
        .to_string();
    let mut response_data = response.data;
    if response.kind == "operation_detail" {
        if let Some(operation_id) = response.operation.as_deref() {
            response_data =
                enrich_operation_detail_payload(response_data, &command_head, operation_id);
        }
    }
    let mut envelope = OutputEnvelope::success(
        &response.kind,
        &response.protocol,
        &response.endpoint,
        response.operation.as_deref(),
        response_data,
        response.duration_ms,
    )
    .with_daemon_meta(
        daemon_used,
        daemon_autostarted,
        daemon_restarted_for_version_mismatch,
        response.meta.daemon_session_reused,
    );

    if let Some(schema_involved) = response.meta.schema_involved {
        envelope = envelope.with_schema_meta(
            schema_involved,
            response.meta.cache_source.as_deref(),
            response.meta.cache_age_ms,
            response.meta.cache_stale,
            response.meta.cache_fallback,
        );
    }
    Ok(envelope)
}

fn global_help_envelope() -> Result<OutputEnvelope> {
    let data = serde_json::to_value(help_data_for_path(&[]))?;

    Ok(OutputEnvelope::success(
        "global_help",
        "cli",
        "uxc",
        None,
        data,
        None,
    ))
}

fn subcommand_help_envelope(path: &[&str]) -> Result<OutputEnvelope> {
    let data = serde_json::to_value(help_data_for_path(path))?;
    Ok(OutputEnvelope::success(
        "subcommand_help",
        "cli",
        "uxc",
        None,
        data,
        None,
    ))
}

fn commands(entries: &[(&str, &str)]) -> Vec<HelpCommand> {
    entries
        .iter()
        .map(|(name, about)| HelpCommand {
            name: (*name).to_string(),
            about: (*about).to_string(),
        })
        .collect()
}

fn help_data_for_path(path: &[&str]) -> HelpData {
    match path {
        [] => HelpData {
            path: "uxc".to_string(),
            about: "Universal X-Protocol CLI".to_string(),
            usage: "uxc [OPTIONS] [URL] [COMMAND]".to_string(),
            commands: commands(&[
                ("help", "Show global help"),
                ("cache", "Manage schema cache"),
                ("auth", "Manage credentials, bindings, and OAuth"),
                ("link", "Create a host-bound shortcut command"),
                ("daemon", "Manage local runtime daemon"),
                ("subscribe", "Manage background subscriptions via daemon"),
            ]),
            notes: vec![
                "Default output is JSON. Use --text for human-readable output.".to_string(),
                "For endpoints, use: uxc <host> -h, uxc <host> <operation_id> -h, and uxc <host> <operation_id> ...".to_string(),
                "--inject-env NAME={{secret}} is available for stdio endpoints when a credential is supplied.".to_string(),
            ],
            examples: vec![
                "uxc -h".to_string(),
                "uxc <host> -h".to_string(),
                "uxc <host> <operation_id> -h".to_string(),
                "uxc <host> <operation_id> key=value".to_string(),
                "uxc --auth thegraph --inject-env THEGRAPH_API_KEY={{secret}} \"npx -y mcp-remote --header \\\"Authorization: Bearer ${THEGRAPH_API_KEY}\\\" https://subgraphs.mcp.thegraph.com/sse\" -h".to_string(),
            ],
        },
        ["link"] => HelpData {
            path: "uxc link".to_string(),
            about: "Create a host-bound shortcut command".to_string(),
            usage:
                "uxc link <name> <host> [--dir <dir>] [--schema-url <url>] [--credential <credential_id>] [--inject-env NAME={{secret}} ...] [--daemon-exclusive <key> ...] [--daemon-idle-ttl <seconds>] [--force]"
                    .to_string(),
            commands: vec![],
            notes: vec![
                "Use --schema-url to persist a default OpenAPI schema URL in the shortcut; callers can still override it by passing --schema-url explicitly."
                    .to_string(),
                "Use --credential and --inject-env for stdio shortcuts that need child-process env auth injection."
                    .to_string(),
                "Use --daemon-exclusive to declare shared state keys that should be exclusive across MCP stdio sessions."
                    .to_string(),
                "Use --daemon-idle-ttl to override daemon stdio idle cleanup per link; 0 disables idle reaping for that linked session."
                    .to_string(),
            ],
            examples: vec![
                "uxc link petcli petstore3.swagger.io/api/v3".to_string(),
                "uxc link discord-openapi-cli https://discord.com/api/v10 --schema-url https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json".to_string(),
                "uxc link thegraph-mcp-cli \"/bin/zsh -lc 'npx -y mcp-remote --header \\\"Authorization: Bearer ${THEGRAPH_API_KEY}\\\" https://subgraphs.mcp.thegraph.com/sse'\" --credential thegraph --inject-env THEGRAPH_API_KEY={{secret}}".to_string(),
                "uxc link --daemon-exclusive ~/.uxc/playwright-profile --daemon-idle-ttl 0 playwright-mcp-ui \"npx -y @playwright/mcp@latest --user-data-dir ~/.uxc/playwright-profile\"".to_string(),
                "petcli -h".to_string(),
            ],
        },
        ["daemon"] => HelpData {
            path: "uxc daemon".to_string(),
            about: "Manage local runtime daemon".to_string(),
            usage: "uxc daemon <start|stop|status|sessions|restart>".to_string(),
            commands: commands(&[
                ("start", "Start daemon process"),
                ("stop", "Stop daemon process"),
                ("status", "Show daemon status"),
                ("sessions", "List daemon MCP sessions"),
                ("restart", "Restart daemon process"),
            ]),
            notes: vec![
                "Endpoint invocations auto-start daemon when needed.".to_string(),
                "Daemon serves endpoint requests over local Unix socket JSON-RPC.".to_string(),
            ],
            examples: vec![
                "uxc daemon status".to_string(),
                "uxc daemon start".to_string(),
                "uxc daemon stop".to_string(),
                "uxc daemon restart".to_string(),
            ],
        },
        ["daemon", "start"] => HelpData {
            path: "uxc daemon start".to_string(),
            about: "Start daemon process".to_string(),
            usage: "uxc daemon start".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc daemon start".to_string()],
        },
        ["daemon", "stop"] => HelpData {
            path: "uxc daemon stop".to_string(),
            about: "Stop daemon process".to_string(),
            usage: "uxc daemon stop".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc daemon stop".to_string()],
        },
        ["daemon", "status"] => HelpData {
            path: "uxc daemon status".to_string(),
            about: "Show daemon status".to_string(),
            usage: "uxc daemon status".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc daemon status".to_string()],
        },
        ["daemon", "sessions"] => HelpData {
            path: "uxc daemon sessions".to_string(),
            about: "List daemon MCP sessions".to_string(),
            usage: "uxc daemon sessions".to_string(),
            commands: vec![],
            notes: vec![
                "Shows live stdio daemon session metadata including command summary, reuse eligibility, recent stderr, per-session idle TTL where 0 disables idle reaping, and the latest uxc/can_reap contract state."
                    .to_string(),
            ],
            examples: vec!["uxc daemon sessions".to_string()],
        },
        ["daemon", "restart"] => HelpData {
            path: "uxc daemon restart".to_string(),
            about: "Restart daemon process".to_string(),
            usage: "uxc daemon restart".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc daemon restart".to_string()],
        },
        ["subscribe"] => HelpData {
            path: "uxc subscribe".to_string(),
            about: "Manage background subscriptions via daemon".to_string(),
            usage: "uxc subscribe <start|list|status|stop> ...".to_string(),
            commands: commands(&[
                ("start", "Start a background subscription job"),
                ("list", "List background subscription jobs"),
                ("status", "Show a background subscription job"),
                ("stop", "Stop a background subscription job"),
            ]),
            notes: vec![
                "Supports stream subscriptions plus polling-based subscriptions under the same daemon-backed job model.".to_string(),
                "Use --sink file:/path.ndjson to append normalized event envelopes to a file."
                    .to_string(),
                "Subscriptions are durable by default and auto-resume after daemon restart; pass --ephemeral to avoid restart recovery."
                    .to_string(),
                "Stream mode covers raw HTTP JSON streams, GraphQL subscriptions, JSON-RPC pubsub over WebSocket, explicit raw WebSocket streams, Slack Socket Mode, and MCP resource subscriptions; poll mode repeatedly executes a normal operation and emits only new items."
                    .to_string(),
                "For MCP resource subscriptions, add --read-resource if the sink should also capture resources/read snapshots after each update notification."
                    .to_string(),
            ],
            examples: vec![
                "uxc subscribe start https://example.com/stream --sink file:/tmp/events.ndjson"
                    .to_string(),
                "uxc subscribe start https://example.com/stream --ephemeral --sink file:/tmp/oneshot.ndjson".to_string(),
                "uxc subscribe start wss://stream.binance.com:9443/ws/btcusdt@trade --transport websocket --sink file:/tmp/binance.ndjson".to_string(),
                "uxc subscribe start https://discord.com/api/v10 --transport discord-gateway --auth discord-bot --sink file:/tmp/discord.ndjson".to_string(),
                "uxc subscribe start https://slack.com/api --transport slack-socket-mode --auth slack-app --sink file:/tmp/slack.ndjson".to_string(),
                "uxc subscribe start https://open.feishu.cn/open-apis --transport feishu-long-connection --auth feishu-tenant --sink file:/tmp/feishu.ndjson".to_string(),
                "uxc subscribe start https://example.com/graphql subscription/messageAdded '{\"roomId\":\"abc\"}' --sink file:/tmp/graphql.ndjson".to_string(),
                "uxc subscribe start wss://example.com/ws eth_subscribe '{\"params\":[\"newHeads\"]}' --sink file:/tmp/heads.ndjson".to_string(),
                "uxc subscribe start https://example.com/api get:/events --mode poll --poll-config '{\"interval_secs\":5,\"extract_items_pointer\":\"/items\",\"request_cursor_arg\":\"cursor\",\"response_cursor_pointer\":\"/next_cursor\",\"checkpoint_strategy\":{\"type\":\"cursor_only\"}}' --sink file:/tmp/poll.ndjson".to_string(),
                "uxc subscribe start https://api.telegram.org post:/getUpdates --mode poll --poll-config '{\"interval_secs\":2,\"extract_items_pointer\":\"/result\",\"request_cursor_arg\":\"offset\",\"cursor_from_item_pointer\":\"/update_id\",\"cursor_transform\":\"increment\",\"checkpoint_strategy\":{\"type\":\"item_key\",\"item_key_pointer\":\"/update_id\"}}' --sink file:/tmp/telegram.ndjson".to_string(),
                "uxc subscribe start https://example.com/mcp --resource-uri file:///tmp/log --read-resource --sink file:/tmp/mcp-http.ndjson".to_string(),
                "uxc subscribe start \"npx -y my-mcp-server\" --resource-uri file:///tmp/log --read-resource --sink file:/tmp/mcp.ndjson".to_string(),
                "uxc subscribe list".to_string(),
                "uxc subscribe status sub_123".to_string(),
                "uxc subscribe stop sub_123".to_string(),
            ],
        },
        ["subscribe", "start"] => HelpData {
            path: "uxc subscribe start".to_string(),
            about: "Start a background subscription job".to_string(),
            usage: "uxc subscribe start <endpoint> [<operation_id> [key=value ... | '{...}']] --sink file:<path> [--ephemeral] [--transport websocket|discord-gateway|slack-socket-mode|feishu-long-connection] [--subprotocol <value> ...] [--init-frame <text-or-json> ...] [--mode <stream|poll>] [--poll-config <json>] [--resource-uri <uri>] [--read-resource]".to_string(),
            commands: vec![],
            notes: vec![
                "For raw HTTP streams, omit <operation_id> and use <endpoint> as the final stream URL.".to_string(),
                "For generic raw WebSocket streams, pass --transport websocket plus a ws:// or wss:// endpoint; --subprotocol and --init-frame are optional and can be repeated independently.".to_string(),
                "For Discord Gateway, pass --transport discord-gateway plus a Discord REST API base such as https://discord.com/api/v10 and a bot token via --auth; optional config may be provided as positional JSON or via --input-json to set intents, os, browser, or device.".to_string(),
                "For Slack Socket Mode, pass --transport slack-socket-mode plus a Slack Web API base endpoint such as https://slack.com/api and an app-level xapp token via --auth; the runtime opens a fresh temporary WebSocket URL on each connect attempt.".to_string(),
                "For Feishu or Lark long connection, pass --transport feishu-long-connection plus an Open Platform base endpoint such as https://open.feishu.cn/open-apis and a credential whose fields include app_id and app_secret; the runtime opens a fresh temporary WebSocket URL on each connect attempt.".to_string(),
                "Raw WebSocket sink events preserve frame type in meta: JSON text frames populate data, plain text frames populate meta.text, and binary frames populate meta.base64.".to_string(),
                "For GraphQL subscriptions, pass subscription/<field>; the runtime derives ws(s) from the HTTP endpoint, reuses auth/cache behavior, and automatically falls back between modern and legacy GraphQL websocket profiles.".to_string(),
                "For JSON-RPC pubsub, pass a ws:// or wss:// endpoint plus a method ending in _subscribe; send raw JSON-RPC params through '{\"params\":...}'.".to_string(),
                "For MCP, pass either an MCP HTTP endpoint or a stdio command plus --resource-uri <uri>; add --read-resource to append resources/read snapshots alongside update notifications.".to_string(),
                "For poll mode, pass a normal operation ID plus --mode poll and --poll-config '{...}'; poll config controls interval, extraction, checkpoint strategy, and optional item-derived request cursors.".to_string(),
                "Subscriptions are durable by default. Use --ephemeral when the job should not auto-resume after daemon restart.".to_string(),
            ],
            examples: vec![
                "uxc subscribe start https://example.com/stream --sink file:/tmp/events.ndjson"
                    .to_string(),
                "uxc subscribe start https://example.com/stream --ephemeral --sink file:/tmp/oneshot.ndjson".to_string(),
                "uxc subscribe start wss://ws.okx.com:8443/ws/v5/public --transport websocket --init-frame '{\"op\":\"subscribe\",\"args\":[{\"channel\":\"tickers\",\"instId\":\"BTC-USDT\"}]}' --sink file:/tmp/okx.ndjson".to_string(),
                "uxc subscribe start https://discord.com/api/v10 --transport discord-gateway --auth discord-bot '{\"intents\":37377}' --sink file:/tmp/discord.ndjson".to_string(),
                "uxc subscribe start https://slack.com/api --transport slack-socket-mode --auth slack-app --sink file:/tmp/slack.ndjson".to_string(),
                "uxc subscribe start https://open.feishu.cn/open-apis --transport feishu-long-connection --auth feishu-tenant --sink file:/tmp/feishu.ndjson".to_string(),
                "uxc subscribe start https://example.com/graphql subscription/messageAdded '{\"roomId\":\"abc\",\"_select\":\"id body\"}' --sink file:/tmp/graphql.ndjson".to_string(),
                "uxc subscribe start wss://example.com/ws eth_subscribe '{\"params\":[\"logs\",{\"address\":\"0xabc\"}]}' --sink file:/tmp/logs.ndjson".to_string(),
                "uxc subscribe start https://example.com/api get:/events --mode poll --poll-config '{\"interval_secs\":5,\"extract_items_pointer\":\"/items\",\"checkpoint_strategy\":{\"type\":\"item_key\",\"item_key_pointer\":\"/id\"}}' --sink file:/tmp/events.ndjson".to_string(),
                "uxc subscribe start https://api.telegram.org post:/getUpdates --mode poll --poll-config '{\"interval_secs\":2,\"extract_items_pointer\":\"/result\",\"request_cursor_arg\":\"offset\",\"cursor_from_item_pointer\":\"/update_id\",\"cursor_transform\":\"increment\",\"checkpoint_strategy\":{\"type\":\"item_key\",\"item_key_pointer\":\"/update_id\"}}' --sink file:/tmp/telegram.ndjson".to_string(),
                "uxc subscribe start https://example.com/mcp --resource-uri file:///tmp/log --read-resource --sink file:/tmp/mcp-http.ndjson".to_string(),
                "uxc subscribe start \"npx -y my-mcp-server\" --resource-uri file:///tmp/log --read-resource --sink file:/tmp/mcp.ndjson".to_string(),
            ],
        },
        ["subscribe", "list"] => HelpData {
            path: "uxc subscribe list".to_string(),
            about: "List background subscription jobs".to_string(),
            usage: "uxc subscribe list".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc subscribe list".to_string()],
        },
        ["subscribe", "status"] => HelpData {
            path: "uxc subscribe status".to_string(),
            about: "Show a background subscription job".to_string(),
            usage: "uxc subscribe status <job_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc subscribe status sub_123".to_string()],
        },
        ["subscribe", "stop"] => HelpData {
            path: "uxc subscribe stop".to_string(),
            about: "Stop a background subscription job".to_string(),
            usage: "uxc subscribe stop <job_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc subscribe stop sub_123".to_string()],
        },
        ["cache"] => HelpData {
            path: "uxc cache".to_string(),
            about: "Manage schema cache".to_string(),
            usage: "uxc cache <list|stats|clear>".to_string(),
            commands: commands(&[
                ("list", "List cache entries"),
                ("stats", "Show cache statistics"),
                ("clear", "Clear cache entries"),
            ]),
            notes: vec![],
            examples: vec![
                "uxc cache list".to_string(),
                "uxc cache stats".to_string(),
                "uxc cache clear --all".to_string(),
            ],
        },
        ["cache", "list"] => HelpData {
            path: "uxc cache list".to_string(),
            about: "List cache entries".to_string(),
            usage: "uxc cache list".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc cache list".to_string()],
        },
        ["cache", "stats"] => HelpData {
            path: "uxc cache stats".to_string(),
            about: "Show cache statistics".to_string(),
            usage: "uxc cache stats".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc cache stats".to_string()],
        },
        ["cache", "clear"] => HelpData {
            path: "uxc cache clear".to_string(),
            about: "Clear cache entries".to_string(),
            usage: "uxc cache clear <url> | uxc cache clear --key <cache_key> | uxc cache clear --all".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec![
                "uxc cache clear https://petstore3.swagger.io/api/v3".to_string(),
                "uxc cache clear --key 767d50e00f278ca8".to_string(),
                "uxc cache clear --all".to_string(),
            ],
        },
        ["auth"] => HelpData {
            path: "uxc auth".to_string(),
            about: "Manage authentication credentials and bindings".to_string(),
            usage: "uxc auth <credential|info|binding|bootstrap|oauth> ...".to_string(),
            commands: commands(&[
                ("credential", "Manage credentials"),
                ("info", "Alias for auth credential info"),
                ("binding", "Manage endpoint auth bindings"),
                ("bootstrap", "Manage app-credential token bootstrap"),
                ("oauth", "Manage OAuth credentials"),
            ]),
            notes: vec![],
            examples: vec![
                "uxc auth credential list".to_string(),
                "uxc auth info deepwiki".to_string(),
                "uxc auth binding list".to_string(),
                "uxc auth bootstrap info feishu-tenant".to_string(),
            ],
        },
        ["auth", "info"] => HelpData {
            path: "uxc auth info".to_string(),
            about: "Alias for auth credential info".to_string(),
            usage: "uxc auth info <credential_id>".to_string(),
            commands: vec![],
            notes: vec!["Equivalent to: uxc auth credential info <credential_id>".to_string()],
            examples: vec!["uxc auth info deepwiki".to_string()],
        },
        ["auth", "credential"] => HelpData {
            path: "uxc auth credential".to_string(),
            about: "Manage credentials".to_string(),
            usage: "uxc auth credential <list|info|set|remove> ...".to_string(),
            commands: commands(&[
                ("list", "List all credentials"),
                ("info", "Show information about a specific credential"),
                ("set", "Set or update a credential"),
                ("remove", "Remove a credential"),
            ]),
            notes: vec![],
            examples: vec![
                "uxc auth credential list".to_string(),
                "uxc auth credential set demo --secret-env DEMO_TOKEN".to_string(),
                "uxc auth credential set demo --secret-op op://Vault/Item/token".to_string(),
                "uxc auth credential set binance --auth-type api_key --field api_key=env:BINANCE_API_KEY --field secret_key=env:BINANCE_SECRET_KEY".to_string(),
            ],
        },
        ["auth", "credential", "list"] => HelpData {
            path: "uxc auth credential list".to_string(),
            about: "List all credentials".to_string(),
            usage: "uxc auth credential list".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth credential list".to_string()],
        },
        ["auth", "credential", "info"] => HelpData {
            path: "uxc auth credential info".to_string(),
            about: "Show information about a specific credential".to_string(),
            usage: "uxc auth credential info <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth credential info deepwiki".to_string()],
        },
        ["auth", "credential", "set"] => HelpData {
            path: "uxc auth credential set".to_string(),
            about: "Set or update a credential".to_string(),
            usage: "uxc auth credential set <credential_id> [--auth-type <type>] [--secret <value>|--secret-env <key>|--secret-op <op://...>] [--field <name>=<literal:...|env:...|op://...>]... [--api-key-header <name>|--header <name>=<template>] [--query-param <name>=<template>] [--path-prefix-template <template>] [--description <text>]".to_string(),
            commands: vec![],
            notes: vec![
                "--field is repeatable and stores additional named auth fields on the credential.".to_string(),
                "{{secret}} remains available and is equivalent to {{field:secret}} for compatible credentials.".to_string(),
                "--path-prefix-template is for APIs that place credentials in the request path, such as Telegram Bot API.".to_string(),
            ],
            examples: vec![
                "uxc auth credential set deepwiki --secret-env DEEPWIKI_TOKEN".to_string(),
                "uxc auth credential set deepwiki --secret-op op://Engineering/deepwiki/token"
                    .to_string(),
                "uxc auth credential set flipside --auth-type api_key --query-param \"apiKey={{secret}}\" --secret-env FLIPSIDE_API_KEY".to_string(),
                "uxc auth credential set binance --auth-type api_key --field api_key=env:BINANCE_API_KEY --field secret_key=env:BINANCE_SECRET_KEY --header \"X-API-Key={{field:api_key}}\"".to_string(),
                "uxc auth credential set telegram-bot --auth-type api_key --secret-env TELEGRAM_BOT_TOKEN --path-prefix-template \"/bot{{secret}}\"".to_string(),
            ],
        },
        ["auth", "credential", "remove"] => HelpData {
            path: "uxc auth credential remove".to_string(),
            about: "Remove a credential".to_string(),
            usage: "uxc auth credential remove <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth credential remove deepwiki".to_string()],
        },
        ["auth", "binding"] => HelpData {
            path: "uxc auth binding".to_string(),
            about: "Manage endpoint auth bindings".to_string(),
            usage: "uxc auth binding <list|add|remove|match> ...".to_string(),
            commands: commands(&[
                ("list", "List all endpoint auth bindings"),
                ("add", "Add a binding rule"),
                ("remove", "Remove a binding rule"),
                ("match", "Match endpoint against bindings"),
            ]),
            notes: vec![],
            examples: vec![
                "uxc auth binding list".to_string(),
                "uxc auth binding match https://mcp.deepwiki.com/mcp".to_string(),
            ],
        },
        ["auth", "binding", "list"] => HelpData {
            path: "uxc auth binding list".to_string(),
            about: "List all endpoint auth bindings".to_string(),
            usage: "uxc auth binding list".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth binding list".to_string()],
        },
        ["auth", "binding", "add"] => HelpData {
            path: "uxc auth binding add".to_string(),
            about: "Add a binding rule".to_string(),
            usage: "uxc auth binding add --id <id> --host <host> --credential <credential> [--path-prefix <path>] [--scheme <scheme>] [--signer-json <json>] [--priority <n>] [--disabled]".to_string(),
            commands: vec![],
            notes: vec![
                "--signer-json attaches a typed signer config to this binding, e.g. {\"kind\":\"hmac_query_v1\", ...}, {\"kind\":\"ed25519_query_v1\", ...}, or {\"kind\":\"jwt_bearer_v1\", ...}.".to_string(),
            ],
            examples: vec![
                "uxc auth binding add --id deepwiki-mcp --host mcp.deepwiki.com --path-prefix /mcp --scheme https --credential deepwiki --priority 100".to_string(),
                "uxc auth binding add --id binance-account --host api.binance.com --path-prefix /api/v3 --scheme https --credential binance --signer-json '{\"kind\":\"hmac_query_v1\",\"algorithm\":\"hmac_sha256\",\"signing_field\":\"secret_key\",\"key_field\":\"api_key\",\"key_placement\":\"header\",\"key_name\":\"X-MBX-APIKEY\",\"signature_param\":\"signature\",\"signature_encoding\":\"hex\",\"timestamp_param\":\"timestamp\",\"timestamp_unit\":\"milliseconds\",\"canonicalization\":{\"mode\":\"preserve_order\"}}'".to_string(),
                "uxc auth binding add --id binance-account-ed25519 --host api.binance.com --path-prefix /api/v3 --scheme https --credential binance-ed25519 --signer-json '{\"kind\":\"ed25519_query_v1\",\"algorithm\":\"ed25519\",\"signing_field\":\"private_key\",\"key_field\":\"api_key\",\"key_placement\":\"header\",\"key_name\":\"X-MBX-APIKEY\",\"signature_param\":\"signature\",\"signature_encoding\":\"base64\",\"timestamp_param\":\"timestamp\",\"timestamp_unit\":\"milliseconds\",\"canonicalization\":{\"mode\":\"preserve_order\"}}'".to_string(),
                "uxc auth binding add --id coinbase-advanced-trade --host api.coinbase.com --path-prefix /api/v3/brokerage --scheme https --credential coinbase-advanced-trade --signer-json '{\"kind\":\"jwt_bearer_v1\",\"algorithm\":\"es256\",\"private_key_field\":\"private_key\",\"header_typ\":\"JWT\",\"header_kid_field\":\"key_id\",\"expires_in_seconds\":120,\"claims\":{\"static\":{\"iss\":\"cdp\"},\"from_fields\":{\"sub\":\"key_id\"},\"time\":{\"nbf\":\"now\",\"exp\":\"now_plus_ttl\"}},\"request_claim\":{\"name\":\"uri\",\"format\":\"string\",\"value_template\":\"{{request.method}} {{request.host}}{{request.path}}\"}}'".to_string(),
            ],
        },
        ["auth", "binding", "remove"] => HelpData {
            path: "uxc auth binding remove".to_string(),
            about: "Remove a binding rule".to_string(),
            usage: "uxc auth binding remove <binding_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth binding remove deepwiki-mcp".to_string()],
        },
        ["auth", "binding", "match"] => HelpData {
            path: "uxc auth binding match".to_string(),
            about: "Match endpoint against bindings".to_string(),
            usage: "uxc auth binding match <endpoint>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth binding match https://mcp.deepwiki.com/mcp".to_string()],
        },
        ["auth", "bootstrap"] => HelpData {
            path: "uxc auth bootstrap".to_string(),
            about: "Manage app-credential token bootstrap".to_string(),
            usage: "uxc auth bootstrap <set|info|refresh|remove> ...".to_string(),
            commands: commands(&[
                ("set", "Configure token bootstrap for a credential"),
                ("info", "Show token bootstrap configuration and state"),
                ("refresh", "Force refresh a bootstrap-backed token"),
                ("remove", "Remove token bootstrap configuration and state"),
            ]),
            notes: vec![
                "Use this for providers that exchange named credential fields such as app_id/app_secret for a short-lived bearer token.".to_string(),
            ],
            examples: vec![
                "uxc auth bootstrap info feishu-tenant".to_string(),
                "uxc auth bootstrap refresh feishu-tenant".to_string(),
            ],
        },
        ["auth", "bootstrap", "set"] => HelpData {
            path: "uxc auth bootstrap set".to_string(),
            about: "Configure token bootstrap for a credential".to_string(),
            usage: "uxc auth bootstrap set <credential_id> --token-endpoint <url> --request-json <json-template> --access-token-pointer <pointer> --expires-in-pointer <pointer> [--header <name>=<template>]... [--token-type-pointer <pointer>] [--success-code-pointer <pointer> --success-code-value <json-literal>] [--refresh-skew-seconds <n>]".to_string(),
            commands: vec![],
            notes: vec![
                "The credential must already exist and should normally use --auth-type bearer.".to_string(),
                "--request-json and --header templates support {{secret}}, {{field:name}}, {{env:VAR}}, and {{op://...}}.".to_string(),
            ],
            examples: vec![
                "uxc auth bootstrap set feishu-tenant --token-endpoint https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal --header 'Content-Type=application/json; charset=utf-8' --request-json '{\"app_id\":\"{{field:app_id}}\",\"app_secret\":\"{{field:app_secret}}\"}' --access-token-pointer /tenant_access_token --expires-in-pointer /expire --success-code-pointer /code --success-code-value 0".to_string(),
            ],
        },
        ["auth", "bootstrap", "info"] => HelpData {
            path: "uxc auth bootstrap info".to_string(),
            about: "Show token bootstrap configuration and state".to_string(),
            usage: "uxc auth bootstrap info <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth bootstrap info feishu-tenant".to_string()],
        },
        ["auth", "bootstrap", "refresh"] => HelpData {
            path: "uxc auth bootstrap refresh".to_string(),
            about: "Force refresh a bootstrap-backed token".to_string(),
            usage: "uxc auth bootstrap refresh <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth bootstrap refresh feishu-tenant".to_string()],
        },
        ["auth", "bootstrap", "remove"] => HelpData {
            path: "uxc auth bootstrap remove".to_string(),
            about: "Remove token bootstrap configuration and state".to_string(),
            usage: "uxc auth bootstrap remove <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth bootstrap remove feishu-tenant".to_string()],
        },
        ["auth", "oauth"] => HelpData {
            path: "uxc auth oauth".to_string(),
            about: "Manage OAuth credentials".to_string(),
            usage: "uxc auth oauth <list|start|complete|login|refresh|info|logout> ...".to_string(),
            commands: commands(&[
                ("list", "List OAuth credentials"),
                ("start", "Start non-interactive OAuth authorization_code login"),
                ("complete", "Complete non-interactive OAuth authorization_code login"),
                ("login", "Login with OAuth and save tokens"),
                ("refresh", "Refresh OAuth token"),
                ("info", "Show OAuth credential information"),
                ("logout", "Remove OAuth token data from credential"),
            ]),
            notes: vec![],
            examples: vec![
                "uxc auth oauth list".to_string(),
                "uxc auth oauth info deepwiki".to_string(),
                "uxc auth oauth refresh deepwiki".to_string(),
            ],
        },
        ["auth", "oauth", "list"] => HelpData {
            path: "uxc auth oauth list".to_string(),
            about: "List OAuth credentials".to_string(),
            usage: "uxc auth oauth list".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth oauth list".to_string()],
        },
        ["auth", "oauth", "start"] => HelpData {
            path: "uxc auth oauth start".to_string(),
            about: "Start non-interactive OAuth authorization_code login".to_string(),
            usage: "uxc auth oauth start <credential_id> --endpoint <url> --redirect-uri <uri> [--scope <scope>] [--client-id <id>] [--client-secret <secret>] [--issuer <url>] [--authorization-endpoint <url>] [--token-endpoint <url>] [--registration-endpoint <url>] [--resource-metadata-url <url>]".to_string(),
            commands: vec![],
            notes: vec!["Returns an authorization URL and session ID for agent-driven completion.".to_string()],
            examples: vec!["uxc auth oauth start notion --endpoint https://mcp.notion.com/mcp --redirect-uri http://127.0.0.1:11111/callback --client-id <id> --scope read".to_string()],
        },
        ["auth", "oauth", "complete"] => HelpData {
            path: "uxc auth oauth complete".to_string(),
            about: "Complete non-interactive OAuth authorization_code login".to_string(),
            usage: "uxc auth oauth complete <credential_id> --session-id <id> --authorization-response <callback_url_or_code>".to_string(),
            commands: vec![],
            notes: vec!["The authorization response can be a callback URL, query string, or plain authorization code.".to_string()],
            examples: vec!["uxc auth oauth complete notion --session-id abc123 --authorization-response 'http://127.0.0.1:11111/callback?code=...'".to_string()],
        },
        ["auth", "oauth", "login"] => HelpData {
            path: "uxc auth oauth login".to_string(),
            about: "Login with OAuth and save tokens".to_string(),
            usage: "uxc auth oauth login <credential_id> --endpoint <url> [--flow <device_code|authorization_code|client_credentials>] [--scope <scope>] [--client-id <id>] [--client-secret <secret>] [--redirect-uri <uri>] [--authorization-code <code>] [--issuer <url>] [--authorization-endpoint <url>] [--token-endpoint <url>] [--device-authorization-endpoint <url>] [--registration-endpoint <url>] [--resource-metadata-url <url>]".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth oauth login deepwiki --endpoint https://mcp.deepwiki.com/mcp --flow device_code --client-id <id>".to_string()],
        },
        ["auth", "oauth", "refresh"] => HelpData {
            path: "uxc auth oauth refresh".to_string(),
            about: "Refresh OAuth token".to_string(),
            usage: "uxc auth oauth refresh <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth oauth refresh deepwiki".to_string()],
        },
        ["auth", "oauth", "info"] => HelpData {
            path: "uxc auth oauth info".to_string(),
            about: "Show OAuth credential information".to_string(),
            usage: "uxc auth oauth info <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth oauth info deepwiki".to_string()],
        },
        ["auth", "oauth", "logout"] => HelpData {
            path: "uxc auth oauth logout".to_string(),
            about: "Remove OAuth token data from credential".to_string(),
            usage: "uxc auth oauth logout <credential_id>".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc auth oauth logout deepwiki".to_string()],
        },
        _ => HelpData {
            path: "uxc".to_string(),
            about: "Universal X-Protocol CLI".to_string(),
            usage: "uxc [OPTIONS] [URL] [COMMAND]".to_string(),
            commands: vec![],
            notes: vec![],
            examples: vec!["uxc help".to_string()],
        },
    }
}

fn render_output(envelope: &OutputEnvelope, output_mode: OutputMode) -> Result<()> {
    match output_mode {
        OutputMode::Json => print_json(envelope),
        OutputMode::Text => render_text_output(envelope),
    }
}

fn render_text_output(envelope: &OutputEnvelope) -> Result<()> {
    if !envelope.ok {
        if let Some(err) = &envelope.error {
            println!("{}", err.message);
        }
        return Ok(());
    }

    match envelope.kind.as_deref() {
        Some("global_help") | Some("subcommand_help") => {
            let data: HelpData = decode_envelope_data(envelope)?;
            print_help_text(&data);
            Ok(())
        }
        Some("host_help") => {
            let endpoint = envelope.endpoint.as_deref().unwrap_or("unknown");
            let protocol = envelope.protocol.as_deref().unwrap_or("unknown");
            let data: HostHelpData = decode_envelope_data(envelope)?;
            print_host_help_text_from_summaries(
                protocol,
                endpoint,
                &data.operations,
                &data.examples,
                &data.service,
            );
            Ok(())
        }
        Some("operation_detail") => {
            let endpoint = envelope.endpoint.as_deref().unwrap_or("unknown");
            let protocol = envelope.protocol.as_deref().unwrap_or("unknown");
            let detail: OperationDetail = decode_envelope_data(envelope)?;
            let invocation_examples = envelope
                .data
                .as_ref()
                .and_then(|value| value.get("invocation_examples"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            print_detail_text(protocol, endpoint, &detail, &invocation_examples);
            Ok(())
        }
        Some("inspect_result") => {
            let protocol = envelope.protocol.as_deref().unwrap_or("unknown");
            let endpoint = envelope.endpoint.as_deref().unwrap_or("unknown");
            let data = envelope.data.clone().unwrap_or(Value::Null);
            println!("Protocol: {}", protocol);
            println!("Endpoint: {}", endpoint);
            if let Some(schema) = data.get("schema").filter(|v| !v.is_null()) {
                println!("\nSchema:\n{}", serde_json::to_string_pretty(schema)?);
            }
            Ok(())
        }
        Some("call_result") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&envelope.data.clone().unwrap_or(Value::Null))?
            );
            Ok(())
        }
        Some("cache_stats") => {
            let stats: cache::CacheStats = decode_envelope_data(envelope)?;
            println!("{}", stats.display());
            Ok(())
        }
        Some("cache_list") => {
            let data: CacheListData = decode_envelope_data(envelope)?;
            if data.entries.is_empty() {
                println!("No cache entries.");
                return Ok(());
            }
            for entry in data.entries {
                println!(
                    "{} [{}] stale={} url={}",
                    entry.key, entry.protocol, entry.stale, entry.url
                );
            }
            Ok(())
        }
        Some("cache_clear_result") => {
            let data: CacheClearData = decode_envelope_data(envelope)?;
            if data.scope == "all" {
                println!("Cache cleared successfully.");
            } else if data.scope == "key" {
                if let Some(key) = data.key {
                    println!("Cache entry cleared for key: {}", key);
                } else {
                    println!("Cache cleared.");
                }
            } else if let Some(url) = data.url {
                println!("Cache entry cleared for: {}", url);
            } else {
                println!("Cache cleared.");
            }
            Ok(())
        }
        Some("daemon_start_result") => {
            let data = envelope.data.clone().unwrap_or(Value::Null);
            if data
                .get("restarted_for_version_mismatch")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                println!("Daemon restarted due to version mismatch.");
                if let Some(previous_version) = data.get("previous_version").and_then(Value::as_str)
                {
                    println!("Previous Version: {}", previous_version);
                }
                if let Some(version) = data.get("version").and_then(Value::as_str) {
                    println!("Version: {}", version);
                }
            } else if data
                .get("autostarted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                println!("Daemon started.");
            } else {
                println!("Daemon already running.");
            }
            if let Some(socket) = data.get("socket").and_then(Value::as_str) {
                println!("Socket: {}", socket);
            }
            Ok(())
        }
        Some("daemon_stop_result") => {
            let data = envelope.data.clone().unwrap_or(Value::Null);
            if data
                .get("stopped")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                println!("Daemon stopped.");
            } else {
                println!("Daemon is not running.");
            }
            Ok(())
        }
        Some("daemon_status") => {
            let data = envelope.data.clone().unwrap_or(Value::Null);
            let running = data
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            println!("Running: {}", running);
            if !running {
                if let Some(err) = data.get("error") {
                    if let Some(message) = err.get("message").and_then(Value::as_str) {
                        println!("Error: {}", message);
                    }
                }
            }
            if let Some(pid) = data.get("pid").and_then(Value::as_u64) {
                println!("PID: {}", pid);
            }
            if let Some(socket) = data.get("socket").and_then(Value::as_str) {
                println!("Socket: {}", socket);
            }
            if let Some(version) = data.get("version").and_then(Value::as_str) {
                println!("Daemon Version: {}", version);
            }
            if let Some(version) = data.get("client_version").and_then(Value::as_str) {
                println!("CLI Version: {}", version);
            }
            if let Some(mismatch) = data.get("version_mismatch").and_then(Value::as_bool) {
                println!("Version Mismatch: {}", mismatch);
            }
            if let Some(requests) = data.get("request_count").and_then(Value::as_u64) {
                println!("Requests: {}", requests);
            }
            Ok(())
        }
        Some("daemon_restart_result") => {
            let data = envelope.data.clone().unwrap_or(Value::Null);
            if data
                .get("stopped")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                println!("Daemon stopped.");
            } else {
                println!("Daemon was not running.");
            }
            println!("Daemon started.");
            if let Some(socket) = data.get("socket").and_then(Value::as_str) {
                println!("Socket: {}", socket);
            }
            Ok(())
        }
        Some("subscribe_start_result") => {
            let data = envelope.data.clone().unwrap_or(Value::Null);
            if let Some(job_id) = data.get("job_id").and_then(Value::as_str) {
                println!("Job ID: {}", job_id);
            }
            if let Some(status) = data.get("status").and_then(Value::as_str) {
                println!("Status: {}", status);
            }
            if let Some(protocol) = data.get("protocol").and_then(Value::as_str) {
                println!("Protocol: {}", protocol);
            }
            if let Some(sink) = data.get("sink").and_then(Value::as_str) {
                println!("Sink: {}", sink);
            }
            Ok(())
        }
        Some("subscribe_list") => {
            let data: SubscribeListData = decode_envelope_data(envelope)?;
            if data.jobs.is_empty() {
                println!("No subscription jobs.");
                return Ok(());
            }
            for job in data.jobs {
                println!(
                    "{} [{}] {} -> {}",
                    job.job_id, job.protocol, job.status, job.sink
                );
            }
            Ok(())
        }
        Some("subscribe_status") => {
            let data: daemon::SubscriptionJobView = decode_envelope_data(envelope)?;
            println!("Job ID: {}", data.job_id);
            println!("Status: {}", data.status);
            println!("Protocol: {}", data.protocol);
            println!("Endpoint: {}", data.endpoint);
            println!("Sink: {}", data.sink);
            if let Some(resource_uri) = data.resource_uri {
                println!("Resource URI: {}", resource_uri);
            }
            if let Some(last_error) = data.last_error {
                println!("Last Error: {}", last_error);
            }
            Ok(())
        }
        Some("subscribe_stop_result") => {
            let data = envelope.data.clone().unwrap_or(Value::Null);
            if data
                .get("stopped")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                println!("Subscription stopped.");
            } else {
                println!("Subscription was not running.");
            }
            Ok(())
        }
        Some("auth_list") => {
            let data: AuthListData = decode_envelope_data(envelope)?;
            if data.credentials.is_empty() {
                println!("No credentials found.");
                println!("\nCreate one with: uxc auth credential set <id> --secret <value>");
                println!(
                    "Or add named fields with: uxc auth credential set <id> --field api_key=env:API_KEY --field secret_key=env:SECRET_KEY"
                );
                return Ok(());
            }

            println!("Credentials:\n");
            for credential in data.credentials {
                println!("  {}", credential.name);
                println!("    Type: {}", credential.auth_type);
                println!("    Secret: {}", credential.api_key_masked);
                if let Some(source) = credential.secret_source {
                    println!("    Source: {}", source.kind);
                }
                if let Some(fields) = credential.fields {
                    let names = fields
                        .into_iter()
                        .map(|field| field.name)
                        .collect::<Vec<_>>();
                    if !names.is_empty() {
                        println!("    Fields: {}", names.join(", "));
                    }
                }
                if let Some(headers) = credential.auth_headers {
                    let names = headers.into_iter().map(|h| h.name).collect::<Vec<_>>();
                    if !names.is_empty() {
                        println!("    Auth Headers: {}", names.join(", "));
                    }
                }
                if let Some(params) = credential.auth_query_params {
                    let names = params.into_iter().map(|p| p.name).collect::<Vec<_>>();
                    if !names.is_empty() {
                        println!("    Auth Query Params: {}", names.join(", "));
                    }
                }
                if let Some(oauth) = credential.oauth {
                    println!(
                        "    OAuth Flow: {}",
                        oauth.flow.unwrap_or_else(|| "unknown".to_string())
                    );
                    if let Some(issuer) = oauth.provider_issuer {
                        println!("    OAuth Issuer: {}", issuer);
                    }
                }
                if let Some(desc) = credential.description {
                    println!("    Description: {}", desc);
                }
                println!();
            }
            Ok(())
        }
        Some("auth_info") | Some("auth_set_result") => {
            let credential: AuthProfileView = decode_envelope_data(envelope)?;
            println!("Credential: {}", credential.name);
            println!("  Type: {}", credential.auth_type);
            println!("  Secret: {}", credential.api_key_masked);
            if let Some(source) = credential.secret_source {
                println!("  Source: {}", source.kind);
            }
            if let Some(fields) = credential.fields {
                let names = fields
                    .into_iter()
                    .map(|field| field.name)
                    .collect::<Vec<_>>();
                if !names.is_empty() {
                    println!("  Fields: {}", names.join(", "));
                }
            }
            if let Some(headers) = credential.auth_headers {
                let names = headers.into_iter().map(|h| h.name).collect::<Vec<_>>();
                if !names.is_empty() {
                    println!("  Auth Headers: {}", names.join(", "));
                }
            }
            if let Some(params) = credential.auth_query_params {
                let names = params.into_iter().map(|p| p.name).collect::<Vec<_>>();
                if !names.is_empty() {
                    println!("  Auth Query Params: {}", names.join(", "));
                }
            }
            if let Some(oauth) = credential.oauth {
                println!(
                    "  OAuth Flow: {}",
                    oauth.flow.unwrap_or_else(|| "unknown".to_string())
                );
                if let Some(issuer) = oauth.provider_issuer {
                    println!("  OAuth Issuer: {}", issuer);
                }
                if !oauth.scopes.is_empty() {
                    println!("  OAuth Scopes: {}", oauth.scopes.join(", "));
                }
                if let Some(expires_at) = oauth.expires_at {
                    println!("  OAuth Expires At: {}", expires_at);
                }
                println!(
                    "  OAuth Refresh Token: {}",
                    if oauth.has_refresh_token {
                        "available"
                    } else {
                        "none"
                    }
                );
            }
            if let Some(desc) = credential.description {
                println!("  Description: {}", desc);
            }
            Ok(())
        }
        Some("auth_oauth_start_result") => {
            let data: AuthOAuthStartData = decode_envelope_data(envelope)?;
            println!("Open this URL to authorize:");
            println!("{}", data.authorization_url);
            println!();
            println!("Session ID: {}", data.session_id);
            println!("Expires At: {}", data.expires_at);
            println!();
            println!(
                "Complete with:\nuxc auth oauth complete {} --session-id {} --authorization-response '<callback-url>'",
                data.credential, data.session_id
            );
            Ok(())
        }
        Some("auth_remove_result") => {
            let data: AuthRemoveData = decode_envelope_data(envelope)?;
            println!("Credential '{}' removed successfully.", data.credential);
            Ok(())
        }
        Some("auth_binding_list") => {
            let data: AuthBindingListData = decode_envelope_data(envelope)?;
            if data.bindings.is_empty() {
                println!("No auth bindings found.");
                return Ok(());
            }
            for binding in data.bindings {
                println!(
                    "{} -> {} (host={}, path_prefix={}, scheme={}, priority={}, enabled={})",
                    binding.id,
                    binding.credential,
                    binding.host,
                    binding.path_prefix.unwrap_or_else(|| "/".to_string()),
                    binding.scheme.unwrap_or_else(|| "*".to_string()),
                    binding.priority,
                    binding.enabled
                );
            }
            Ok(())
        }
        Some("auth_binding_match") => {
            let data: AuthBindingMatchData = decode_envelope_data(envelope)?;
            if let Some(binding) = data.binding {
                println!(
                    "Matched '{}' for {} -> credential '{}'",
                    binding.id, data.endpoint, binding.credential
                );
            } else {
                println!("No binding matched {}", data.endpoint);
            }
            Ok(())
        }
        Some("auth_binding_set_result") => {
            let data: AuthBindingSetData = decode_envelope_data(envelope)?;
            println!(
                "Created binding '{}' -> credential '{}' (host={}, path_prefix={}, scheme={}, priority={}, enabled={}).",
                data.id,
                data.credential,
                data.host,
                data.path_prefix.unwrap_or_else(|| "/".to_string()),
                data.scheme.unwrap_or_else(|| "*".to_string()),
                data.priority,
                data.enabled
            );
            Ok(())
        }
        Some("auth_binding_remove_result") => {
            let data: AuthBindingRemoveData = decode_envelope_data(envelope)?;
            println!("Removed binding '{}'.", data.binding_id);
            Ok(())
        }
        Some("link_create_result") => {
            let data: LinkCreateData = decode_envelope_data(envelope)?;
            if data.overwritten {
                println!("Updated shortcut '{}' -> {}", data.name, data.host);
            } else {
                println!("Created shortcut '{}' -> {}", data.name, data.host);
            }
            println!("Path: {}", data.path);
            if let Some(schema_url) = data.schema_url {
                println!("Schema URL: {}", schema_url);
            }
            if let Some(credential) = data.credential {
                println!("Credential: {}", credential);
            }
            if !data.inject_env.is_empty() {
                println!("Injected Env: {}", data.inject_env.join(", "));
            }
            if !data.dir_in_path {
                println!(
                    "Note: shortcut directory is not in PATH. Add it before invoking '{}'.",
                    data.name
                );
            }
            Ok(())
        }
        _ => {
            if let Some(data) = &envelope.data {
                println!("{}", serde_json::to_string_pretty(data)?);
            }
            Ok(())
        }
    }
}

fn decode_envelope_data<T: DeserializeOwned>(envelope: &OutputEnvelope) -> Result<T> {
    let value = envelope
        .data
        .as_ref()
        .ok_or_else(|| UxcError::GenericError(anyhow::anyhow!("Envelope data is missing")))?;
    Ok(T::deserialize(value)?)
}

fn resolve_endpoint_command(cli: &Cli) -> Result<EndpointCommand> {
    match &cli.command {
        None => Ok(EndpointCommand::HostHelp),
        Some(Commands::External(tokens)) => parse_external_command(tokens, cli.help),
        Some(Commands::Cache { .. })
        | Some(Commands::Auth { .. })
        | Some(Commands::Link { .. })
        | Some(Commands::Daemon { .. })
        | Some(Commands::Subscribe { .. }) => Err(UxcError::InvalidArguments(
            "Internal routing error for management command".to_string(),
        )
        .into()),
    }
}

/// Build a helpful error message for invalid operation arguments
fn build_invalid_arg_error(arg: &str, operation_id: &str) -> String {
    format!(
        "Unknown argument '{}' for operation '{}'.\n\nHint: Use key=value for scalar fields:\n  uxc <host> {} key1=value1 key2=value2\n\nUse path-style keys for nested fields:\n  uxc <host> {} filter.status=active items[0].id=1\n\nUse := for per-field JSON values:\n  uxc <host> {} filter:='{{\"status\":\"active\"}}' tags:='[\"rust\",\"cli\"]'\n\nOr pass one full JSON object:\n  uxc <host> {} '{{\"key1\":\"value1\",\"key2\":\"value2\"}}'",
        arg, operation_id, operation_id, operation_id, operation_id, operation_id
    )
}

fn parse_external_command(tokens: &[String], global_help: bool) -> Result<EndpointCommand> {
    if tokens.is_empty() {
        return Err(UxcError::InvalidArguments("Operation ID is required".to_string()).into());
    }

    let operation_id = tokens[0].clone();

    if global_help {
        return Ok(EndpointCommand::Describe { operation_id });
    }

    let mut args = Vec::new();
    let mut input_json = None;
    let mut positional = Vec::new();
    let mut idx = 1;

    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "-h" | "--help" => {
                return Ok(EndpointCommand::Describe { operation_id });
            }
            "-a" | "--args" => {
                idx += 1;
                let arg = tokens.get(idx).ok_or_else(|| {
                    UxcError::InvalidArguments("Missing value for --args".to_string())
                })?;
                args.push(arg.clone());
            }
            "--input-json" => {
                idx += 1;
                let payload = tokens.get(idx).ok_or_else(|| {
                    UxcError::InvalidArguments("Missing value for --input-json".to_string())
                })?;
                input_json = Some(payload.clone());
            }
            token if token.contains('=') && !token.starts_with('-') => {
                args.push(token.to_string());
            }
            token if !token.starts_with('-') => {
                positional.push(token.to_string());
            }
            unknown => {
                return Err(UxcError::InvalidArguments(build_invalid_arg_error(
                    unknown,
                    &operation_id,
                ))
                .into());
            }
        }

        idx += 1;
    }

    let (args, input_json) =
        normalize_operation_inputs(&operation_id, args, input_json, &positional)?;

    Ok(EndpointCommand::Execute {
        operation_id,
        args,
        input_json,
    })
}

fn normalize_operation_inputs(
    operation_id: &str,
    mut args: Vec<String>,
    explicit_input_json: Option<String>,
    positional: &[String],
) -> Result<(Vec<String>, Option<String>)> {
    let mut bare_json_payload = None;

    for token in positional {
        if token.contains('=') && !token.starts_with('-') {
            args.push(token.clone());
            continue;
        }

        if token.starts_with('-') {
            return Err(
                UxcError::InvalidArguments(build_invalid_arg_error(token, operation_id)).into(),
            );
        }

        if bare_json_payload.is_some() {
            return Err(UxcError::InvalidArguments(format!(
                "Unexpected argument '{}' for operation '{}'",
                token, operation_id
            ))
            .into());
        }

        let parsed = serde_json::from_str::<Value>(token).map_err(|_| {
            UxcError::InvalidArguments(build_invalid_arg_error(token, operation_id))
        })?;

        if !parsed.is_object() {
            return Err(UxcError::InvalidArguments(format!(
                "Positional JSON payload for operation '{}' must be an object",
                operation_id
            ))
            .into());
        }

        bare_json_payload = Some(token.clone());
    }

    if explicit_input_json.is_some() && bare_json_payload.is_some() {
        return Err(UxcError::InvalidArguments(
            "Cannot provide both --input-json and positional JSON payload. Choose one input style: keep --input-json <json> and remove the positional JSON, or remove --input-json and keep exactly one positional JSON object."
                .to_string(),
        )
        .into());
    }

    for arg in &args {
        if arg.contains('=') {
            continue;
        }

        if serde_json::from_str::<Value>(arg).is_ok() {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid --args value '{}' for operation '{}'. Use key=value for --args, or pass a JSON object as a positional argument",
                arg, operation_id
            ))
            .into());
        }

        return Err(UxcError::InvalidArguments(format!(
            "Invalid --args value '{}' for operation '{}'. Expected key=value",
            arg, operation_id
        ))
        .into());
    }

    Ok((args, explicit_input_json.or(bare_json_payload)))
}

fn parse_arguments(
    args: Vec<String>,
    input_json: Option<String>,
) -> Result<HashMap<String, Value>> {
    crate::cli::ArgumentParser::parse_arguments(args, input_json)
}

fn enrich_operation_detail_payload(data: Value, command_head: &str, operation_id: &str) -> Value {
    let mut data = data;
    let Some(object) = data.as_object_mut() else {
        return data;
    };
    if object.contains_key("invocation_examples") {
        return data;
    }

    let examples = build_operation_invocation_examples(
        &Value::Object(object.clone()),
        command_head,
        operation_id,
    );
    if !examples.is_empty() {
        object.insert(
            "invocation_examples".to_string(),
            Value::Array(examples.into_iter().map(Value::String).collect()),
        );
    }
    data
}

fn build_operation_invocation_examples(
    detail_data: &Value,
    command_head: &str,
    operation_id: &str,
) -> Vec<String> {
    let mut snippets = Vec::new();
    let input_schema = detail_data.get("input_schema");

    if let Some(schema) = input_schema.and_then(select_operation_input_schema_for_examples) {
        if let Some((key, value)) = first_scalar_path_example(&schema) {
            push_unique(&mut snippets, format!("{}={}", key, value));
        }
        if let Some((key, value)) = first_nested_path_example(&schema) {
            push_unique(&mut snippets, format!("{}={}", key, value));
        }
        if let Some((key, value)) = first_array_path_example(&schema) {
            push_unique(&mut snippets, format!("{}={}", key, value));
        }
        if let Some((key, json_value)) = first_json_assignment_example(&schema) {
            let encoded = serde_json::to_string(&json_value).unwrap_or_else(|_| "{}".to_string());
            push_unique(&mut snippets, format!("{}:='{}'", key, encoded));
        }
    }

    if let Some(file_field) = input_schema.and_then(first_file_field_name) {
        push_unique(
            &mut snippets,
            format!("{}=@/abs/path/{}.bin", file_field, file_field),
        );
    }

    if snippets.is_empty() {
        if let Some(first_param) = detail_data
            .get("parameters")
            .and_then(Value::as_array)
            .and_then(|params| params.first())
            .and_then(|param| param.get("name"))
            .and_then(Value::as_str)
        {
            snippets.push(format!("{}=value", first_param));
        }
    }

    snippets
        .into_iter()
        .map(|snippet| format!("{command_head} {operation_id} {snippet}"))
        .collect()
}

fn select_operation_input_schema_for_examples(input_schema: &Value) -> Option<Value> {
    if input_schema.get("kind").and_then(Value::as_str) == Some("grpc_message") {
        return input_schema.get("schema").cloned();
    }

    if input_schema.get("kind").and_then(Value::as_str) == Some("openrpc_method") {
        let params = input_schema.get("params").and_then(Value::as_array)?;
        let mut properties = serde_json::Map::new();
        for param in params {
            let name = param.get("name").and_then(Value::as_str)?;
            let schema = param
                .get("schema")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            properties.insert(name.to_string(), schema);
        }
        return Some(json!({
            "type": "object",
            "properties": properties
        }));
    }

    if let Some(content) = input_schema.get("content").and_then(Value::as_object) {
        for media_type in [
            "application/json",
            "application/x-www-form-urlencoded",
            "multipart/form-data",
        ] {
            if let Some(schema) = content
                .get(media_type)
                .and_then(|entry| entry.get("schema"))
            {
                return Some(schema.clone());
            }
        }
    }

    if input_schema.get("type").is_some() {
        return Some(input_schema.clone());
    }

    input_schema.get("schema").cloned()
}

fn first_scalar_path_example(schema: &Value) -> Option<(String, String)> {
    let properties = schema.get("properties").and_then(Value::as_object)?;
    for (name, property_schema) in properties {
        if let Some(sample) = scalar_sample_value(property_schema) {
            return Some((name.clone(), sample));
        }
    }
    None
}

fn first_nested_path_example(schema: &Value) -> Option<(String, String)> {
    let properties = schema.get("properties").and_then(Value::as_object)?;
    for (name, property_schema) in properties {
        if schema_type(property_schema) != Some("object") {
            continue;
        }
        let nested_properties = property_schema
            .get("properties")
            .and_then(Value::as_object)?;
        for (child_name, child_schema) in nested_properties {
            if let Some(sample) = scalar_sample_value(child_schema) {
                return Some((format!("{name}.{child_name}"), sample));
            }
        }
    }
    None
}

fn first_array_path_example(schema: &Value) -> Option<(String, String)> {
    let properties = schema.get("properties").and_then(Value::as_object)?;
    for (name, property_schema) in properties {
        if schema_type(property_schema) != Some("array") {
            continue;
        }
        let item_schema = property_schema.get("items")?;
        if let Some(sample) = scalar_sample_value(item_schema) {
            return Some((format!("{name}[0]"), sample));
        }
        if schema_type(item_schema) == Some("object") {
            let nested_properties = item_schema.get("properties").and_then(Value::as_object)?;
            for (child_name, child_schema) in nested_properties {
                if let Some(sample) = scalar_sample_value(child_schema) {
                    return Some((format!("{name}[0].{child_name}"), sample));
                }
            }
        }
    }
    None
}

fn first_json_assignment_example(schema: &Value) -> Option<(String, Value)> {
    let properties = schema.get("properties").and_then(Value::as_object)?;
    for (name, property_schema) in properties {
        if !matches!(schema_type(property_schema), Some("object" | "array")) {
            continue;
        }
        if let Some(value) = sample_json_value(property_schema, 2) {
            return Some((name.clone(), value));
        }
    }
    None
}

fn first_file_field_name(input_schema: &Value) -> Option<String> {
    if let Some(name) = input_schema
        .get("content")
        .and_then(|content| content.get("multipart/form-data"))
        .and_then(|multipart| multipart.get("x-uxc-file-fields"))
        .and_then(Value::as_array)
        .and_then(|fields| fields.first())
        .and_then(Value::as_str)
    {
        return Some(name.to_string());
    }

    let schema = select_operation_input_schema_for_examples(input_schema)?;
    let properties = schema.get("properties").and_then(Value::as_object)?;
    for (name, property_schema) in properties {
        if schema_type(property_schema) == Some("string")
            && property_schema.get("format").and_then(Value::as_str) == Some("binary")
        {
            return Some(name.clone());
        }
    }
    None
}

fn sample_json_value(schema: &Value, depth: usize) -> Option<Value> {
    if depth == 0 {
        return None;
    }
    match schema_type(schema) {
        Some("object") => {
            let properties = schema.get("properties").and_then(Value::as_object)?;
            let mut object = serde_json::Map::new();
            for (name, property_schema) in properties.iter().take(2) {
                if let Some(value) = sample_json_value(property_schema, depth.saturating_sub(1)) {
                    object.insert(name.clone(), value);
                }
            }
            if object.is_empty() {
                Some(Value::Object(serde_json::Map::new()))
            } else {
                Some(Value::Object(object))
            }
        }
        Some("array") => {
            let item_schema = schema.get("items");
            let item = item_schema
                .and_then(|inner| sample_json_value(inner, depth.saturating_sub(1)))
                .unwrap_or_else(|| Value::String("value".to_string()));
            Some(Value::Array(vec![item]))
        }
        _ => scalar_json_sample(schema),
    }
}

fn scalar_sample_value(schema: &Value) -> Option<String> {
    if let Some(sample) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return value_to_cli_scalar(sample);
    }
    if let Some(sample) = schema.get("const") {
        return value_to_cli_scalar(sample);
    }
    match schema_type(schema) {
        Some("integer") => Some("1".to_string()),
        Some("number") => Some("1.5".to_string()),
        Some("boolean") => Some("true".to_string()),
        Some("string") => Some("value".to_string()),
        _ => None,
    }
}

fn value_to_cli_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        Value::Null => Some("null".to_string()),
        _ => None,
    }
}

fn schema_type(schema: &Value) -> Option<&str> {
    match schema.get("type") {
        Some(Value::String(v)) => Some(v.as_str()),
        Some(Value::Array(items)) => items.iter().find_map(Value::as_str),
        _ => None,
    }
}

fn push_unique(values: &mut Vec<String>, candidate: String) {
    if !values.iter().any(|item| item == &candidate) {
        values.push(candidate);
    }
}

fn scalar_json_sample(schema: &Value) -> Option<Value> {
    if let Some(sample) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return Some(sample.clone());
    }
    if let Some(sample) = schema.get("const") {
        return Some(sample.clone());
    }
    match schema_type(schema) {
        Some("integer") => Some(Value::Number(serde_json::Number::from(1))),
        Some("number") => serde_json::Number::from_f64(1.5).map(Value::Number),
        Some("boolean") => Some(Value::Bool(true)),
        Some("string") => Some(Value::String("value".to_string())),
        _ => None,
    }
}

fn print_json(envelope: &OutputEnvelope) -> Result<()> {
    println!("{}", envelope.to_json()?);
    Ok(())
}

fn print_host_help_text_from_summaries(
    protocol: &str,
    endpoint: &str,
    operations: &[OperationSummary],
    examples: &[String],
    service: &Option<ServiceSummary>,
) {
    println!("Protocol: {}", protocol);
    println!("Endpoint: {}", endpoint);
    if let Some(service) = service {
        println!();
        println!("Service:");
        if let Some(name) = &service.name {
            println!("  Name: {}", name);
        }
        if let Some(description) = &service.description {
            println!("  Description: {}", description);
        }
    }
    println!();
    println!("Available operations:");
    for op in operations {
        if let Some(desc) = &op.summary {
            println!("- {} ({}) : {}", op.display_name, op.operation_id, desc);
        } else {
            println!("- {} ({})", op.display_name, op.operation_id);
        }
    }

    if !examples.is_empty() {
        println!();
        println!("Examples:");
        for line in examples {
            println!("  {}", line);
        }
    }
}

fn print_help_text(data: &HelpData) {
    println!("{}", data.about);
    println!();
    println!("Path: {}", data.path);
    println!("Usage: {}", data.usage);

    if !data.commands.is_empty() {
        println!();
        println!("Commands:");
        for command in &data.commands {
            println!("  {:<12} {}", command.name, command.about);
        }
    }

    if !data.notes.is_empty() {
        println!();
        println!("Notes:");
        for note in &data.notes {
            println!("  {}", note);
        }
    }

    if !data.examples.is_empty() {
        println!();
        println!("Examples:");
        for example in &data.examples {
            println!("  {}", example);
        }
    }
}

fn print_detail_text(
    protocol: &str,
    endpoint: &str,
    detail: &OperationDetail,
    invocation_examples: &[String],
) {
    println!("Protocol: {}", protocol);
    println!("Endpoint: {}", endpoint);
    println!("Operation ID: {}", detail.operation_id);
    println!("Display Name: {}", detail.display_name);

    if let Some(description) = &detail.description {
        println!("Description: {}", description);
    }

    if let Some(return_type) = &detail.return_type {
        println!("Return Type: {}", return_type);
    }

    if !detail.parameters.is_empty() {
        println!("\nParameters:");
        for param in &detail.parameters {
            println!(
                "- {} ({}){}",
                param.name,
                param.param_type,
                if param.required { " required" } else { "" }
            );
            if let Some(desc) = &param.description {
                println!("  {}", desc);
            }
        }
    }

    if let Some(input_schema) = &detail.input_schema {
        println!(
            "\nInput Schema:\n{}",
            serde_json::to_string_pretty(input_schema).unwrap_or_else(|_| "{}".to_string())
        );
    }

    if !invocation_examples.is_empty() {
        println!("\nInvocation Examples:");
        for line in invocation_examples {
            println!("  {}", line);
        }
    }
}

async fn handle_link_command(
    name: &str,
    host: &str,
    options: LinkCommandOptions<'_>,
) -> Result<OutputEnvelope> {
    validate_link_name(name)?;

    let host = host.trim();
    if host.is_empty() {
        return Err(UxcError::InvalidArguments("Host cannot be empty".to_string()).into());
    }
    let schema_url = match options.schema_url {
        Some(value) if value.trim().is_empty() => {
            return Err(
                UxcError::InvalidArguments("Schema URL cannot be empty".to_string()).into(),
            );
        }
        Some(value) => Some(value.trim()),
        None => None,
    };
    let credential = options
        .credential
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if options.explicit_auth.is_some() && credential.is_some() {
        return Err(UxcError::InvalidArguments(
            "uxc link does not allow both global --auth and --credential; use --credential only"
                .to_string(),
        )
        .into());
    }
    let persisted_credential = credential.or(options.explicit_auth);
    if let Some(id) = persisted_credential {
        auth::Profiles::validate_profile_name(id)
            .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
    }
    if !options.inject_env.is_empty() && persisted_credential.is_none() {
        return Err(UxcError::InvalidArguments(
            "--inject-env on uxc link requires either --credential <credential_id> or global --auth"
                .to_string(),
        )
        .into());
    }
    if (!options.inject_env.is_empty() || persisted_credential.is_some())
        && !adapters::mcp::McpAdapter::is_stdio_command(host)
    {
        return Err(UxcError::InvalidArguments(
            "--credential and --inject-env are only supported for stdio link targets".to_string(),
        )
        .into());
    }

    let target_dir = resolve_link_dir(options.dir)?;
    fs::create_dir_all(&target_dir)?;

    let target_path = link_target_path(&target_dir, name);
    let launcher = build_link_launcher(
        name,
        host,
        schema_url,
        persisted_credential,
        options.inject_env,
        options.daemon_exclusive,
        options.daemon_idle_ttl,
    );
    let target_exists_before = target_path.exists();
    write_link_file(&target_path, launcher.as_bytes(), options.force)?;
    set_executable_if_unix(&target_path)?;

    let data = serde_json::to_value(LinkCreateData {
        name: name.to_string(),
        host: host.to_string(),
        path: target_path.display().to_string(),
        overwritten: target_exists_before,
        dir_in_path: is_dir_in_path(&target_dir),
        schema_url: schema_url.map(ToOwned::to_owned),
        credential: persisted_credential.map(ToOwned::to_owned),
        inject_env: options
            .inject_env
            .iter()
            .map(InjectEnvSpec::as_cli_arg)
            .collect(),
        daemon_idle_ttl: options.daemon_idle_ttl,
    })?;

    Ok(OutputEnvelope::success(
        "link_create_result",
        "cli",
        "uxc",
        Some(name),
        data,
        None,
    ))
}

fn link_target_path(dir: &Path, name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            dir.join(name)
        } else {
            dir.join(format!("{}.cmd", name))
        }
    }
    #[cfg(not(windows))]
    {
        dir.join(name)
    }
}

fn build_link_launcher(
    name: &str,
    host: &str,
    schema_url: Option<&str>,
    credential: Option<&str>,
    inject_env: &[InjectEnvSpec],
    daemon_exclusive: &[String],
    daemon_idle_ttl: Option<u64>,
) -> String {
    const LINK_SENTINEL: &str = "# Generated by uxc link; do not edit by hand";

    let exclusive = daemon_exclusive.join(";");
    let idle_ttl = daemon_idle_ttl.map(|value| value.to_string());

    #[cfg(windows)]
    {
        let escaped_name = windows_batch_escape(name);
        let escaped = windows_batch_escape(host);
        let escaped_exclusive = windows_batch_escape(&exclusive);
        let exclusive_line = if escaped_exclusive.is_empty() {
            "REM UXC_DAEMON_EXCLUSIVE is empty".to_string()
        } else {
            format!("set \"UXC_DAEMON_EXCLUSIVE={}\"", escaped_exclusive)
        };
        let idle_ttl_line = if let Some(idle_ttl) = idle_ttl.as_deref() {
            format!(
                "set \"UXC_DAEMON_IDLE_TTL={}\"",
                windows_batch_escape(idle_ttl)
            )
        } else {
            "REM UXC_DAEMON_IDLE_TTL is empty".to_string()
        };
        let mut base_command = format!("uxc \"{}\"", escaped);
        if let Some(credential) = credential {
            base_command.push_str(&format!(" --auth \"{}\"", windows_batch_escape(credential)));
        }
        for spec in inject_env {
            base_command.push_str(&format!(
                " --inject-env \"{}\"",
                windows_batch_escape(&spec.as_cli_arg())
            ));
        }
        let schema_logic = schema_url
            .map(|url| {
                let escaped_url = windows_batch_escape(url);
                format!(
                    "set \"UXC_HAS_SCHEMA_URL=\"\r\nfor %%A in (%*) do (\r\n  if /I \"%%~A\"==\"--schema-url\" set \"UXC_HAS_SCHEMA_URL=1\"\r\n  for /F \"tokens=1 delims==\" %%B in (\"%%~A\") do if /I \"%%~B\"==\"--schema-url\" set \"UXC_HAS_SCHEMA_URL=1\"\r\n)\r\nif defined UXC_HAS_SCHEMA_URL (\r\n  {} %*\r\n) else (\r\n  {} --schema-url \"{}\" %*\r\n)\r\n",
                    base_command, base_command, escaped_url
                )
            })
            .unwrap_or_else(|| format!("{} %*\r\n", base_command));
        return format!(
            "@echo off\r\nREM {}\r\nset \"UXC_LINK_NAME={}\"\r\n{}\r\n{}\r\n{}",
            LINK_SENTINEL, escaped_name, exclusive_line, idle_ttl_line, schema_logic
        );
    }
    #[cfg(not(windows))]
    {
        let exclusive_prefix = if exclusive.is_empty() {
            String::new()
        } else {
            format!("UXC_DAEMON_EXCLUSIVE={} ", shell_single_quote(&exclusive))
        };
        let idle_ttl_prefix = idle_ttl
            .as_deref()
            .map(|value| format!("UXC_DAEMON_IDLE_TTL={} ", shell_single_quote(value)))
            .unwrap_or_default();
        let mut exec_prefix = format!(
            "{}{}UXC_LINK_NAME={} exec uxc {}",
            exclusive_prefix,
            idle_ttl_prefix,
            shell_single_quote(name),
            shell_single_quote(host)
        );
        if let Some(credential) = credential {
            exec_prefix.push_str(&format!(" --auth {}", shell_single_quote(credential)));
        }
        for spec in inject_env {
            exec_prefix.push_str(&format!(
                " --inject-env {}",
                shell_single_quote(&spec.as_cli_arg())
            ));
        }
        if let Some(schema_url) = schema_url {
            format!(
                "#!/usr/bin/env sh\n{}\nhas_schema_url=false\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    --schema-url|--schema-url=*)\n      has_schema_url=true\n      break\n      ;;\n  esac\ndone\n\nif [ \"$has_schema_url\" = true ]; then\n  {} \"$@\"\nelse\n  {} --schema-url {} \"$@\"\nfi\n",
                LINK_SENTINEL,
                exec_prefix,
                exec_prefix,
                shell_single_quote(schema_url)
            )
        } else {
            format!(
                "#!/usr/bin/env sh\n{}\n{} \"$@\"\n",
                LINK_SENTINEL, exec_prefix
            )
        }
    }
}

#[cfg(windows)]
fn windows_batch_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\"\""),
            '%' => out.push_str("%%"),
            '^' => out.push_str("^^"),
            '&' => out.push_str("^&"),
            '|' => out.push_str("^|"),
            '<' => out.push_str("^<"),
            '>' => out.push_str("^>"),
            '\r' | '\n' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

fn collect_daemon_exclusive_keys(cli: &Cli) -> Result<Vec<String>> {
    let mut keys = Vec::new();

    for k in &cli.daemon_exclusive {
        let trimmed = k.trim();
        if trimmed.is_empty() {
            continue;
        }
        keys.push(expand_tilde_key(trimmed)?);
    }

    if let Ok(raw) = std::env::var("UXC_DAEMON_EXCLUSIVE") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let parts: Vec<&str> = if trimmed.contains(';') {
                trimmed.split(';').collect()
            } else {
                // Support ":" for POSIX convenience.
                // On Windows, do NOT split on ":" to avoid drive letter ambiguity (C:\...).
                if cfg!(windows) {
                    vec![trimmed]
                } else {
                    trimmed.split(':').collect()
                }
            };
            for part in parts {
                let t = part.trim();
                if t.is_empty() {
                    continue;
                }
                keys.push(expand_tilde_key(t)?);
            }
        }
    }

    keys.sort();
    keys.dedup();
    Ok(keys)
}

fn collect_daemon_idle_ttl(cli: &Cli) -> Result<Option<u64>> {
    if let Some(value) = cli.daemon_idle_ttl {
        return Ok(Some(value));
    }

    let Ok(raw) = std::env::var("UXC_DAEMON_IDLE_TTL") else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let ttl = trimmed.parse::<u64>().map_err(|_| {
        UxcError::InvalidArguments(format!(
            "Invalid UXC_DAEMON_IDLE_TTL value '{}': expected non-negative integer seconds",
            trimmed
        ))
    })?;
    Ok(Some(ttl))
}

fn collect_inject_env_specs(cli: &Cli) -> Result<Vec<InjectEnvSpec>> {
    parse_inject_env_specs(&cli.inject_env)
}

fn expand_tilde_key(key: &str) -> Result<String> {
    if key == "~" || key.starts_with("~/") || key.starts_with("~\\") {
        let home = resolve_home_dir().ok_or_else(|| {
            UxcError::ExecutionFailed("Could not determine home directory".to_string())
        })?;
        if key == "~" {
            return Ok(home.display().to_string());
        }
        let rest = key
            .strip_prefix("~/")
            .or_else(|| key.strip_prefix("~\\"))
            .unwrap_or("");
        return Ok(home.join(rest).display().to_string());
    }
    Ok(key.to_string())
}

fn write_link_file(target_path: &Path, content: &[u8], force: bool) -> Result<()> {
    if !force {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target_path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    UxcError::InvalidArguments(format!(
                        "Shortcut '{}' already exists at {}. Use --force to overwrite.",
                        target_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("shortcut"),
                        target_path.display()
                    ))
                } else {
                    UxcError::IoError(err)
                }
            })?;
        file.write_all(content)?;
        file.sync_all()?;
        return Ok(());
    }

    if target_path.exists() && !is_uxc_managed_link_file(target_path)? {
        return Err(UxcError::InvalidArguments(format!(
            "Refusing to overwrite '{}': existing file is not a uxc-managed shortcut.",
            target_path.display()
        ))
        .into());
    }

    let temp_path = temporary_link_path(target_path);
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
    }

    #[cfg(windows)]
    if target_path.exists() {
        fs::remove_file(target_path)?;
    }

    fs::rename(&temp_path, target_path).map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        UxcError::IoError(err)
    })?;
    Ok(())
}

fn is_uxc_managed_link_file(target_path: &Path) -> Result<bool> {
    let metadata = fs::metadata(target_path)?;
    if !metadata.is_file() {
        return Ok(false);
    }

    let bytes = read_file_prefix(target_path, 4096)?;
    Ok(looks_like_uxc_link_launcher(&bytes))
}

fn looks_like_uxc_link_launcher(content: &[u8]) -> bool {
    let text = String::from_utf8_lossy(content);
    let sentinel = "Generated by uxc link; do not edit by hand";
    if !text.contains(sentinel) {
        return false;
    }

    let unix_has_link_name = text
        .lines()
        .take(30)
        .any(|line| line.contains("UXC_LINK_NAME='"));
    let unix_has_exec = text.lines().take(40).any(|line| line.contains("exec uxc "));
    let unix_like = unix_has_link_name && unix_has_exec;

    // Windows launcher shape:
    //   set "UXC_LINK_NAME=name"
    //   uxc "host" %*
    let mut windows_like = false;
    let mut set_line_index: Option<usize> = None;
    for (idx, line) in text.lines().take(20).enumerate() {
        let trimmed = line.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("set \"uxc_link_name=") {
            set_line_index = Some(idx);
            break;
        }
    }
    if let Some(start_idx) = set_line_index {
        for line in text.lines().skip(start_idx + 1).take(10) {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                continue;
            }
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with("uxc \"") && lower.ends_with("%*") {
                windows_like = true;
                break;
            }
        }
    }

    unix_like || windows_like
}

fn read_file_prefix(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(max_bytes).read_to_end(&mut buf)?;
    Ok(buf)
}

fn temporary_link_path(target_path: &Path) -> PathBuf {
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("uxc-link");
    let pid = std::process::id();
    for nonce in 0..1000u32 {
        let candidate = parent.join(format!(".{}.{}.{}.tmp", file_name, pid, nonce));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!(".{}.{}.tmp", file_name, pid))
}

fn set_executable_if_unix(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(path)?;
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

fn validate_link_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(UxcError::InvalidArguments("Shortcut name cannot be empty".to_string()).into());
    }
    if name == "." || name == ".." {
        return Err(
            UxcError::InvalidArguments("Shortcut name cannot be '.' or '..'".to_string()).into(),
        );
    }
    if name.contains('/') || name.contains('\\') {
        return Err(UxcError::InvalidArguments(
            "Shortcut name cannot contain path separators".to_string(),
        )
        .into());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Err(UxcError::InvalidArguments(
            "Shortcut name may only contain letters, digits, '-', '_', and '.'".to_string(),
        )
        .into());
    }
    Ok(())
}

fn resolve_link_dir(dir: Option<&str>) -> Result<PathBuf> {
    match dir {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(UxcError::InvalidArguments(
                    "Shortcut directory cannot be empty".to_string(),
                )
                .into());
            }
            if trimmed == "~" || trimmed.starts_with("~/") {
                let home = resolve_home_dir().ok_or_else(|| {
                    UxcError::ExecutionFailed("Could not determine home directory".to_string())
                })?;
                if trimmed == "~" {
                    Ok(home)
                } else {
                    Ok(home.join(trimmed.trim_start_matches("~/")))
                }
            } else {
                Ok(PathBuf::from(trimmed))
            }
        }
        None => {
            let home = resolve_home_dir().ok_or_else(|| {
                UxcError::ExecutionFailed("Could not determine home directory".to_string())
            })?;
            #[cfg(windows)]
            {
                Ok(home.join(".uxc").join("bin"))
            }
            #[cfg(not(windows))]
            {
                Ok(home.join(".local").join("bin"))
            }
        }
    }
}

fn resolve_home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        let home_drive = std::env::var_os("HOMEDRIVE");
        let home_path = std::env::var_os("HOMEPATH");
        if let (Some(drive), Some(path)) = (home_drive, home_path) {
            let mut combined = PathBuf::from(drive);
            combined.push(path);
            return Some(combined);
        }
    }
    None
}

fn shell_single_quote(input: &str) -> String {
    if input.is_empty() {
        "''".to_string()
    } else {
        format!("'{}'", input.replace('\'', "'\"'\"'"))
    }
}

fn normalized_existing_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_dir_in_path(dir: &Path) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    let normalized_dir = normalized_existing_path(dir);
    std::env::split_paths(&path_var)
        .map(|entry| normalized_existing_path(&entry))
        .any(|entry| entry == normalized_dir)
}

fn error_code(err: &anyhow::Error) -> &'static str {
    for cause in err.chain() {
        if let Some(uxc_error) = cause.downcast_ref::<UxcError>() {
            return match uxc_error {
                UxcError::ProtocolDetectionFailed(_) | UxcError::UnsupportedProtocol(_) => {
                    "PROTOCOL_DETECTION_FAILED"
                }
                UxcError::OperationNotFound(_) => "OPERATION_NOT_FOUND",
                UxcError::InvalidArguments(_) => "INVALID_ARGUMENT",
                UxcError::HttpError { .. } => "HTTP_ERROR",
                UxcError::OAuthRequired(_) => "OAUTH_REQUIRED",
                UxcError::OAuthDiscoveryFailed(_) => "OAUTH_DISCOVERY_FAILED",
                UxcError::OAuthTokenExchangeFailed(_) => "OAUTH_TOKEN_EXCHANGE_FAILED",
                UxcError::OAuthSessionNotFound(_) => "OAUTH_SESSION_NOT_FOUND",
                UxcError::OAuthSessionExpired(_) => "OAUTH_SESSION_EXPIRED",
                UxcError::OAuthRefreshFailed(_) => "OAUTH_REFRESH_FAILED",
                UxcError::OAuthScopeInsufficient(_) => "OAUTH_SCOPE_INSUFFICIENT",
                UxcError::DaemonVersionMismatch(_) => "DAEMON_VERSION_MISMATCH",
                UxcError::ExecutionFailed(_)
                | UxcError::SchemaRetrievalFailed(_)
                | UxcError::NetworkError(_)
                | UxcError::JsonError(_)
                | UxcError::IoError(_)
                | UxcError::GenericError(_) => "EXECUTION_FAILED",
            };
        }

        if cause.downcast_ref::<serde_json::Error>().is_some() {
            return "INVALID_ARGUMENT";
        }
    }

    "EXECUTION_FAILED"
}

async fn handle_cache_command(
    command: &CacheCommands,
    cache_config: CacheConfig,
) -> Result<OutputEnvelope> {
    let cache = cache::create_cache(cache_config)?;

    match command {
        CacheCommands::List => {
            let entries = cache.list_entries()?;
            let data = serde_json::to_value(CacheListData {
                count: entries.len(),
                entries,
            })?;
            Ok(OutputEnvelope::success(
                "cache_list",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        CacheCommands::Stats => {
            let stats = cache.stats()?;
            let data = serde_json::to_value(stats)?;
            Ok(OutputEnvelope::success(
                "cache_stats",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        CacheCommands::Clear { url, all, key } => {
            if *all {
                cache.clear()?;
                let data = serde_json::to_value(CacheClearData {
                    scope: "all".to_string(),
                    url: None,
                    key: None,
                })?;
                Ok(OutputEnvelope::success(
                    "cache_clear_result",
                    "cli",
                    "uxc",
                    None,
                    data,
                    None,
                ))
            } else if let Some(key) = key {
                let normalized_input = key.trim();
                cache.invalidate_by_key(normalized_input)?;
                let normalized_key = normalized_input
                    .strip_suffix(".json")
                    .unwrap_or(normalized_input)
                    .to_string();
                let data = serde_json::to_value(CacheClearData {
                    scope: "key".to_string(),
                    url: None,
                    key: Some(normalized_key),
                })?;
                Ok(OutputEnvelope::success(
                    "cache_clear_result",
                    "cli",
                    "uxc",
                    None,
                    data,
                    None,
                ))
            } else if let Some(url) = url {
                let normalized_url = normalize_endpoint_url(url);
                cache.invalidate(&normalized_url)?;
                let data = serde_json::to_value(CacheClearData {
                    scope: "url".to_string(),
                    url: Some(normalized_url),
                    key: None,
                })?;
                Ok(OutputEnvelope::success(
                    "cache_clear_result",
                    "cli",
                    "uxc",
                    None,
                    data,
                    None,
                ))
            } else {
                Err(UxcError::InvalidArguments(
                    "Usage: uxc cache clear <url> | uxc cache clear --key <cache_key> | uxc cache clear --all".to_string(),
                )
                .into())
            }
        }
    }
}

async fn handle_daemon_command(command: &DaemonCommands) -> Result<OutputEnvelope> {
    match command {
        DaemonCommands::Start => {
            let outcome = daemon::daemon_start_local().await?;
            let data = json!({
                "started": outcome.started_now,
                "autostarted": outcome.started_now && !outcome.restarted_for_version_mismatch,
                "started_now": outcome.started_now,
                "already_running": !outcome.started_now,
                "restarted_for_version_mismatch": outcome.restarted_for_version_mismatch,
                "previous_version": outcome.previous_version,
                "version": env!("CARGO_PKG_VERSION"),
                "socket": daemon::socket_path().display().to_string()
            });
            Ok(OutputEnvelope::success(
                "daemon_start_result",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        DaemonCommands::Stop => {
            let stopped = daemon::daemon_stop_local().await?;
            let data = json!({
                "stopped": stopped,
                "socket": daemon::socket_path().display().to_string()
            });
            Ok(OutputEnvelope::success(
                "daemon_stop_result",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        DaemonCommands::Status => {
            let status = match daemon::daemon_status_local().await {
                Ok(status) => {
                    let running = status.running;
                    let version_mismatch =
                        running && status.version.as_deref() != Some(env!("CARGO_PKG_VERSION"));
                    let mut value = serde_json::to_value(status)?;
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert(
                            "client_version".to_string(),
                            Value::String(env!("CARGO_PKG_VERSION").to_string()),
                        );
                        obj.insert(
                            "version_mismatch".to_string(),
                            Value::Bool(version_mismatch),
                        );
                    }
                    value
                }
                Err(err) => json!({
                    "running": false,
                    "socket": daemon::socket_path().display().to_string(),
                    "client_version": env!("CARGO_PKG_VERSION"),
                    "version_mismatch": false,
                    "error": {
                        "code": "DAEMON_UNREACHABLE",
                        "message": err.to_string()
                    }
                }),
            };
            Ok(OutputEnvelope::success(
                "daemon_status",
                "cli",
                "uxc",
                None,
                status,
                None,
            ))
        }
        DaemonCommands::Sessions => {
            let sessions = daemon::daemon_sessions_local().await?;
            Ok(OutputEnvelope::success(
                "daemon_sessions",
                "cli",
                "uxc",
                None,
                serde_json::to_value(sessions)?,
                None,
            ))
        }
        DaemonCommands::Restart => {
            let stopped = daemon::daemon_stop_local().await?;
            let outcome = daemon::daemon_start_local().await?;
            let data = json!({
                "stopped": stopped,
                "started_now": outcome.started_now,
                "restarted_for_version_mismatch": outcome.restarted_for_version_mismatch,
                "previous_version": outcome.previous_version,
                "version": env!("CARGO_PKG_VERSION"),
                "socket": daemon::socket_path().display().to_string()
            });
            Ok(OutputEnvelope::success(
                "daemon_restart_result",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        DaemonCommands::Serve => {
            daemon::run_daemon_server().await?;
            Ok(OutputEnvelope::success(
                "daemon_serve_result",
                "cli",
                "uxc",
                None,
                json!({"stopped": true}),
                None,
            ))
        }
    }
}

async fn handle_subscribe_command(
    command: &SubscribeCommands,
    cli: &Cli,
) -> Result<OutputEnvelope> {
    if cli.schema_url.is_some() {
        return Err(UxcError::InvalidArguments(
            "--schema-url is not supported for subscribe commands".to_string(),
        )
        .into());
    }

    let daemon_used = daemon::daemon_supported();
    let daemon_ensure = if daemon_used {
        Some(daemon::ensure_compatible_daemon_running().await?)
    } else {
        None
    };
    let daemon_autostarted = daemon_ensure
        .as_ref()
        .map(|outcome| outcome.started_now && !outcome.restarted_for_version_mismatch);
    let daemon_restarted_for_version_mismatch = daemon_ensure
        .as_ref()
        .map(|outcome| outcome.restarted_for_version_mismatch);

    let envelope = match command {
        SubscribeCommands::Start {
            endpoint,
            operation_id,
            args,
            input_json,
            sink,
            resource_uri,
            read_resource,
            transport,
            subprotocols,
            init_frames,
            mode,
            poll_config,
            ephemeral,
        } => {
            let transport_hint = transport.as_ref().map(|value| match value {
                SubscribeTransportArg::Websocket => daemon::SubscriptionTransportHint::Websocket,
                SubscribeTransportArg::DiscordGateway => {
                    daemon::SubscriptionTransportHint::DiscordGateway
                }
                SubscribeTransportArg::SlackSocketMode => {
                    daemon::SubscriptionTransportHint::SlackSocketMode
                }
                SubscribeTransportArg::FeishuLongConnection => {
                    daemon::SubscriptionTransportHint::FeishuLongConnection
                }
            });
            let mut transport_operation_id = operation_id.clone();
            let mut transport_input_json = input_json.clone();
            if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::DiscordGateway)
            ) {
                if let Some(candidate) = transport_operation_id.as_deref() {
                    if serde_json::from_str::<Value>(candidate)
                        .ok()
                        .is_some_and(|value| value.is_object())
                    {
                        if transport_input_json.is_some() {
                            return Err(UxcError::InvalidArguments(
                                "Cannot provide both --input-json and positional JSON payload"
                                    .to_string(),
                            )
                            .into());
                        }
                        transport_input_json = Some(candidate.to_string());
                        transport_operation_id = None;
                    }
                }
            }
            if !matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::Websocket)
            ) && (!subprotocols.is_empty() || !init_frames.is_empty())
            {
                return Err(UxcError::InvalidArguments(
                    "--subprotocol and --init-frame require explicit --transport websocket"
                        .to_string(),
                )
                .into());
            }
            if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::Websocket)
            ) {
                if operation_id.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport websocket cannot be combined with an operation_id".to_string(),
                    )
                    .into());
                }
                if resource_uri.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport websocket cannot be combined with --resource-uri".to_string(),
                    )
                    .into());
                }
                if !matches!(mode, SubscribeModeArg::Stream) {
                    return Err(UxcError::InvalidArguments(
                        "--transport websocket is only valid with --mode stream".to_string(),
                    )
                    .into());
                }
            }
            if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::DiscordGateway)
            ) {
                if transport_operation_id.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport discord-gateway cannot be combined with an operation_id"
                            .to_string(),
                    )
                    .into());
                }
                if resource_uri.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport discord-gateway cannot be combined with --resource-uri"
                            .to_string(),
                    )
                    .into());
                }
                if !matches!(mode, SubscribeModeArg::Stream) {
                    return Err(UxcError::InvalidArguments(
                        "--transport discord-gateway is only valid with --mode stream".to_string(),
                    )
                    .into());
                }
            }
            if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::SlackSocketMode)
            ) {
                if operation_id.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport slack-socket-mode cannot be combined with an operation_id"
                            .to_string(),
                    )
                    .into());
                }
                if resource_uri.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport slack-socket-mode cannot be combined with --resource-uri"
                            .to_string(),
                    )
                    .into());
                }
                if !matches!(mode, SubscribeModeArg::Stream) {
                    return Err(UxcError::InvalidArguments(
                        "--transport slack-socket-mode is only valid with --mode stream"
                            .to_string(),
                    )
                    .into());
                }
            }
            if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::FeishuLongConnection)
            ) {
                if operation_id.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport feishu-long-connection cannot be combined with an operation_id"
                            .to_string(),
                    )
                    .into());
                }
                if resource_uri.is_some() {
                    return Err(UxcError::InvalidArguments(
                        "--transport feishu-long-connection cannot be combined with --resource-uri"
                            .to_string(),
                    )
                    .into());
                }
                if !matches!(mode, SubscribeModeArg::Stream) {
                    return Err(UxcError::InvalidArguments(
                        "--transport feishu-long-connection is only valid with --mode stream"
                            .to_string(),
                    )
                    .into());
                }
            }
            if *read_resource && resource_uri.is_none() {
                return Err(UxcError::InvalidArguments(
                    "--read-resource requires --resource-uri".to_string(),
                )
                .into());
            }
            if *read_resource && !matches!(mode, SubscribeModeArg::Stream) {
                return Err(UxcError::InvalidArguments(
                    "--read-resource is only valid with --mode stream".to_string(),
                )
                .into());
            }
            let (normalized_args, normalized_input_json) = match transport_operation_id.as_ref() {
                Some(op) => {
                    let mut explicit_args = Vec::new();
                    let mut positional = Vec::new();
                    for arg in args {
                        if arg.contains('=') {
                            explicit_args.push(arg.clone());
                        } else {
                            positional.push(arg.clone());
                        }
                    }
                    normalize_operation_inputs(
                        op,
                        explicit_args,
                        transport_input_json.clone(),
                        &positional,
                    )?
                }
                None => {
                    if matches!(
                        transport_hint,
                        Some(daemon::SubscriptionTransportHint::DiscordGateway)
                    ) {
                        let mut explicit_args = Vec::new();
                        let mut positional = Vec::new();
                        for arg in args {
                            if arg.contains('=') {
                                explicit_args.push(arg.clone());
                            } else {
                                positional.push(arg.clone());
                            }
                        }
                        normalize_operation_inputs(
                            "discord-gateway",
                            explicit_args,
                            transport_input_json.clone(),
                            &positional,
                        )?
                    } else if matches!(
                        transport_hint,
                        Some(daemon::SubscriptionTransportHint::FeishuLongConnection)
                    ) {
                        let mut explicit_args = Vec::new();
                        let mut positional = Vec::new();
                        for arg in args {
                            if arg.contains('=') {
                                explicit_args.push(arg.clone());
                            } else {
                                positional.push(arg.clone());
                            }
                        }
                        normalize_operation_inputs(
                            "feishu-long-connection",
                            explicit_args,
                            transport_input_json.clone(),
                            &positional,
                        )?
                    } else if transport_input_json.is_some() || !args.is_empty() {
                        return Err(UxcError::InvalidArguments(
                            "subscribe start only accepts operation arguments when <operation_id> is provided".to_string(),
                        )
                        .into());
                    } else {
                        (Vec::new(), None)
                    }
                }
            };
            let args_map = if let Some(op) = transport_operation_id.as_ref() {
                Some(
                    parse_arguments(normalized_args, normalized_input_json).map_err(|err| {
                        UxcError::InvalidArguments(format!(
                            "Invalid arguments for subscribe operation '{}': {}",
                            op, err
                        ))
                    })?,
                )
            } else if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::DiscordGateway)
            ) && (normalized_input_json.is_some() || !normalized_args.is_empty())
            {
                Some(
                    parse_arguments(normalized_args, normalized_input_json).map_err(|err| {
                        UxcError::InvalidArguments(format!(
                            "Invalid arguments for subscribe transport 'discord-gateway': {}",
                            err
                        ))
                    })?,
                )
            } else if matches!(
                transport_hint,
                Some(daemon::SubscriptionTransportHint::FeishuLongConnection)
            ) && (normalized_input_json.is_some() || !normalized_args.is_empty())
            {
                Some(
                    parse_arguments(normalized_args, normalized_input_json).map_err(|err| {
                        UxcError::InvalidArguments(format!(
                            "Invalid arguments for subscribe transport 'feishu-long-connection': {}",
                            err
                        ))
                    })?,
                )
            } else {
                None
            };
            let poll_config = match poll_config {
                Some(raw) => Some(serde_json::from_str::<Value>(raw).map_err(|err| {
                    UxcError::InvalidArguments(format!("Invalid JSON for --poll-config: {}", err))
                })?),
                None => None,
            };
            let request = daemon::SubscribeStartRequest {
                request_id: format!(
                    "{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ),
                endpoint: normalize_endpoint_url(endpoint),
                sink: sink.clone(),
                operation_id: transport_operation_id,
                args: args_map,
                resource_uri: resource_uri.clone(),
                read_resource: *read_resource,
                transport_hint,
                subprotocols: subprotocols.clone(),
                initial_text_frames: init_frames.clone(),
                mode: match mode {
                    SubscribeModeArg::Stream => daemon::SubscriptionMode::Stream,
                    SubscribeModeArg::Poll => daemon::SubscriptionMode::Poll,
                },
                poll_config,
                ephemeral: *ephemeral,
                options: daemon::RuntimeInvokeOptions {
                    auth: cli.auth.clone(),
                    inject_env: collect_inject_env_specs(cli)?,
                    no_cache: cli.no_cache,
                    cache_ttl: cli.cache_ttl,
                    timeout_ms: cli.timeout_ms,
                    refresh_schema: cli.refresh_schema,
                    schema_url: None,
                    link_name: std::env::var("UXC_LINK_NAME").ok(),
                    schema_mapping_file: None,
                    daemon_exclusive: collect_daemon_exclusive_keys(cli)?,
                    daemon_idle_ttl: collect_daemon_idle_ttl(cli)?,
                },
            };
            let data = serde_json::to_value(daemon::subscribe_start_client(&request).await?)?;
            OutputEnvelope::success("subscribe_start_result", "cli", "uxc", None, data, None)
        }
        SubscribeCommands::List => {
            let data = serde_json::to_value(SubscribeListData {
                jobs: daemon::subscribe_list_client().await?,
            })?;
            OutputEnvelope::success("subscribe_list", "cli", "uxc", None, data, None)
        }
        SubscribeCommands::Status { job_id } => {
            let data = serde_json::to_value(daemon::subscribe_status_client(job_id).await?)?;
            OutputEnvelope::success("subscribe_status", "cli", "uxc", None, data, None)
        }
        SubscribeCommands::Stop { job_id } => {
            let data = serde_json::to_value(daemon::subscribe_stop_client(job_id).await?)?;
            OutputEnvelope::success("subscribe_stop_result", "cli", "uxc", None, data, None)
        }
    };

    Ok(envelope.with_daemon_meta(
        daemon_used,
        daemon_autostarted,
        daemon_restarted_for_version_mismatch,
        None,
    ))
}

async fn handle_auth_command(command: &AuthCommands) -> Result<OutputEnvelope> {
    match command {
        AuthCommands::Credential { credential_command } => {
            handle_auth_credential_command(credential_command).await
        }
        AuthCommands::Info { credential_id } => {
            handle_auth_credential_command(&AuthCredentialCommands::Info {
                credential_id: credential_id.clone(),
            })
            .await
        }
        AuthCommands::Binding { binding_command } => handle_auth_binding_command(binding_command),
        AuthCommands::Bootstrap { bootstrap_command } => {
            handle_auth_bootstrap_command(bootstrap_command).await
        }
        AuthCommands::Oauth { oauth_command } => handle_auth_oauth_command(oauth_command).await,
    }
}

async fn handle_auth_bootstrap_command(command: &AuthBootstrapCommands) -> Result<OutputEnvelope> {
    match command {
        AuthBootstrapCommands::Set {
            credential_id,
            token_endpoint,
            request_json,
            header,
            access_token_pointer,
            expires_in_pointer,
            token_type_pointer,
            success_code_pointer,
            success_code_value,
            refresh_skew_seconds,
        } => {
            let mut profiles = Profiles::load_profiles()?;
            let mut profile = profiles.get_profile(credential_id)?.clone();
            profile.name = Some(credential_id.clone());
            if profile.auth_type != AuthType::Bearer {
                return Err(UxcError::InvalidArguments(format!(
                    "Token bootstrap is only supported for bearer credentials, not '{}'",
                    profile.auth_type
                ))
                .into());
            }

            let mut headers = Vec::with_capacity(header.len());
            for spec in header {
                let parsed = AuthHeader::parse(spec)
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                headers.push(parsed);
            }

            let normalized_success_code_value = success_code_value
                .as_ref()
                .map(|value| serde_json::from_str::<Value>(value))
                .transpose()
                .map_err(|e| {
                    UxcError::InvalidArguments(format!(
                        "Invalid --success-code-value JSON literal: {}",
                        e
                    ))
                })?
                .map(|value| value.to_string());

            let config = auth::TokenBootstrapConfig {
                token_endpoint: token_endpoint.clone(),
                request_json: request_json.clone(),
                headers,
                access_token_pointer: access_token_pointer.clone(),
                expires_in_pointer: expires_in_pointer.clone(),
                token_type_pointer: token_type_pointer.clone(),
                success_code_pointer: success_code_pointer.clone(),
                success_code_value: normalized_success_code_value,
                refresh_skew_seconds: *refresh_skew_seconds,
            };
            auth::validate_token_bootstrap_config(&config)
                .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;

            profile.bootstrap = Some(config);
            profile.bootstrap_state = None;
            profile.api_key.clear();
            profiles.set_profile(credential_id.clone(), profile.clone())?;
            profiles.save_profiles()?;

            let data = serde_json::to_value(AuthBootstrapInfoData {
                credential: credential_id.clone(),
                auth_type: profile.auth_type.to_string(),
                fields: {
                    let fields = profile
                        .field_source_kinds()
                        .into_iter()
                        .map(|(field_name, source_kind)| AuthFieldView {
                            name: field_name,
                            source_kind,
                            value_masked: "***".to_string(),
                        })
                        .collect::<Vec<_>>();
                    if fields.is_empty() {
                        None
                    } else {
                        Some(fields)
                    }
                },
                bootstrap: to_auth_bootstrap_view(&profile)
                    .expect("bootstrap should exist after bootstrap set"),
            })?;
            Ok(OutputEnvelope::success(
                "auth_bootstrap_set_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthBootstrapCommands::Info { credential_id } => {
            let profiles = Profiles::load_profiles()?;
            let profile = profiles.get_profile(credential_id)?;
            let bootstrap = to_auth_bootstrap_view(profile).ok_or_else(|| {
                UxcError::InvalidArguments(format!(
                    "Credential '{}' does not have token bootstrap configured",
                    credential_id
                ))
            })?;
            let data = serde_json::to_value(AuthBootstrapInfoData {
                credential: credential_id.clone(),
                auth_type: profile.auth_type.to_string(),
                fields: {
                    let fields = profile
                        .field_source_kinds()
                        .into_iter()
                        .map(|(field_name, source_kind)| AuthFieldView {
                            name: field_name,
                            source_kind,
                            value_masked: "***".to_string(),
                        })
                        .collect::<Vec<_>>();
                    if fields.is_empty() {
                        None
                    } else {
                        Some(fields)
                    }
                },
                bootstrap,
            })?;
            Ok(OutputEnvelope::success(
                "auth_bootstrap_info",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthBootstrapCommands::Refresh { credential_id } => {
            let client = build_resilient_http_client(
                std::time::Duration::from_secs(30),
                "auth bootstrap refresh",
            )?;
            let mut profiles = Profiles::load_profiles()?;
            let mut profile = profiles.get_profile(credential_id)?.clone();
            profile.name = Some(credential_id.clone());
            let refreshed = auth::refresh_bootstrap_profile(&mut profile, &client).await?;
            profiles.set_profile(credential_id.clone(), profile.clone())?;
            profiles.save_profiles()?;
            let data = serde_json::to_value(json!({
                "credential": credential_id,
                "refreshed": refreshed,
                "bootstrap": to_auth_bootstrap_view(&profile),
            }))?;
            Ok(OutputEnvelope::success(
                "auth_bootstrap_refresh_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthBootstrapCommands::Remove { credential_id } => {
            let mut profiles = Profiles::load_profiles()?;
            let mut profile = profiles.get_profile(credential_id)?.clone();
            let had_bootstrap = profile.bootstrap.is_some() || profile.bootstrap_state.is_some();
            profile.bootstrap = None;
            profile.bootstrap_state = None;
            if profile.auth_type == AuthType::Bearer && profile.secret_source.is_none() {
                profile.api_key.clear();
            }
            profiles.set_profile(credential_id.clone(), profile)?;
            profiles.save_profiles()?;
            let data = serde_json::to_value(json!({
                "credential": credential_id,
                "removed": had_bootstrap,
            }))?;
            Ok(OutputEnvelope::success(
                "auth_bootstrap_remove_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
    }
}

async fn handle_auth_credential_command(
    command: &AuthCredentialCommands,
) -> Result<OutputEnvelope> {
    match command {
        AuthCredentialCommands::List => {
            let profiles = Profiles::load_profiles()?;
            let mut rendered = Vec::new();
            for name in profiles.profile_names() {
                let profile = profiles.get_profile(&name)?;
                rendered.push(to_auth_profile_view(&name, profile));
            }
            let data = serde_json::to_value(AuthListData {
                count: rendered.len(),
                credentials: rendered,
            })?;
            Ok(OutputEnvelope::success(
                "auth_list",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        AuthCredentialCommands::Info { credential_id } => {
            let profiles = Profiles::load_profiles()?;
            let profile_data = profiles.get_profile(credential_id)?;
            let data = serde_json::to_value(to_auth_profile_view(credential_id, profile_data))?;
            Ok(OutputEnvelope::success(
                "auth_info",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthCredentialCommands::Set {
            credential_id,
            auth_type,
            secret,
            secret_env,
            secret_op,
            api_key_header,
            header,
            query_param,
            path_prefix_template,
            field,
            description,
        } => {
            let mut profiles = Profiles::load_profiles()?;
            let existing = profiles.profiles.get(credential_id).cloned();
            let previous_auth_type = existing.as_ref().map(|p| p.auth_type.clone());

            let resolved_auth_type = match auth_type {
                Some(value) => value
                    .parse::<AuthType>()
                    .map_err(|e| anyhow::anyhow!("Invalid auth type: {}", e))?,
                None => existing
                    .as_ref()
                    .map(|p| p.auth_type.clone())
                    .unwrap_or(AuthType::Bearer),
            };

            let provided_secret_flags =
                [secret.is_some(), secret_env.is_some(), secret_op.is_some()]
                    .iter()
                    .filter(|present| **present)
                    .count();
            if provided_secret_flags > 1 {
                return Err(UxcError::InvalidArguments(
                    "Use only one of --secret, --secret-env, or --secret-op".to_string(),
                )
                .into());
            }

            if resolved_auth_type == AuthType::OAuth && provided_secret_flags > 0 {
                return Err(UxcError::InvalidArguments(
                    "OAuth credential set does not accept --secret/--secret-env/--secret-op. Use `uxc auth oauth login <credential_id> ...`.".to_string(),
                )
                .into());
            }

            if resolved_auth_type != AuthType::ApiKey && path_prefix_template.is_some() {
                return Err(UxcError::InvalidArguments(
                    "--path-prefix-template can only be used with --auth-type api_key".to_string(),
                )
                .into());
            }
            if resolved_auth_type != AuthType::ApiKey
                && resolved_auth_type != AuthType::OAuth
                && (api_key_header.is_some() || !header.is_empty() || !query_param.is_empty())
            {
                return Err(UxcError::InvalidArguments(
                    "--api-key-header/--header/--query-param can only be used with --auth-type api_key or oauth"
                        .to_string(),
                )
                .into());
            }

            if resolved_auth_type != AuthType::OAuth
                && previous_auth_type == Some(AuthType::OAuth)
                && provided_secret_flags == 0
                && (resolved_auth_type != AuthType::ApiKey
                    || (api_key_header.is_none() && header.is_empty() && query_param.is_empty()))
            {
                return Err(UxcError::InvalidArguments(
                    "Switching credential from oauth to non-oauth requires an explicit secret source (--secret, --secret-env, or --secret-op).".to_string(),
                )
                .into());
            }

            let mut profile_obj =
                existing.unwrap_or_else(|| Profile::new(String::new(), resolved_auth_type.clone()));
            profile_obj.auth_type = resolved_auth_type.clone();
            profile_obj.name = Some(credential_id.clone());

            if let Some(header_name) = api_key_header {
                let parsed = AuthHeader::new(header_name, "{{secret}}")
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                profile_obj.auth_headers = Some(vec![parsed]);
            } else if !header.is_empty() {
                let mut auth_headers = Vec::with_capacity(header.len());
                for spec in header {
                    let parsed = AuthHeader::parse(spec)
                        .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                    auth_headers.push(parsed);
                }
                crate::auth::validate_auth_headers(&auth_headers)
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                profile_obj.auth_headers = Some(auth_headers);
            }
            if !query_param.is_empty() {
                let mut auth_query_params = Vec::with_capacity(query_param.len());
                for spec in query_param {
                    let parsed = crate::auth::AuthQueryParam::parse(spec)
                        .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                    auth_query_params.push(parsed);
                }
                crate::auth::validate_auth_query_params(&auth_query_params)
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                profile_obj.auth_query_params = Some(auth_query_params);
            }
            if let Some(template) = path_prefix_template {
                let normalized = crate::auth::validate_auth_path_prefix_template(template)
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                profile_obj.auth_path_prefix = Some(normalized);
            }

            if resolved_auth_type == AuthType::OAuth {
                profile_obj.clear_fields();
                if !field.is_empty() {
                    return Err(UxcError::InvalidArguments(
                        "--field is not supported for OAuth credentials".to_string(),
                    )
                    .into());
                }
            } else if !field.is_empty() {
                profile_obj.clear_fields();
                for spec in field {
                    let (name, source) = crate::auth::parse_field_spec(spec)
                        .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                    profile_obj
                        .set_field_source(name, source)
                        .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                }
            }

            if resolved_auth_type != AuthType::OAuth {
                let has_existing_secret = matches!(
                    profile_obj.secret_source,
                    Some(crate::auth::SecretSource::Literal { .. })
                        | Some(crate::auth::SecretSource::Env { .. })
                        | Some(crate::auth::SecretSource::Op { .. })
                ) || (previous_auth_type != Some(AuthType::OAuth)
                    && !profile_obj.api_key.is_empty());

                let requires_secret = if resolved_auth_type == AuthType::ApiKey {
                    if profile_obj.has_custom_api_key_headers()
                        || profile_obj.has_custom_api_key_query_params()
                        || profile_obj.has_custom_api_key_path_prefix()
                    {
                        profile_obj.api_key_injections_require_secret()
                    } else {
                        profile_obj.fields.is_empty()
                    }
                } else if resolved_auth_type == AuthType::Bearer {
                    profile_obj.fields.is_empty()
                } else {
                    true
                };

                if provided_secret_flags == 0 && !has_existing_secret && requires_secret {
                    return Err(UxcError::InvalidArguments(
                        "Credential set requires one of --secret, --secret-env, or --secret-op"
                            .to_string(),
                    )
                    .into());
                }
            }

            match (secret, secret_env, secret_op) {
                (Some(value), None, None) => {
                    profile_obj.secret_source = Some(crate::auth::SecretSource::Literal {
                        value: value.clone(),
                    });
                    profile_obj.api_key = value.clone();
                }
                (None, Some(env_key), None) => {
                    profile_obj.secret_source = Some(crate::auth::SecretSource::Env {
                        key: env_key.clone(),
                    });
                    profile_obj.api_key.clear();
                }
                (None, None, Some(reference)) => {
                    profile_obj.secret_source = Some(crate::auth::SecretSource::Op {
                        reference: reference.clone(),
                    });
                    profile_obj.api_key.clear();
                }
                (None, None, None) => {}
                _ => unreachable!("secret argument exclusivity is validated above"),
            }

            if provided_secret_flags == 0
                && profile_obj.api_key.is_empty()
                && matches!(
                    profile_obj.secret_source,
                    Some(crate::auth::SecretSource::Literal { ref value }) if value.is_empty()
                )
            {
                profile_obj.secret_source = None;
            }

            if resolved_auth_type == AuthType::OAuth {
                profile_obj.secret_source = None;
                profile_obj.auth_path_prefix = None;
                profile_obj.bootstrap = None;
                profile_obj.bootstrap_state = None;
            } else {
                profile_obj.oauth = None;
                if resolved_auth_type != AuthType::ApiKey {
                    profile_obj.auth_headers = None;
                    profile_obj.auth_query_params = None;
                    profile_obj.auth_path_prefix = None;
                } else if profile_obj
                    .auth_headers
                    .as_ref()
                    .is_some_and(|headers| !headers.is_empty())
                {
                    crate::auth::validate_auth_headers(
                        profile_obj.auth_headers.as_ref().expect("checked is_some"),
                    )
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                }
                if profile_obj
                    .auth_query_params
                    .as_ref()
                    .is_some_and(|params| !params.is_empty())
                {
                    crate::auth::validate_auth_query_params(
                        profile_obj
                            .auth_query_params
                            .as_ref()
                            .expect("checked is_some"),
                    )
                    .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
                }
                if let Some(prefix) = profile_obj.auth_path_prefix.as_ref() {
                    profile_obj.auth_path_prefix = Some(
                        crate::auth::validate_auth_path_prefix_template(prefix)
                            .map_err(|e| UxcError::InvalidArguments(e.to_string()))?,
                    );
                }
                if resolved_auth_type != AuthType::Bearer {
                    profile_obj.bootstrap = None;
                    profile_obj.bootstrap_state = None;
                }
            }

            if let Some(desc) = description {
                profile_obj.description = Some(desc.clone());
            } else if profile_obj.description.is_none() {
                profile_obj.description = None;
            }

            profiles.set_profile(credential_id.clone(), profile_obj)?;
            profiles.save_profiles()?;
            let profile_data = profiles.get_profile(credential_id)?;
            let view = to_auth_profile_view(credential_id, profile_data);
            let data = serde_json::to_value(view)?;
            Ok(OutputEnvelope::success(
                "auth_set_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthCredentialCommands::Remove { credential_id } => {
            let mut profiles = Profiles::load_profiles()?;

            if !profiles.has_profile(credential_id) {
                return Err(UxcError::InvalidArguments(format!(
                    "Credential '{}' not found. Available credentials: {}",
                    credential_id,
                    profiles.list_names()
                ))
                .into());
            }

            profiles.remove_profile(credential_id)?;
            profiles.save_profiles()?;
            let data = serde_json::to_value(AuthRemoveData {
                credential: credential_id.clone(),
            })?;
            Ok(OutputEnvelope::success(
                "auth_remove_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
    }
}

fn handle_auth_binding_command(command: &AuthBindingCommands) -> Result<OutputEnvelope> {
    match command {
        AuthBindingCommands::List => {
            let mut bindings = AuthBindings::load_bindings()?;
            bindings.bindings.sort_by(|a, b| a.id.cmp(&b.id));
            let data = serde_json::to_value(AuthBindingListData {
                count: bindings.bindings.len(),
                bindings: bindings.bindings,
            })?;
            Ok(OutputEnvelope::success(
                "auth_binding_list",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        AuthBindingCommands::Add {
            id,
            host,
            path_prefix,
            scheme,
            credential,
            signer_json,
            priority,
            disabled,
        } => {
            let profiles = Profiles::load_profiles()?;
            if !profiles.has_profile(credential) {
                return Err(UxcError::InvalidArguments(format!(
                    "Credential '{}' not found. Available credentials: {}",
                    credential,
                    profiles.list_names()
                ))
                .into());
            }

            let mut bindings = AuthBindings::load_bindings()?;
            let signer = signer_json
                .as_ref()
                .map(|value| serde_json::from_str::<auth::AuthSignerConfig>(value))
                .transpose()
                .map_err(|e| {
                    UxcError::InvalidArguments(format!("Invalid --signer-json payload: {}", e))
                })?;
            bindings
                .add_binding(AuthBindingRule {
                    id: id.clone(),
                    host: host.clone(),
                    path_prefix: path_prefix.clone(),
                    scheme: scheme.clone(),
                    credential: credential.clone(),
                    signer: signer.clone(),
                    priority: *priority,
                    enabled: !disabled,
                })
                .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
            bindings.save_bindings()?;

            let data = serde_json::to_value(AuthBindingSetData {
                id: id.clone(),
                credential: credential.clone(),
                host: host.clone(),
                path_prefix: path_prefix.clone(),
                scheme: scheme.clone(),
                signer,
                priority: *priority,
                enabled: !disabled,
            })?;
            Ok(OutputEnvelope::success(
                "auth_binding_set_result",
                "cli",
                "uxc",
                Some(id),
                data,
                None,
            ))
        }
        AuthBindingCommands::Remove { binding_id } => {
            let mut bindings = AuthBindings::load_bindings()?;
            bindings
                .remove_binding(binding_id)
                .map_err(|e| UxcError::InvalidArguments(e.to_string()))?;
            bindings.save_bindings()?;
            let data = serde_json::to_value(AuthBindingRemoveData {
                binding_id: binding_id.clone(),
            })?;
            Ok(OutputEnvelope::success(
                "auth_binding_remove_result",
                "cli",
                "uxc",
                Some(binding_id),
                data,
                None,
            ))
        }
        AuthBindingCommands::Match { endpoint } => {
            let endpoint = normalize_endpoint_url(endpoint);
            if url::Url::parse(&endpoint).is_err() {
                return Err(UxcError::InvalidArguments(format!(
                    "Invalid endpoint URL '{}'. Use a URL/host like api.example.com/path or https://api.example.com/path",
                    endpoint
                ))
                .into());
            }
            let bindings = AuthBindings::load_bindings()?;
            let matched = bindings.matching_rule(&endpoint).cloned();
            let data = serde_json::to_value(AuthBindingMatchData {
                endpoint,
                matched: matched.is_some(),
                binding: matched,
            })?;
            Ok(OutputEnvelope::success(
                "auth_binding_match",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
    }
}

async fn handle_auth_oauth_command(command: &AuthOauthCommands) -> Result<OutputEnvelope> {
    match command {
        AuthOauthCommands::List => {
            let profiles = Profiles::load_profiles()?;
            let mut rendered = Vec::new();
            for name in profiles.profile_names() {
                let profile = profiles.get_profile(&name)?;
                if profile.auth_type == AuthType::OAuth {
                    rendered.push(to_auth_profile_view(&name, profile));
                }
            }
            let data = serde_json::to_value(AuthListData {
                count: rendered.len(),
                credentials: rendered,
            })?;
            Ok(OutputEnvelope::success(
                "auth_list",
                "cli",
                "uxc",
                None,
                data,
                None,
            ))
        }
        AuthOauthCommands::Start {
            credential_id,
            endpoint,
            scope,
            client_id,
            client_secret,
            redirect_uri,
            issuer,
            authorization_endpoint,
            token_endpoint,
            registration_endpoint,
            resource_metadata_url,
        } => {
            auth::oauth_sessions::purge_expired_sessions(current_unix_timestamp())?;
            let endpoint = normalize_endpoint_url(endpoint);
            let scopes = auth::oauth::resolve_oauth_scopes_for_endpoint(
                &endpoint,
                &auth::oauth::parse_scopes(scope),
            )?;
            let client = build_resilient_http_client(
                std::time::Duration::from_secs(30),
                "OAuth start command",
            )?;
            let discovery_overrides = build_oauth_discovery_overrides(
                issuer,
                authorization_endpoint,
                token_endpoint,
                &None,
                registration_endpoint,
                resource_metadata_url,
            );
            let prepared = auth::oauth::prepare_authorization_code_login(
                &endpoint,
                &client,
                credential_id,
                client_id.as_deref(),
                client_secret.as_deref(),
                &scopes,
                redirect_uri,
                &discovery_overrides,
            )
            .await?;
            auth::oauth_sessions::save_session(&prepared.session)?;

            let data = serde_json::to_value(AuthOAuthStartData {
                credential: credential_id.clone(),
                flow: "authorization_code".to_string(),
                session_id: prepared.session.session_id,
                authorization_url: prepared.authorization_url,
                redirect_uri: redirect_uri.clone(),
                expires_at: prepared.session.expires_at,
                scopes,
            })?;
            Ok(OutputEnvelope::success(
                "auth_oauth_start_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthOauthCommands::Complete {
            credential_id,
            session_id,
            authorization_response,
        } => {
            let client = build_resilient_http_client(
                std::time::Duration::from_secs(30),
                "OAuth complete command",
            )?;
            let session = auth::oauth_sessions::load_session(session_id).map_err(|err| {
                let path = auth::oauth_sessions::session_path(session_id)
                    .ok()
                    .map(|value| value.display().to_string())
                    .unwrap_or_else(|| session_id.clone());
                if err
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
                {
                    UxcError::OAuthSessionNotFound(format!(
                        "session '{}' was not found at {}",
                        session_id, path
                    ))
                    .into()
                } else {
                    err
                }
            })?;

            if session.credential_id != *credential_id {
                return Err(UxcError::InvalidArguments(format!(
                    "OAuth session '{}' belongs to credential '{}', not '{}'",
                    session_id, session.credential_id, credential_id
                ))
                .into());
            }
            if session.is_expired(current_unix_timestamp()) {
                auth::oauth_sessions::remove_session(session_id)?;
                return Err(UxcError::OAuthSessionExpired(format!(
                    "session '{}' has expired",
                    session_id
                ))
                .into());
            }

            let completion = auth::oauth::finish_authorization_code_login(
                &client,
                &session,
                authorization_response,
            )
            .await;
            match completion {
                Ok(login) => {
                    auth::oauth_sessions::remove_session(session_id)?;
                    persist_oauth_login(
                        credential_id,
                        OAuthFlow::AuthorizationCode,
                        login.login.metadata,
                        login.login.token,
                        Some(login.client_id),
                        login.client_secret,
                        session.scopes,
                    )
                }
                Err(err) => {
                    if err.should_remove_session() {
                        auth::oauth_sessions::remove_session(session_id)?;
                    }
                    Err(err.into_error())
                }
            }
        }
        AuthOauthCommands::Login {
            credential_id,
            endpoint,
            flow,
            scope,
            client_id,
            client_secret,
            redirect_uri,
            authorization_code,
            issuer,
            authorization_endpoint,
            token_endpoint,
            device_authorization_endpoint,
            registration_endpoint,
            resource_metadata_url,
        } => {
            let flow = parse_oauth_flow(flow)?;
            let endpoint = normalize_endpoint_url(endpoint);
            let scopes = auth::oauth::resolve_oauth_scopes_for_endpoint(
                &endpoint,
                &auth::oauth::parse_scopes(scope),
            )?;
            let client = build_resilient_http_client(
                std::time::Duration::from_secs(30),
                "OAuth login command",
            )?;
            let discovery_overrides = build_oauth_discovery_overrides(
                issuer,
                authorization_endpoint,
                token_endpoint,
                device_authorization_endpoint,
                registration_endpoint,
                resource_metadata_url,
            );

            let (metadata, token, resolved_client_id, resolved_client_secret) = match flow {
                OAuthFlow::DeviceCode => {
                    let client_id = client_id.clone().ok_or_else(|| {
                        UxcError::InvalidArguments(
                            "device_code flow requires --client-id".to_string(),
                        )
                    })?;
                    let login = auth::oauth::login_with_device_code(
                        &endpoint,
                        &client,
                        &client_id,
                        &scopes,
                        &discovery_overrides,
                    )
                    .await?;
                    (
                        login.metadata,
                        login.token,
                        Some(client_id),
                        client_secret.clone(),
                    )
                }
                OAuthFlow::AuthorizationCode => {
                    let redirect_uri = redirect_uri.clone().ok_or_else(|| {
                        UxcError::InvalidArguments(
                            "authorization_code flow requires --redirect-uri".to_string(),
                        )
                    })?;
                    let login = auth::oauth::login_with_authorization_code(
                        &endpoint,
                        &client,
                        credential_id,
                        client_id.as_deref(),
                        client_secret.as_deref(),
                        &scopes,
                        &redirect_uri,
                        authorization_code.clone(),
                        &discovery_overrides,
                    )
                    .await?;
                    (
                        login.login.metadata,
                        login.login.token,
                        Some(login.client_id),
                        login.client_secret,
                    )
                }
                OAuthFlow::ClientCredentials => {
                    let client_id = client_id.clone().ok_or_else(|| {
                        UxcError::InvalidArguments(
                            "client_credentials flow requires --client-id".to_string(),
                        )
                    })?;
                    let client_secret = client_secret.clone().ok_or_else(|| {
                        UxcError::InvalidArguments(
                            "client_credentials flow requires --client-secret".to_string(),
                        )
                    })?;
                    let login = auth::oauth::login_with_client_credentials(
                        &endpoint,
                        &client,
                        &client_id,
                        &client_secret,
                        &scopes,
                        &discovery_overrides,
                    )
                    .await?;
                    (
                        login.metadata,
                        login.token,
                        Some(client_id),
                        Some(client_secret),
                    )
                }
            };
            persist_oauth_login(
                credential_id,
                flow,
                metadata,
                token,
                resolved_client_id,
                resolved_client_secret,
                scopes,
            )
        }
        AuthOauthCommands::Refresh { credential_id } => {
            let client = build_resilient_http_client(
                std::time::Duration::from_secs(30),
                "OAuth refresh command",
            )?;
            let mut profiles = Profiles::load_profiles()?;
            let mut profile_data = profiles.get_profile(credential_id)?.clone();
            profile_data.name = Some(credential_id.clone());
            if profile_data.auth_type != AuthType::OAuth {
                return Err(UxcError::InvalidArguments(format!(
                    "Credential '{}' is not an oauth credential",
                    credential_id
                ))
                .into());
            }

            auth::oauth::refresh_oauth_profile(&mut profile_data, &client).await?;
            profiles.set_profile(credential_id.clone(), profile_data.clone())?;
            profiles.save_profiles()?;

            let data = serde_json::to_value(to_auth_profile_view(credential_id, &profile_data))?;
            Ok(OutputEnvelope::success(
                "auth_set_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthOauthCommands::Info { credential_id } => {
            let profiles = Profiles::load_profiles()?;
            let profile_data = profiles.get_profile(credential_id)?;
            let data = serde_json::to_value(to_auth_profile_view(credential_id, profile_data))?;
            Ok(OutputEnvelope::success(
                "auth_info",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
        AuthOauthCommands::Logout { credential_id } => {
            let mut profiles = Profiles::load_profiles()?;
            let mut profile_data = profiles.get_profile(credential_id)?.clone();
            profile_data.oauth = None;
            profile_data.api_key.clear();
            profile_data.auth_type = AuthType::OAuth;
            profiles.set_profile(credential_id.clone(), profile_data)?;
            profiles.save_profiles()?;

            let data = serde_json::to_value(AuthRemoveData {
                credential: credential_id.clone(),
            })?;
            Ok(OutputEnvelope::success(
                "auth_remove_result",
                "cli",
                "uxc",
                Some(credential_id),
                data,
                None,
            ))
        }
    }
}

fn build_oauth_discovery_overrides(
    issuer: &Option<String>,
    authorization_endpoint: &Option<String>,
    token_endpoint: &Option<String>,
    device_authorization_endpoint: &Option<String>,
    registration_endpoint: &Option<String>,
    resource_metadata_url: &Option<String>,
) -> auth::oauth::OAuthDiscoveryOverrides {
    auth::oauth::OAuthDiscoveryOverrides {
        issuer: issuer.clone(),
        authorization_endpoint: authorization_endpoint.clone(),
        token_endpoint: token_endpoint.clone(),
        device_authorization_endpoint: device_authorization_endpoint.clone(),
        registration_endpoint: registration_endpoint.clone(),
        resource_metadata_url: resource_metadata_url.clone(),
    }
}

fn persist_oauth_login(
    credential_id: &str,
    flow: OAuthFlow,
    metadata: auth::oauth::OAuthProviderMetadata,
    token: auth::oauth::OAuthTokenResponse,
    resolved_client_id: Option<String>,
    resolved_client_secret: Option<String>,
    scopes: Vec<String>,
) -> Result<OutputEnvelope> {
    let mut profiles = Profiles::load_profiles()?;
    let mut profile_obj = profiles
        .get_profile(credential_id)
        .cloned()
        .unwrap_or_else(|_| Profile::new(String::new(), AuthType::OAuth));
    profile_obj.name = Some(credential_id.to_string());
    auth::oauth::apply_token_to_profile(
        &mut profile_obj,
        flow,
        metadata,
        token,
        resolved_client_id,
        resolved_client_secret,
        scopes,
    );
    profiles.set_profile(credential_id.to_string(), profile_obj.clone())?;
    profiles.save_profiles()?;

    let data = serde_json::to_value(to_auth_profile_view(credential_id, &profile_obj))?;
    Ok(OutputEnvelope::success(
        "auth_set_result",
        "cli",
        "uxc",
        Some(credential_id),
        data,
        None,
    ))
}

fn current_unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn to_auth_profile_view(name: &str, profile: &Profile) -> AuthProfileView {
    let oauth = profile.oauth.as_ref().map(|oauth| AuthOAuthView {
        flow: oauth.oauth_flow.as_ref().map(|flow| match flow {
            OAuthFlow::DeviceCode => "device_code".to_string(),
            OAuthFlow::AuthorizationCode => "authorization_code".to_string(),
            OAuthFlow::ClientCredentials => "client_credentials".to_string(),
        }),
        provider_issuer: oauth.provider_issuer.clone(),
        resource_metadata_url: oauth.resource_metadata_url.clone(),
        scopes: oauth.scopes.clone(),
        expires_at: oauth.expires_at,
        has_refresh_token: oauth.refresh_token.is_some(),
    });

    let bootstrap = to_auth_bootstrap_view(profile);

    AuthProfileView {
        name: name.to_string(),
        auth_type: profile.auth_type.to_string(),
        api_key_masked: profile.mask_api_key(),
        secret_source: profile
            .secret_source
            .as_ref()
            .map(|source| AuthSecretSourceView {
                kind: source.kind().to_string(),
            }),
        fields: {
            let fields = profile
                .field_source_kinds()
                .into_iter()
                .map(|(field_name, source_kind)| AuthFieldView {
                    name: field_name,
                    source_kind,
                    value_masked: "***".to_string(),
                })
                .collect::<Vec<_>>();
            if fields.is_empty() {
                None
            } else {
                Some(fields)
            }
        },
        auth_headers: profile.auth_headers.as_ref().map(|headers| {
            headers
                .iter()
                .map(|header| AuthHeaderView {
                    name: header.name.clone(),
                    value_masked: "***".to_string(),
                })
                .collect()
        }),
        auth_query_params: profile.auth_query_params.as_ref().map(|params| {
            params
                .iter()
                .map(|param| AuthQueryParamView {
                    name: param.name.clone(),
                    value_masked: "***".to_string(),
                })
                .collect()
        }),
        auth_path_prefix: profile
            .auth_path_prefix
            .as_ref()
            .map(|_| AuthPathPrefixView {
                value_masked: "***".to_string(),
            }),
        description: profile.description.clone(),
        oauth,
        bootstrap,
    }
}

fn to_auth_bootstrap_view(profile: &Profile) -> Option<AuthBootstrapView> {
    let config = profile.bootstrap.as_ref()?;
    Some(AuthBootstrapView {
        token_endpoint: config.token_endpoint.clone(),
        request_json_masked: "***".to_string(),
        headers: if config.headers.is_empty() {
            None
        } else {
            Some(
                config
                    .headers
                    .iter()
                    .map(|header| AuthHeaderView {
                        name: header.name.clone(),
                        value_masked: "***".to_string(),
                    })
                    .collect(),
            )
        },
        access_token_pointer: config.access_token_pointer.clone(),
        expires_in_pointer: config.expires_in_pointer.clone(),
        token_type_pointer: config.token_type_pointer.clone(),
        success_code_pointer: config.success_code_pointer.clone(),
        success_code_value: config.success_code_value.clone(),
        refresh_skew_seconds: config.refresh_skew_seconds,
        token_present: profile
            .bootstrap_state
            .as_ref()
            .is_some_and(|state| !state.access_token.is_empty()),
        expires_at: profile
            .bootstrap_state
            .as_ref()
            .and_then(|state| state.expires_at),
    })
}

fn parse_oauth_flow(value: &str) -> Result<OAuthFlow> {
    match value.to_ascii_lowercase().as_str() {
        "device_code" => Ok(OAuthFlow::DeviceCode),
        "authorization_code" => Ok(OAuthFlow::AuthorizationCode),
        "client_credentials" => Ok(OAuthFlow::ClientCredentials),
        _ => Err(UxcError::InvalidArguments(format!(
            "Invalid oauth flow '{}'. Valid values: device_code, authorization_code, client_credentials",
            value
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_link_launcher, collect_daemon_idle_ttl, enrich_operation_detail_payload,
        infer_scheme_for_endpoint, link_target_path, normalize_endpoint_url, parse_arguments,
        resolve_home_dir, resolve_link_dir, shell_single_quote, validate_link_name, Cli,
    };
    use clap::Parser;
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn process_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn infer_scheme_for_public_host() {
        assert_eq!(
            normalize_endpoint_url("petstore3.swagger.io/api/v3"),
            "https://petstore3.swagger.io/api/v3"
        );
        assert_eq!(
            normalize_endpoint_url("petstore3.swagger.io"),
            "https://petstore3.swagger.io"
        );
    }

    #[test]
    fn infer_http_for_local_hosts() {
        assert_eq!(
            normalize_endpoint_url("localhost:8080/graphql"),
            "http://localhost:8080/graphql"
        );
        assert_eq!(
            normalize_endpoint_url("127.0.0.1:8080"),
            "http://127.0.0.1:8080"
        );
    }

    #[test]
    fn keep_explicit_or_non_http_targets_unchanged() {
        assert_eq!(
            normalize_endpoint_url("https://petstore3.swagger.io/api/v3"),
            "https://petstore3.swagger.io/api/v3"
        );
        assert_eq!(normalize_endpoint_url("mcp://server"), "mcp://server");
        assert_eq!(normalize_endpoint_url("post:/pet"), "post:/pet");
        assert_eq!(normalize_endpoint_url("query/viewer"), "query/viewer");
    }

    #[test]
    fn skip_ambiguous_host_port_without_path() {
        assert_eq!(infer_scheme_for_endpoint("grpcb.in:9000"), None);
    }

    #[test]
    fn validate_link_name_rejects_invalid_values() {
        assert!(validate_link_name("petcli").is_ok());
        assert!(validate_link_name("acme-petcli").is_ok());
        assert!(validate_link_name("acme_pet.cli").is_ok());
        assert!(validate_link_name("").is_err());
        assert!(validate_link_name(".").is_err());
        assert!(validate_link_name("..").is_err());
        assert!(validate_link_name("bad/name").is_err());
        assert!(validate_link_name("bad name").is_err());
    }

    #[test]
    fn shell_quote_wraps_values_safely() {
        assert_eq!(
            shell_single_quote("petstore3.swagger.io/api/v3"),
            "'petstore3.swagger.io/api/v3'"
        );
        assert_eq!(shell_single_quote(""), "''");
        assert_eq!(shell_single_quote("o'connor"), "'o'\"'\"'connor'");
    }

    #[test]
    fn resolve_link_dir_expands_home_shortcuts() {
        let home = resolve_home_dir().expect("home directory should exist in test environment");
        assert_eq!(resolve_link_dir(Some("~")).expect("~ should resolve"), home);
        assert_eq!(
            resolve_link_dir(Some("~/bin")).expect("~/bin should resolve"),
            home.join("bin")
        );
    }

    #[test]
    fn resolve_link_dir_uses_platform_default_when_unspecified() {
        let home = resolve_home_dir().expect("home directory should exist in test environment");
        #[cfg(windows)]
        assert_eq!(
            resolve_link_dir(None).expect("default dir should resolve"),
            home.join(".uxc").join("bin")
        );
        #[cfg(not(windows))]
        assert_eq!(
            resolve_link_dir(None).expect("default dir should resolve"),
            home.join(".local").join("bin")
        );
    }

    #[test]
    fn link_target_path_uses_platform_suffix() {
        let dir = Path::new("/tmp");
        #[cfg(windows)]
        {
            assert_eq!(link_target_path(dir, "petcli"), dir.join("petcli.cmd"));
            assert_eq!(link_target_path(dir, "petcli.cmd"), dir.join("petcli.cmd"));
        }
        #[cfg(not(windows))]
        {
            assert_eq!(link_target_path(dir, "petcli"), dir.join("petcli"));
        }
    }

    #[test]
    fn build_link_launcher_persists_daemon_idle_ttl() {
        let launcher = build_link_launcher(
            "board-webmcp-ui",
            "example.com",
            None,
            None,
            &[],
            &["~/.uxc/profile".to_string()],
            Some(0),
        );
        assert!(launcher.contains("UXC_DAEMON_IDLE_TTL"));
        assert!(launcher.contains("0"));
    }

    #[test]
    fn collect_daemon_idle_ttl_prefers_flag_over_env() {
        let _guard = process_lock().lock().unwrap();
        // SAFETY: tests serialize access to process env with a mutex.
        unsafe {
            std::env::set_var("UXC_DAEMON_IDLE_TTL", "15");
        }
        let cli = Cli::parse_from(["uxc", "--daemon-idle-ttl", "0", "example.com", "-h"]);
        assert_eq!(collect_daemon_idle_ttl(&cli).unwrap(), Some(0));
        // SAFETY: tests serialize access to process env with a mutex.
        unsafe {
            std::env::remove_var("UXC_DAEMON_IDLE_TTL");
        }
    }

    #[test]
    fn collect_daemon_idle_ttl_reads_env_when_flag_missing() {
        let _guard = process_lock().lock().unwrap();
        // SAFETY: tests serialize access to process env with a mutex.
        unsafe {
            std::env::set_var("UXC_DAEMON_IDLE_TTL", "42");
        }
        let cli = Cli::parse_from(["uxc", "example.com", "-h"]);
        assert_eq!(collect_daemon_idle_ttl(&cli).unwrap(), Some(42));
        // SAFETY: tests serialize access to process env with a mutex.
        unsafe {
            std::env::remove_var("UXC_DAEMON_IDLE_TTL");
        }
    }

    #[test]
    fn cli_parses_timeout_ms_as_request_option() {
        let cli = Cli::parse_from([
            "uxc",
            "--timeout-ms",
            "45000",
            "https://api.example.com",
            "get:/health",
        ]);
        assert_eq!(cli.timeout_ms, Some(45_000));
    }

    #[test]
    fn normalize_endpoint_url_absolutizes_relative_stdio_command_paths() {
        let _guard = process_lock().lock().unwrap();
        let cwd = std::env::current_dir().expect("current dir should resolve");
        let temp = tempdir().expect("tempdir should be created");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir should exist");
        let bin_path = bin_dir.join("uxc-test-mcp-stdio-server");
        std::fs::write(&bin_path, "#!/bin/sh\n").expect("dummy bin should be written");
        std::env::set_current_dir(temp.path()).expect("should switch cwd");

        let normalized = normalize_endpoint_url("bin/uxc-test-mcp-stdio-server ok");

        std::env::set_current_dir(cwd).expect("should restore cwd");
        assert!(normalized.contains(bin_path.to_string_lossy().as_ref()));
        assert!(normalized.ends_with(" ok"));
    }

    #[test]
    fn parse_arguments_supports_path_and_json_field_assignment() {
        let args = vec![
            "filter.status=active".to_string(),
            r#"tags:=["rust","cli"]"#.to_string(),
        ];
        let parsed = parse_arguments(args, None).expect("arguments should parse");
        assert_eq!(parsed["filter"]["status"], "active");
        assert_eq!(parsed["tags"][0], "rust");
        assert_eq!(parsed["tags"][1], "cli");
    }

    #[test]
    fn enrich_operation_detail_payload_adds_schema_aware_examples() {
        let detail = json!({
            "operation_id": "post:/upload",
            "display_name": "POST /upload",
            "parameters": [],
            "input_schema": {
                "kind": "openapi_request_body",
                "content": {
                    "multipart/form-data": {
                        "x-uxc-file-fields": ["file"],
                        "schema": {
                            "type": "object",
                            "properties": {
                                "caption": { "type": "string" },
                                "meta": {
                                    "type": "object",
                                    "properties": {
                                        "status": { "type": "string" }
                                    }
                                },
                                "tags": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "file": { "type": "string", "format": "binary" }
                            }
                        }
                    }
                }
            }
        });

        let enriched = enrich_operation_detail_payload(detail, "uxc <host>", "post:/upload");
        let examples = enriched
            .get("invocation_examples")
            .and_then(|v| v.as_array())
            .expect("invocation_examples should exist");
        let flattened = examples
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(flattened.contains("caption=value"));
        assert!(flattened.contains("meta.status=value"));
        assert!(flattened.contains("tags[0]=value"));
        assert!(flattened.contains("meta:='"));
        assert!(flattened.contains("file=@/abs/path/file.bin"));
    }
}
