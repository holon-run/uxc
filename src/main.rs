use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::IpAddr;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tracing::info;

mod adapters;
mod auth;
mod cache;
pub mod cli;
mod error;
mod output;
mod schema_mapping;

use adapters::{Adapter, DetectionOptions, Operation, OperationDetail, ProtocolDetector};
use auth::{AuthBindingRule, AuthBindings, AuthType, OAuthFlow, Profile, Profiles};
use cache::CacheConfig;
use error::UxcError;
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

#[derive(Parser)]
#[command(name = "uxc")]
#[command(about = "Universal X-Protocol Call", long_about = None)]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(disable_help_flag = true)]
#[command(disable_help_subcommand = true)]
struct Cli {
    /// Show help
    #[arg(short = 'h', long = "help", global = true)]
    help: bool,

    /// Explicit credential ID for this request (overrides endpoint binding auto-match)
    #[arg(long, global = true)]
    auth: Option<String>,

    /// Disable cache for this operation
    #[arg(long, global = true)]
    no_cache: bool,

    /// Cache TTL in seconds
    #[arg(long, global = true)]
    cache_ttl: Option<u64>,

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
    #[arg(value_name = "URL", global = true)]
    url: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List available operations
    List {
        /// Show detailed information (text mode only)
        #[arg(short, long)]
        verbose: bool,
    },

    /// Describe one operation in detail
    Describe {
        /// Operation ID (e.g., "get:/users/{id}", "query/user", "ask_question")
        #[arg(value_name = "OPERATION_ID")]
        operation_id: String,
    },

    /// Show endpoint help, or operation help when OPERATION_ID is provided
    Help {
        /// Optional operation ID
        #[arg(value_name = "OPERATION_ID")]
        operation_id: Option<String>,
    },

    /// Inspect endpoint/schema
    Inspect {
        /// Show full schema
        #[arg(short, long)]
        full: bool,
    },

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

        /// Overwrite existing shortcut file
        #[arg(long)]
        force: bool,
    },

    /// Execute an operation explicitly
    Call {
        /// Operation ID
        #[arg(value_name = "OPERATION_ID")]
        operation_id: String,

        /// Key-value arguments (e.g., "id=42")
        #[arg(short, long)]
        args: Vec<String>,

        /// JSON input payload
        #[arg(long)]
        json: Option<String>,
    },

    /// Dynamic operation execution: `uxc <url> <operation_id> [--json ...] [--args k=v]`
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Show cache statistics
    Stats,

    /// Clear cache entries
    Clear {
        /// Optional URL to clear specific cache entry
        url: Option<String>,

        /// Clear all cached entries
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Manage credentials
    Credential {
        #[command(subcommand)]
        credential_command: AuthCredentialCommands,
    },

    /// Manage endpoint auth bindings
    Binding {
        #[command(subcommand)]
        binding_command: AuthBindingCommands,
    },

    /// Manage OAuth credentials
    Oauth {
        #[command(subcommand)]
        oauth_command: AuthOauthCommands,
    },
}

#[derive(Subcommand)]
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
        #[arg(short = 't', long, default_value = "bearer")]
        auth_type: String,

        /// Literal secret value
        #[arg(long, conflicts_with = "secret_env")]
        secret: Option<String>,

        /// Environment variable key containing secret
        #[arg(long, conflicts_with = "secret")]
        secret_env: Option<String>,

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

#[derive(Subcommand)]
enum AuthOauthCommands {
    /// Login with OAuth and save tokens
    Login {
        /// Credential ID
        #[arg(value_name = "CREDENTIAL_ID")]
        credential_id: String,

        /// MCP HTTP endpoint URL
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

enum EndpointCommand {
    HostHelp,
    List {
        verbose: bool,
    },
    Describe {
        operation_id: String,
    },
    Inspect {
        full: bool,
    },
    Execute {
        operation_id: String,
        args: Vec<String>,
        json: Option<String>,
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
    next: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OperationListData {
    operations: Vec<OperationSummary>,
    count: usize,
    verbose: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlobalHelpData {
    name: String,
    about: String,
    usage: String,
    commands: Vec<GlobalHelpCommand>,
    notes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GlobalHelpCommand {
    name: String,
    about: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheClearData {
    scope: String,
    url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthProfileView {
    name: String,
    auth_type: String,
    api_key_masked: String,
    description: Option<String>,
    oauth: Option<AuthOAuthView>,
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
struct AuthListData {
    credentials: Vec<AuthProfileView>,
    count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthRemoveData {
    credential: String,
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
    priority: i32,
    enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthBindingRemoveData {
    binding_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct LinkCreateData {
    name: String,
    host: String,
    path: String,
    overwritten: bool,
    dir_in_path: bool,
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

    if let Err(err) = run(normalized_args).await {
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

    let code = error_code(err);
    let envelope = OutputEnvelope::error(code, &err.to_string());
    match envelope.to_json() {
        Ok(json) => println!("{}", json),
        Err(ser_err) => {
            eprintln!("failed to serialize error output: {}", ser_err);
            eprintln!("{}", err);
        }
    }
}

async fn run(args: Vec<String>) -> Result<()> {
    let cli = Cli::parse_from(args);
    let output_mode = resolve_output_mode(&cli);
    let envelope = execute_cli(&cli).await?;
    render_output(&envelope, output_mode)
}

fn resolve_output_mode(cli: &Cli) -> OutputMode {
    if cli.text || cli.format == Some(OutputFormat::Text) {
        OutputMode::Text
    } else if cli.help && cli.url.is_none() && cli.command.is_none() {
        // Preserve classic `uxc -h/--help` text UX.
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
        let is_global_bool = matches!(arg.as_str(), "--text" | "--no-cache");
        let is_global_kv = matches!(
            arg.as_str(),
            "--format" | "--auth" | "--cache-ttl" | "--schema-url"
        );
        let is_global_inline = arg.starts_with("--format=")
            || arg.starts_with("--auth=")
            || arg.starts_with("--cache-ttl=")
            || arg.starts_with("--schema-url=");

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

fn normalize_endpoint_url(input: &str) -> String {
    match infer_scheme_for_endpoint(input) {
        Some(scheme) => format!("{}://{}", scheme, input),
        None => input.to_string(),
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
    if should_show_global_help(cli) {
        return global_help_envelope();
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
        force,
    }) = &cli.command
    {
        return handle_link_command(name, host, dir.as_deref(), *force).await;
    }

    let url = cli
        .url
        .clone()
        .ok_or_else(|| UxcError::InvalidArguments("URL is required".to_string()))
        .map(|raw| normalize_endpoint_url(&raw))?;

    info!("UXC v{} - connecting to {}", env!("CARGO_PKG_VERSION"), url);

    let endpoint_command = resolve_endpoint_command(cli)?;
    let auth_profile = auth::resolve_auth_for_endpoint(&url, cli.auth.clone())?;
    let cache = cache::create_cache(cache_config)?;

    let detector = ProtocolDetector::new();
    let detection_options = DetectionOptions {
        schema_url: cli.schema_url.as_deref().map(normalize_endpoint_url),
        auth_profile: auth_profile.clone(),
    };
    let mut adapter = detector
        .detect_adapter_with_options(&url, &detection_options)
        .await?;
    adapter = inject_cache_if_supported(adapter, cache);
    adapter = inject_auth_if_supported(adapter, auth_profile);

    let envelope = match endpoint_command {
        EndpointCommand::HostHelp => {
            let start = std::time::Instant::now();
            let operations = adapter.list_operations(&url).await?;
            let protocol = adapter.protocol_type().as_str();
            let duration_ms = start.elapsed().as_millis() as u64;
            let summaries = operations
                .iter()
                .map(|op| to_operation_summary(protocol, op))
                .collect::<Vec<_>>();
            let data = serde_json::to_value(HostHelpData {
                count: summaries.len(),
                operations: summaries,
                next: vec![
                    "uxc <host> list".to_string(),
                    "uxc <host> describe <operation_id>".to_string(),
                    "uxc <host> call <operation_id> --json '{...}'".to_string(),
                ],
            })?;
            OutputEnvelope::success("host_help", protocol, &url, None, data, Some(duration_ms))
        }
        EndpointCommand::List { verbose } => {
            let start = std::time::Instant::now();
            let operations = adapter.list_operations(&url).await?;
            let protocol = adapter.protocol_type().as_str();
            let duration_ms = start.elapsed().as_millis() as u64;
            let summaries = operations
                .iter()
                .map(|op| to_operation_summary(protocol, op))
                .collect::<Vec<_>>();
            let data = serde_json::to_value(OperationListData {
                count: summaries.len(),
                operations: summaries,
                verbose,
            })?;
            OutputEnvelope::success(
                "operation_list",
                protocol,
                &url,
                None,
                data,
                Some(duration_ms),
            )
        }
        EndpointCommand::Describe { operation_id } => {
            let start = std::time::Instant::now();
            let detail = adapter.describe_operation(&url, &operation_id).await?;
            let protocol = adapter.protocol_type().as_str();
            let duration_ms = start.elapsed().as_millis() as u64;
            let data = serde_json::to_value(&detail)?;
            OutputEnvelope::success(
                "operation_detail",
                protocol,
                &url,
                Some(&detail.operation_id),
                data,
                Some(duration_ms),
            )
        }
        EndpointCommand::Inspect { full } => {
            let start = std::time::Instant::now();
            let protocol = adapter.protocol_type().as_str();
            let schema = if full {
                Some(adapter.fetch_schema(&url).await?)
            } else {
                None
            };
            let duration_ms = start.elapsed().as_millis() as u64;
            let data = json!({
                "protocol": protocol,
                "endpoint": url,
                "schema": schema,
            });
            OutputEnvelope::success(
                "inspect_result",
                protocol,
                &url,
                None,
                data,
                Some(duration_ms),
            )
        }
        EndpointCommand::Execute {
            operation_id,
            args,
            json,
        } => {
            let args_map = parse_arguments(args, json)?;
            let result = adapter.execute(&url, &operation_id, args_map).await?;
            let protocol = adapter.protocol_type().as_str();
            OutputEnvelope::success(
                "call_result",
                protocol,
                &url,
                Some(&operation_id),
                result.data,
                Some(result.metadata.duration_ms),
            )
        }
    };

    Ok(envelope)
}

fn should_show_global_help(cli: &Cli) -> bool {
    if cli.url.is_some() {
        return false;
    }

    matches!(
        cli.command,
        None | Some(Commands::Help { operation_id: None })
    )
}

fn print_global_help() -> Result<()> {
    let mut cmd = Cli::command();
    cmd.print_help()?;
    println!();
    Ok(())
}

fn global_help_envelope() -> Result<OutputEnvelope> {
    let data = serde_json::to_value(GlobalHelpData {
        name: "uxc".to_string(),
        about: "Universal X-Protocol Call".to_string(),
        usage: "uxc [OPTIONS] [URL] [COMMAND]".to_string(),
        commands: vec![
            GlobalHelpCommand {
                name: "list".to_string(),
                about: "List available operations".to_string(),
            },
            GlobalHelpCommand {
                name: "describe".to_string(),
                about: "Describe one operation in detail".to_string(),
            },
            GlobalHelpCommand {
                name: "help".to_string(),
                about: "Show endpoint help, or operation help with OPERATION_ID".to_string(),
            },
            GlobalHelpCommand {
                name: "inspect".to_string(),
                about: "Inspect endpoint/schema".to_string(),
            },
            GlobalHelpCommand {
                name: "cache".to_string(),
                about: "Manage schema cache".to_string(),
            },
            GlobalHelpCommand {
                name: "auth".to_string(),
                about: "Manage credentials, bindings, and OAuth".to_string(),
            },
            GlobalHelpCommand {
                name: "link".to_string(),
                about: "Create a host-bound shortcut command".to_string(),
            },
            GlobalHelpCommand {
                name: "call".to_string(),
                about: "Execute an operation explicitly".to_string(),
            },
        ],
        notes: vec![
            "Default output is JSON. Use --text for human-readable output.".to_string(),
            "Examples: uxc <host> help; uxc <host> <operation> help".to_string(),
        ],
    })?;

    Ok(OutputEnvelope::success(
        "global_help",
        "cli",
        "uxc",
        None,
        data,
        None,
    ))
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
        Some("global_help") => print_global_help(),
        Some("host_help") => {
            let endpoint = envelope.endpoint.as_deref().unwrap_or("unknown");
            let protocol = envelope.protocol.as_deref().unwrap_or("unknown");
            let data: HostHelpData = decode_envelope_data(envelope)?;
            print_host_help_text_from_summaries(protocol, endpoint, &data.operations, &data.next);
            Ok(())
        }
        Some("operation_list") => {
            let protocol = envelope.protocol.as_deref().unwrap_or("unknown");
            let data: OperationListData = decode_envelope_data(envelope)?;
            print_list_text_from_summaries(protocol, &data.operations, data.verbose);
            Ok(())
        }
        Some("operation_detail") => {
            let endpoint = envelope.endpoint.as_deref().unwrap_or("unknown");
            let protocol = envelope.protocol.as_deref().unwrap_or("unknown");
            let detail: OperationDetail = decode_envelope_data(envelope)?;
            print_detail_text(protocol, endpoint, &detail);
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
        Some("cache_clear_result") => {
            let data: CacheClearData = decode_envelope_data(envelope)?;
            if data.scope == "all" {
                println!("Cache cleared successfully.");
            } else if let Some(url) = data.url {
                println!("Cache entry cleared for: {}", url);
            } else {
                println!("Cache cleared.");
            }
            Ok(())
        }
        Some("auth_list") => {
            let data: AuthListData = decode_envelope_data(envelope)?;
            if data.credentials.is_empty() {
                println!("No credentials found.");
                println!("\nCreate one with: uxc auth credential set <id> --secret <value>");
                return Ok(());
            }

            println!("Credentials:\n");
            for credential in data.credentials {
                println!("  {}", credential.name);
                println!("    Type: {}", credential.auth_type);
                println!("    Secret: {}", credential.api_key_masked);
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
        Some(Commands::List { verbose }) => Ok(EndpointCommand::List { verbose: *verbose }),
        Some(Commands::Describe { operation_id }) => Ok(EndpointCommand::Describe {
            operation_id: operation_id.clone(),
        }),
        Some(Commands::Help {
            operation_id: Some(operation_id),
        }) => Ok(EndpointCommand::Describe {
            operation_id: operation_id.clone(),
        }),
        Some(Commands::Help { operation_id: None }) => Ok(EndpointCommand::HostHelp),
        Some(Commands::Inspect { full }) => Ok(EndpointCommand::Inspect { full: *full }),
        Some(Commands::Call {
            operation_id,
            args,
            json,
        }) => Ok(EndpointCommand::Execute {
            operation_id: operation_id.clone(),
            args: args.clone(),
            json: json.clone(),
        }),
        Some(Commands::External(tokens)) => parse_external_command(tokens, cli.help),
        Some(Commands::Cache { .. })
        | Some(Commands::Auth { .. })
        | Some(Commands::Link { .. }) => Err(UxcError::InvalidArguments(
            "Internal routing error for cache/auth/link command".to_string(),
        )
        .into()),
    }
}

fn parse_external_command(tokens: &[String], global_help: bool) -> Result<EndpointCommand> {
    if tokens.is_empty() {
        return Err(UxcError::InvalidArguments("Operation ID is required".to_string()).into());
    }

    let operation_id = tokens[0].clone();

    if global_help {
        return Ok(EndpointCommand::Describe { operation_id });
    }

    if tokens.len() >= 2 && tokens[1] == "help" {
        if tokens.len() > 2 {
            return Err(UxcError::InvalidArguments(
                "Unexpected arguments after '<operation_id> help'".to_string(),
            )
            .into());
        }
        return Ok(EndpointCommand::Describe { operation_id });
    }

    let mut args = Vec::new();
    let mut json_payload = None;
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
            "--json" => {
                idx += 1;
                let payload = tokens.get(idx).ok_or_else(|| {
                    UxcError::InvalidArguments("Missing value for --json".to_string())
                })?;
                json_payload = Some(payload.clone());
            }
            token if token.contains('=') && !token.starts_with('-') => {
                args.push(token.to_string());
            }
            unknown => {
                return Err(UxcError::InvalidArguments(format!(
                    "Unknown argument '{}' for operation '{}'. Use --json or --args",
                    unknown, operation_id
                ))
                .into());
            }
        }

        idx += 1;
    }

    Ok(EndpointCommand::Execute {
        operation_id,
        args,
        json: json_payload,
    })
}

fn parse_arguments(
    args: Vec<String>,
    json_payload: Option<String>,
) -> Result<HashMap<String, Value>> {
    let mut args_map = HashMap::new();

    if let Some(json_str) = json_payload {
        let value: Value = serde_json::from_str(&json_str)
            .map_err(|e| UxcError::InvalidArguments(format!("Invalid JSON payload: {}", e)))?;
        if let Some(obj) = value.as_object() {
            for (k, v) in obj {
                args_map.insert(k.clone(), v.clone());
            }
        } else {
            return Err(
                UxcError::InvalidArguments("JSON payload must be an object".to_string()).into(),
            );
        }
    } else {
        for arg in args {
            let parts: Vec<&str> = arg.splitn(2, '=').collect();
            if parts.len() == 2 {
                args_map.insert(parts[0].to_string(), json!(parts[1]));
            }
        }
    }

    Ok(args_map)
}

fn print_json(envelope: &OutputEnvelope) -> Result<()> {
    println!("{}", envelope.to_json()?);
    Ok(())
}

fn print_host_help_text_from_summaries(
    protocol: &str,
    endpoint: &str,
    operations: &[OperationSummary],
    next: &[String],
) {
    println!("Protocol: {}", protocol);
    println!("Endpoint: {}", endpoint);
    println!();
    println!("Available operations:");
    for op in operations {
        if let Some(desc) = &op.summary {
            println!("- {} ({}) : {}", op.display_name, op.operation_id, desc);
        } else {
            println!("- {} ({})", op.display_name, op.operation_id);
        }
    }

    if !next.is_empty() {
        println!();
        println!("Next steps:");
        for line in next {
            println!("  {}", line);
        }
    }
}

fn print_list_text_from_summaries(protocol: &str, operations: &[OperationSummary], verbose: bool) {
    if verbose {
        println!("Detected protocol: {}", protocol);
        println!();
    }
    for op in operations {
        println!("{} ({})", op.display_name, op.operation_id);
        if verbose {
            if let Some(desc) = &op.summary {
                println!("  {}", desc);
            }
            if !op.required.is_empty() {
                println!("  Required: {}", op.required.join(", "));
            }
        }
    }
}

fn print_detail_text(protocol: &str, endpoint: &str, detail: &OperationDetail) {
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
}

fn to_operation_summary(protocol: &str, op: &Operation) -> OperationSummary {
    let required = op
        .parameters
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.clone())
        .collect::<Vec<_>>();

    let protocol_kind = match protocol {
        "mcp" => "tool",
        "graphql" => {
            if op.operation_id.starts_with("query/") {
                "query"
            } else if op.operation_id.starts_with("mutation/") {
                "mutation"
            } else if op.operation_id.starts_with("subscription/") {
                "subscription"
            } else {
                "field"
            }
        }
        "grpc" => "rpc",
        "openapi" => "http_operation",
        "jsonrpc" => "rpc_method",
        _ => "operation",
    }
    .to_string();

    let input_shape_hint = if op.parameters.is_empty() {
        "none".to_string()
    } else {
        "object".to_string()
    };

    OperationSummary {
        operation_id: op.operation_id.clone(),
        display_name: op.display_name.clone(),
        summary: op.description.clone(),
        required,
        input_shape_hint,
        protocol_kind,
    }
}

async fn handle_link_command(
    name: &str,
    host: &str,
    dir: Option<&str>,
    force: bool,
) -> Result<OutputEnvelope> {
    validate_link_name(name)?;

    let host = host.trim();
    if host.is_empty() {
        return Err(UxcError::InvalidArguments("Host cannot be empty".to_string()).into());
    }

    let target_dir = resolve_link_dir(dir)?;
    fs::create_dir_all(&target_dir)?;

    let target_path = link_target_path(&target_dir, name);
    let launcher = build_link_launcher(host);
    let target_exists_before = target_path.exists();
    write_link_file(&target_path, launcher.as_bytes(), force)?;
    set_executable_if_unix(&target_path)?;

    let data = serde_json::to_value(LinkCreateData {
        name: name.to_string(),
        host: host.to_string(),
        path: target_path.display().to_string(),
        overwritten: target_exists_before,
        dir_in_path: is_dir_in_path(&target_dir),
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

fn build_link_launcher(host: &str) -> String {
    #[cfg(windows)]
    {
        let escaped = host.replace('"', "\"\"");
        return format!("@echo off\r\nuxc \"{}\" %*\r\n", escaped);
    }
    #[cfg(not(windows))]
    {
        format!(
            "#!/usr/bin/env sh\nexec uxc {} \"$@\"\n",
            shell_single_quote(host)
        )
    }
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
                UxcError::OAuthRefreshFailed(_) => "OAUTH_REFRESH_FAILED",
                UxcError::OAuthScopeInsufficient(_) => "OAUTH_SCOPE_INSUFFICIENT",
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
        CacheCommands::Clear { url, all } => {
            if *all {
                cache.clear()?;
                let data = serde_json::to_value(CacheClearData {
                    scope: "all".to_string(),
                    url: None,
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
                cache.invalidate(url)?;
                let data = serde_json::to_value(CacheClearData {
                    scope: "url".to_string(),
                    url: Some(url.clone()),
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
                    "Usage: uxc cache clear <url> OR uxc cache clear --all".to_string(),
                )
                .into())
            }
        }
    }
}

async fn handle_auth_command(command: &AuthCommands) -> Result<OutputEnvelope> {
    match command {
        AuthCommands::Credential { credential_command } => {
            handle_auth_credential_command(credential_command).await
        }
        AuthCommands::Binding { binding_command } => handle_auth_binding_command(binding_command),
        AuthCommands::Oauth { oauth_command } => handle_auth_oauth_command(oauth_command).await,
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
            description,
        } => {
            let auth_type = auth_type
                .parse::<AuthType>()
                .map_err(|e| anyhow::anyhow!("Invalid auth type: {}", e))?;

            if auth_type != AuthType::OAuth && secret.is_none() && secret_env.is_none() {
                return Err(UxcError::InvalidArguments(
                    "Credential set requires either --secret or --secret-env".to_string(),
                )
                .into());
            }

            let mut profile_obj = match (&secret, &secret_env) {
                (Some(value), None) => Profile::new(value.clone(), auth_type.clone()),
                (None, Some(env_key)) => {
                    Profile::new(String::new(), auth_type.clone()).with_secret_env(env_key.clone())
                }
                (None, None) => Profile::new(String::new(), auth_type.clone()),
                _ => {
                    return Err(UxcError::InvalidArguments(
                        "Use only one of --secret or --secret-env".to_string(),
                    )
                    .into());
                }
            };
            if let Some(desc) = description {
                profile_obj = profile_obj.with_description(desc.clone());
            }

            let mut profiles = Profiles::load_profiles()?;
            profiles.set_profile(credential_id.clone(), profile_obj)?;
            profiles.save_profiles()?;
            let profile_data = profiles.get_profile(credential_id)?;
            let view = AuthProfileView {
                name: credential_id.clone(),
                auth_type: auth_type.to_string(),
                api_key_masked: profile_data.mask_api_key(),
                description: description.clone(),
                oauth: None,
            };
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
            bindings
                .add_binding(AuthBindingRule {
                    id: id.clone(),
                    host: host.clone(),
                    path_prefix: path_prefix.clone(),
                    scheme: scheme.clone(),
                    credential: credential.clone(),
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
            if url::Url::parse(endpoint).is_err() {
                return Err(UxcError::InvalidArguments(format!(
                    "Invalid endpoint URL '{}'. Use a full URL such as https://api.example.com/path",
                    endpoint
                ))
                .into());
            }
            let bindings = AuthBindings::load_bindings()?;
            let matched = bindings.matching_rule(endpoint).cloned();
            let data = serde_json::to_value(AuthBindingMatchData {
                endpoint: endpoint.clone(),
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
        AuthOauthCommands::Login {
            credential_id,
            endpoint,
            flow,
            scope,
            client_id,
            client_secret,
            redirect_uri,
            authorization_code,
        } => {
            let flow = parse_oauth_flow(flow)?;
            let scopes = auth::oauth::parse_scopes(scope);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?;

            let (metadata, token, resolved_client_id, resolved_client_secret) = match flow {
                OAuthFlow::DeviceCode => {
                    let client_id = client_id.clone().ok_or_else(|| {
                        UxcError::InvalidArguments(
                            "device_code flow requires --client-id".to_string(),
                        )
                    })?;
                    let login =
                        auth::oauth::login_with_device_code(endpoint, &client, &client_id, &scopes)
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
                        endpoint,
                        &client,
                        client_id.as_deref(),
                        client_secret.as_deref(),
                        &scopes,
                        &redirect_uri,
                        authorization_code.clone(),
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
                        endpoint,
                        &client,
                        &client_id,
                        &client_secret,
                        &scopes,
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

            let mut profiles = Profiles::load_profiles()?;
            let mut profile_obj = profiles
                .get_profile(credential_id)
                .cloned()
                .unwrap_or_else(|_| Profile::new(String::new(), AuthType::OAuth));
            profile_obj.name = Some(credential_id.clone());
            auth::oauth::apply_token_to_profile(
                &mut profile_obj,
                flow,
                metadata,
                token,
                resolved_client_id,
                resolved_client_secret,
                scopes,
            );
            profiles.set_profile(credential_id.clone(), profile_obj.clone())?;
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
        AuthOauthCommands::Refresh { credential_id } => {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?;
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

    AuthProfileView {
        name: name.to_string(),
        auth_type: profile.auth_type.to_string(),
        api_key_masked: profile.mask_api_key(),
        description: profile.description.clone(),
        oauth,
    }
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

fn inject_cache_if_supported(
    adapter: adapters::AdapterEnum,
    cache: std::sync::Arc<dyn cache::Cache>,
) -> adapters::AdapterEnum {
    match adapter {
        adapters::AdapterEnum::OpenAPI(a) => adapters::AdapterEnum::OpenAPI(a.with_cache(cache)),
        adapters::AdapterEnum::GraphQL(a) => adapters::AdapterEnum::GraphQL(a.with_cache(cache)),
        adapters::AdapterEnum::GRpc(a) => adapters::AdapterEnum::GRpc(a.with_cache(cache)),
        adapters::AdapterEnum::JsonRpc(a) => adapters::AdapterEnum::JsonRpc(a.with_cache(cache)),
        adapters::AdapterEnum::Mcp(a) => adapters::AdapterEnum::Mcp(a.with_cache(cache)),
    }
}

fn inject_auth_if_supported(
    adapter: adapters::AdapterEnum,
    profile: Option<Profile>,
) -> adapters::AdapterEnum {
    match profile {
        Some(profile) => match adapter {
            adapters::AdapterEnum::OpenAPI(a) => {
                adapters::AdapterEnum::OpenAPI(a.with_auth(profile))
            }
            adapters::AdapterEnum::GraphQL(a) => {
                adapters::AdapterEnum::GraphQL(a.with_auth(profile))
            }
            adapters::AdapterEnum::GRpc(a) => adapters::AdapterEnum::GRpc(a.with_auth(profile)),
            adapters::AdapterEnum::JsonRpc(a) => {
                adapters::AdapterEnum::JsonRpc(a.with_auth(profile))
            }
            adapters::AdapterEnum::Mcp(a) => adapters::AdapterEnum::Mcp(a.with_auth(profile)),
        },
        None => adapter,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        infer_scheme_for_endpoint, link_target_path, normalize_endpoint_url, resolve_home_dir,
        resolve_link_dir, shell_single_quote, validate_link_name,
    };
    use std::path::Path;

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
}
