use crate::auth::injected_env::InjectEnvSpec;
use crate::error::UxcError;
use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpImportSourceKind {
    Auto,
    Path,
    Cursor,
    ClaudeCode,
    ClaudeDesktop,
    Vscode,
    Codex,
    Windsurf,
    OpenCode,
}

impl McpImportSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Path => "path",
            Self::Cursor => "cursor",
            Self::ClaudeCode => "claude-code",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Vscode => "vscode",
            Self::Codex => "codex",
            Self::Windsurf => "windsurf",
            Self::OpenCode => "opencode",
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpImportSourceResolution {
    pub kind: McpImportSourceKind,
    pub input: String,
    pub resolved_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSecretSource {
    Env { key: String },
    Literal { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialKind {
    Bearer,
    ApiKeyHeader { header_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialCandidate {
    pub kind: CredentialKind,
    pub secret: CredentialSecretSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingCandidate {
    pub host: String,
    pub scheme: Option<String>,
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpServerImportPlan {
    pub original_name: String,
    pub recommended_link_name: String,
    pub transport: String,
    pub host: Option<String>,
    pub env_keys: Vec<String>,
    pub warnings: Vec<String>,
    pub inject_env: Vec<InjectEnvSpec>,
    pub credential: Option<CredentialCandidate>,
    pub binding: Option<BindingCandidate>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpImportPlan {
    pub source: McpImportSourceResolution,
    pub discovered_count: usize,
    pub servers: Vec<McpServerImportPlan>,
}

pub fn build_mcp_import_plan_from_input(
    input: &str,
    prefix: Option<&str>,
) -> Result<McpImportPlan> {
    let sources = resolve_sources(input)?;
    let normalized_prefix = prefix
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mut discovered_count = 0usize;
    let mut merged = Vec::new();
    let mut dedupe_hosts = BTreeSet::new();

    for source in &sources {
        let root = parse_source_root(source)?;
        let discovered = collect_mcp_server_entries(&root, source);
        discovered_count += discovered.len();
        for (name, value) in discovered {
            let plan = build_server_plan(&name, value, normalized_prefix.as_deref());
            if let Some(host) = plan.host.as_ref().map(|h| h.trim().to_string()) {
                if !host.is_empty() && plan.error.is_none() && !dedupe_hosts.insert(host) {
                    continue;
                }
            }
            merged.push(plan);
        }
    }

    merged.sort_by(|a, b| a.original_name.cmp(&b.original_name));
    let source = source_for_plan(input, &sources);
    Ok(McpImportPlan {
        source,
        discovered_count,
        servers: merged,
    })
}

fn source_for_plan(
    input: &str,
    sources: &[McpImportSourceResolution],
) -> McpImportSourceResolution {
    if input.trim().eq_ignore_ascii_case("auto") {
        return McpImportSourceResolution {
            kind: McpImportSourceKind::Auto,
            input: "auto".to_string(),
            resolved_path: PathBuf::from("multiple"),
        };
    }
    sources
        .first()
        .cloned()
        .unwrap_or(McpImportSourceResolution {
            kind: McpImportSourceKind::Auto,
            input: "auto".to_string(),
            resolved_path: PathBuf::from("multiple"),
        })
}

fn resolve_sources(input: &str) -> Result<Vec<McpImportSourceResolution>> {
    let trimmed = input.trim();
    let normalized = if trimmed.is_empty() { "auto" } else { trimmed };
    let lower = normalized.to_ascii_lowercase();
    if lower == "auto" {
        let discovered = discover_auto_sources();
        if discovered.is_empty() {
            return Err(UxcError::InvalidArguments(
                "Could not discover any known MCP config source for auto mode".to_string(),
            )
            .into());
        }
        return Ok(discovered);
    }

    if let Some(kind) = source_kind_from_name(&lower) {
        let candidates = paths_for_source(kind.clone());
        if let Some(path) = candidates.iter().find(|path| path.exists()).cloned() {
            return Ok(vec![McpImportSourceResolution {
                kind,
                input: normalized.to_string(),
                resolved_path: path,
            }]);
        }
        let display = candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(UxcError::InvalidArguments(format!(
            "Could not resolve preset source '{}'. Tried: {}",
            normalized, display
        ))
        .into());
    }

    let path = expand_user_path(normalized);
    if !path.exists() {
        return Err(UxcError::InvalidArguments(format!(
            "Import source '{}' not found at {}",
            normalized,
            path.display()
        ))
        .into());
    }
    Ok(vec![McpImportSourceResolution {
        kind: McpImportSourceKind::Path,
        input: normalized.to_string(),
        resolved_path: path,
    }])
}

fn source_kind_from_name(value: &str) -> Option<McpImportSourceKind> {
    match value {
        "cursor" => Some(McpImportSourceKind::Cursor),
        "claude-code" => Some(McpImportSourceKind::ClaudeCode),
        "claude-desktop" => Some(McpImportSourceKind::ClaudeDesktop),
        "vscode" => Some(McpImportSourceKind::Vscode),
        "codex" => Some(McpImportSourceKind::Codex),
        "windsurf" => Some(McpImportSourceKind::Windsurf),
        "opencode" => Some(McpImportSourceKind::OpenCode),
        _ => None,
    }
}

fn discover_auto_sources() -> Vec<McpImportSourceResolution> {
    let ordered = [
        McpImportSourceKind::ClaudeCode,
        McpImportSourceKind::Cursor,
        McpImportSourceKind::Codex,
        McpImportSourceKind::Windsurf,
        McpImportSourceKind::OpenCode,
        McpImportSourceKind::ClaudeDesktop,
        McpImportSourceKind::Vscode,
    ];
    let mut out = Vec::new();
    for kind in ordered {
        for path in paths_for_source(kind.clone()) {
            if path.exists() {
                out.push(McpImportSourceResolution {
                    kind: kind.clone(),
                    input: kind.as_str().to_string(),
                    resolved_path: path,
                });
                break;
            }
        }
    }
    out
}

fn parse_source_root(source: &McpImportSourceResolution) -> Result<Value> {
    let raw = std::fs::read_to_string(&source.resolved_path)?;
    parse_config_text(
        &raw,
        source
            .resolved_path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        &source.resolved_path,
    )
}

fn parse_config_text(raw: &str, extension: &str, path: &Path) -> Result<Value> {
    match extension {
        "toml" => {
            let value: toml::Value = toml::from_str(raw).map_err(|err| {
                UxcError::InvalidArguments(format!(
                    "Failed to parse TOML source {}: {}",
                    path.display(),
                    err
                ))
            })?;
            Ok(serde_json::to_value(value)?)
        }
        "yml" | "yaml" => {
            let value: Value = serde_yaml::from_str(raw).map_err(|err| {
                UxcError::InvalidArguments(format!(
                    "Failed to parse YAML source {}: {}",
                    path.display(),
                    err
                ))
            })?;
            Ok(value)
        }
        _ => match serde_json::from_str::<Value>(raw) {
            Ok(value) => Ok(value),
            Err(json_err) => {
                let json5_value: Value = json5::from_str(raw).map_err(|json5_err| {
                    UxcError::InvalidArguments(format!(
                        "Failed to parse JSON/JSONC source {}: {}; {}",
                        path.display(),
                        json_err,
                        json5_err
                    ))
                })?;
                Ok(json5_value)
            }
        },
    }
}

fn collect_mcp_server_entries(
    root: &Value,
    source: &McpImportSourceResolution,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Some(obj) = root.as_object() else {
        return out;
    };
    let descriptor = source_container_descriptor(source);
    let mut found_container = false;

    if descriptor.allow_mcp_servers {
        if let Some(container) = obj.get("mcpServers").and_then(Value::as_object) {
            found_container = true;
            for (name, value) in container {
                out.insert(name.to_string(), value.clone());
            }
        }
    }
    if descriptor.allow_servers {
        if let Some(container) = obj.get("servers").and_then(Value::as_object) {
            found_container = true;
            for (name, value) in container {
                out.entry(name.to_string()).or_insert_with(|| value.clone());
            }
        }
    }
    if descriptor.allow_mcp {
        if let Some(container) = obj.get("mcp").and_then(Value::as_object) {
            found_container = true;
            for (name, value) in container {
                out.entry(name.to_string()).or_insert_with(|| value.clone());
            }
        }
    }
    if descriptor.allow_mcp_servers {
        if let Some(container) = obj.get("mcp_servers").and_then(Value::as_object) {
            found_container = true;
            for (name, value) in container {
                out.entry(name.to_string()).or_insert_with(|| value.clone());
            }
        }
    }
    if descriptor.allow_root_fallback && !found_container {
        for (name, value) in obj {
            if value.as_object().is_some_and(looks_like_server_entry) {
                out.entry(name.to_string()).or_insert_with(|| value.clone());
            }
        }
    }
    out
}

struct SourceContainerDescriptor {
    allow_mcp_servers: bool,
    allow_servers: bool,
    allow_mcp: bool,
    allow_root_fallback: bool,
}

fn source_container_descriptor(source: &McpImportSourceResolution) -> SourceContainerDescriptor {
    match source.kind {
        McpImportSourceKind::OpenCode => SourceContainerDescriptor {
            allow_mcp_servers: false,
            allow_servers: false,
            allow_mcp: true,
            allow_root_fallback: false,
        },
        McpImportSourceKind::ClaudeCode => {
            let path_text = source.resolved_path.to_string_lossy().replace('\\', "/");
            let allow_root_fallback =
                path_text.ends_with(".claude.json") || path_text.ends_with(".claude/mcp.json");
            SourceContainerDescriptor {
                allow_mcp_servers: true,
                allow_servers: true,
                allow_mcp: true,
                allow_root_fallback,
            }
        }
        _ => SourceContainerDescriptor {
            allow_mcp_servers: true,
            allow_servers: true,
            allow_mcp: true,
            allow_root_fallback: true,
        },
    }
}

fn looks_like_server_entry(value: &serde_json::Map<String, Value>) -> bool {
    value
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
        || value
            .get("url")
            .or_else(|| value.get("endpoint"))
            .or_else(|| value.get("baseUrl"))
            .or_else(|| value.get("base_url"))
            .or_else(|| value.get("serverUrl"))
            .or_else(|| value.get("server_url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .is_some()
}

fn build_server_plan(name: &str, raw: Value, prefix: Option<&str>) -> McpServerImportPlan {
    let mut warnings = Vec::new();
    let mut error = None;
    let recommended_link_name = recommended_link_name(name, prefix);

    let env_map = value_string_map(raw.get("env"));
    let env_keys = env_map.keys().cloned().collect::<Vec<_>>();
    let cwd = raw
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if cwd.is_some() {
        warnings
            .push("Source config contains cwd; current import does not persist cwd".to_string());
    }

    let mut credential = None;
    let mut inject_env = Vec::new();
    let mut binding = None;

    let command = raw
        .get("command")
        .or_else(|| raw.get("executable"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let command_from_array = raw
        .get("command")
        .and_then(Value::as_array)
        .and_then(|parts| {
            let text = parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        });

    let host = if let Some(command) = command.or(command_from_array) {
        let args = raw
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let host = build_stdio_command(&command, &args);
        if let Some((var_name, env_key)) =
            detect_stdio_secret_env_mapping(&env_map, &command, &args)
        {
            match InjectEnvSpec::new(&var_name, "{{secret}}") {
                Ok(spec) => inject_env.push(spec),
                Err(err) => warnings.push(format!(
                    "Failed to derive inject-env for '{}': {}",
                    var_name, err
                )),
            }
            credential = Some(CredentialCandidate {
                kind: CredentialKind::Bearer,
                secret: CredentialSecretSource::Env { key: env_key },
            });
        } else if let Some((var_name, value)) = detect_stdio_secret_literal_mapping(&env_map) {
            match InjectEnvSpec::new(&var_name, "{{secret}}") {
                Ok(spec) => inject_env.push(spec),
                Err(err) => warnings.push(format!(
                    "Failed to derive inject-env for '{}': {}",
                    var_name, err
                )),
            }
            credential = Some(CredentialCandidate {
                kind: CredentialKind::Bearer,
                secret: CredentialSecretSource::Literal { value },
            });
        } else if !env_map.is_empty() {
            warnings.push(
                "No secret-like stdio env mapping detected; credential and --inject-env were not auto-derived"
                    .to_string(),
            );
        }
        Some(host)
    } else {
        let endpoint = raw
            .get("url")
            .or_else(|| raw.get("endpoint"))
            .or_else(|| raw.get("baseUrl"))
            .or_else(|| raw.get("base_url"))
            .or_else(|| raw.get("serverUrl"))
            .or_else(|| raw.get("server_url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        if let Some(endpoint) = endpoint.clone() {
            let mut headers = value_string_map(raw.get("headers"));
            let env_http_headers = value_string_map(raw.get("env_http_headers"));
            headers = merge_headers(headers, env_http_headers);
            if let Some(bearer) = raw
                .get("bearerToken")
                .or_else(|| raw.get("bearer_token"))
                .and_then(Value::as_str)
            {
                headers
                    .entry("Authorization".to_string())
                    .or_insert_with(|| format!("Bearer {}", bearer));
            }
            if let Some(bearer_env) = raw
                .get("bearerTokenEnv")
                .or_else(|| raw.get("bearer_token_env"))
                .and_then(Value::as_str)
            {
                headers
                    .entry("Authorization".to_string())
                    .or_insert_with(|| format!("${{{}}}", bearer_env));
            }

            if let Some(env_var) = raw
                .get("bearer_token_env_var")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                credential = Some(CredentialCandidate {
                    kind: CredentialKind::Bearer,
                    secret: CredentialSecretSource::Env {
                        key: env_var.to_string(),
                    },
                });
            } else if let Some(candidate) = detect_http_credential_candidate(&headers) {
                credential = Some(candidate);
            } else if !headers.is_empty() {
                warnings.push(
                    "Headers detected but no supported auth mapping found for auto credential import"
                        .to_string(),
                );
            }

            binding = binding_from_endpoint(&endpoint);
            Some(endpoint)
        } else {
            error = Some("Server config is missing both command and url/endpoint".to_string());
            None
        }
    };

    let transport = transport_label(&raw, host.as_deref());
    if host.is_none() && error.is_none() {
        error = Some("Server config could not be mapped into a UXC host".to_string());
    }

    McpServerImportPlan {
        original_name: name.to_string(),
        recommended_link_name,
        transport,
        host,
        env_keys,
        warnings,
        inject_env,
        credential,
        binding,
        error,
    }
}

fn merge_headers(
    mut headers: BTreeMap<String, String>,
    extra: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    for (name, value) in extra {
        headers.entry(name).or_insert(value);
    }
    headers
}

fn detect_http_credential_candidate(
    headers: &BTreeMap<String, String>,
) -> Option<CredentialCandidate> {
    let mut names = headers.keys().collect::<Vec<_>>();
    names.sort();
    for name in names {
        let value = headers.get(name)?;
        let lowered = name.to_ascii_lowercase();
        if lowered == "authorization" {
            if let Some(env_key) = parse_bearer_env_placeholder(value) {
                return Some(CredentialCandidate {
                    kind: CredentialKind::Bearer,
                    secret: CredentialSecretSource::Env { key: env_key },
                });
            }
            if let Some(literal) = parse_bearer_literal(value) {
                return Some(CredentialCandidate {
                    kind: CredentialKind::Bearer,
                    secret: CredentialSecretSource::Literal { value: literal },
                });
            }
        }
        if lowered.contains("api-key") || lowered.contains("apikey") || lowered == "x-api-key" {
            if let Some(env_key) = parse_env_placeholder(value) {
                return Some(CredentialCandidate {
                    kind: CredentialKind::ApiKeyHeader {
                        header_name: name.to_string(),
                    },
                    secret: CredentialSecretSource::Env { key: env_key },
                });
            }
            if !value.trim().is_empty() {
                return Some(CredentialCandidate {
                    kind: CredentialKind::ApiKeyHeader {
                        header_name: name.to_string(),
                    },
                    secret: CredentialSecretSource::Literal {
                        value: value.to_string(),
                    },
                });
            }
        }
    }
    None
}

fn detect_stdio_secret_env_mapping(
    env_map: &BTreeMap<String, String>,
    command: &str,
    args: &[String],
) -> Option<(String, String)> {
    let mut command_refs = Vec::with_capacity(args.len() + 1);
    command_refs.push(command.to_string());
    command_refs.extend(args.iter().cloned());
    let command_text = command_refs.join(" ");

    let mut candidates = env_map
        .iter()
        .filter_map(|(name, value)| {
            if !looks_secret_like_name(name) {
                return None;
            }
            let key = parse_env_placeholder(value)?;
            if key != *name && !env_var_is_referenced_in_text(&key, &command_text) {
                return None;
            }
            Some((name.clone(), key))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

fn detect_stdio_secret_literal_mapping(
    env_map: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    let mut candidates = env_map
        .iter()
        .filter_map(|(name, value)| {
            if value.trim().is_empty() {
                return None;
            }
            if looks_secret_like_name(name) {
                return Some((name.clone(), value.clone()));
            }
            None
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

fn binding_from_endpoint(endpoint: &str) -> Option<BindingCandidate> {
    let parsed = url::Url::parse(endpoint).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let scheme = Some(parsed.scheme().to_ascii_lowercase());
    let path_prefix = match parsed.path() {
        "" | "/" => None,
        value => Some(value.to_string()),
    };
    Some(BindingCandidate {
        host,
        scheme,
        path_prefix,
    })
}

fn transport_label(raw: &Value, host: Option<&str>) -> String {
    if host.is_some_and(is_http_url) {
        if let Some(value) = raw
            .get("transport")
            .or_else(|| raw.get("type"))
            .and_then(Value::as_str)
            .map(|s| s.to_ascii_lowercase())
        {
            if value.contains("sse") {
                return "sse".to_string();
            }
            if value.contains("streamable") || value.contains("http") {
                return "streamable_http".to_string();
            }
        }
        return "http".to_string();
    }
    "stdio".to_string()
}

fn is_http_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn parse_bearer_env_placeholder(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("bearer ") {
        return None;
    }
    parse_env_placeholder(trimmed[7..].trim())
}

fn parse_bearer_literal(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("bearer ") {
        return None;
    }
    let token = trimmed[7..].trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn parse_env_placeholder(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if let Some(inner) = trimmed.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
        return validate_env_key(inner.trim()).then(|| inner.trim().to_string());
    }
    if let Some(inner) = trimmed.strip_prefix("$") {
        return validate_env_key(inner.trim()).then(|| inner.trim().to_string());
    }
    if let Some(inner) = trimmed
        .strip_prefix("{{env:")
        .and_then(|v| v.strip_suffix("}}"))
    {
        let key = inner.trim();
        return validate_env_key(key).then(|| key.to_string());
    }
    None
}

fn looks_secret_like_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("API_KEY")
        || upper.contains("APIKEY")
        || upper.contains("ACCESS_KEY")
        || upper.contains("AUTH")
        || upper.contains("BEARER")
}

fn env_var_is_referenced_in_text(env_key: &str, text: &str) -> bool {
    text.contains(&format!("${{{}}}", env_key))
        || text.contains(&format!("${}", env_key))
        || text.contains(&format!("{{{{env:{}}}}}", env_key))
}

fn validate_env_key(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn value_string_map(raw: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(obj) = raw.and_then(Value::as_object) else {
        return out;
    };
    for (name, value) in obj {
        if let Some(v) = value.as_str() {
            out.insert(name.to_string(), v.to_string());
        }
    }
    out
}

fn build_stdio_command(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        return command.to_string();
    }
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_quote_part(command));
    for arg in args {
        parts.push(shell_quote_part(arg));
    }
    parts.join(" ")
}

fn shell_quote_part(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | ':' | '@'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn recommended_link_name(original: &str, prefix: Option<&str>) -> String {
    let base = sanitize_link_name(original);
    let mut candidate = base;
    if let Some(prefix) = prefix {
        let sanitized_prefix = sanitize_link_name(prefix);
        if !sanitized_prefix.is_empty() {
            candidate = format!("{}-{}", sanitized_prefix, candidate);
        }
    }
    if candidate.is_empty() {
        "mcp-link".to_string()
    } else {
        candidate
    }
}

fn sanitize_link_name(value: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in value.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if matches!(ch, '_' | '-' | '.') {
            ch
        } else {
            '-'
        };
        if normalized == '-' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
            out.push('-');
        } else {
            prev_dash = false;
            out.push(normalized);
        }
    }
    out = out.trim_matches('-').trim_matches('.').to_string();
    if out.is_empty() {
        return "mcp".to_string();
    }
    out
}

fn paths_for_source(kind: McpImportSourceKind) -> Vec<PathBuf> {
    let root_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match kind {
        McpImportSourceKind::Cursor => {
            let mut paths = vec![
                root_dir.join(".cursor").join("mcp.json"),
                expand_user_path("~/.cursor/mcp.json"),
            ];
            if let Some(home) = home_dir() {
                paths.push(home.join("Library/Application Support/Cursor/User/mcp.json"));
                paths.push(home.join("AppData/Roaming/Cursor/User/mcp.json"));
            }
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
                paths.push(PathBuf::from(xdg).join("Cursor/User/mcp.json"));
            }
            unique_paths(paths)
        }
        McpImportSourceKind::ClaudeCode => unique_paths(vec![
            root_dir.join(".claude/settings.local.json"),
            root_dir.join(".claude/settings.json"),
            root_dir.join(".claude/mcp.json"),
            expand_user_path("~/.claude/settings.local.json"),
            expand_user_path("~/.claude/settings.json"),
            expand_user_path("~/.claude/mcp.json"),
            expand_user_path("~/.claude.json"),
        ]),
        McpImportSourceKind::ClaudeDesktop => {
            let mut paths = vec![
                expand_user_path("~/Library/Application Support/Claude/settings.json"),
                expand_user_path("~/.config/Claude/settings.json"),
                expand_user_path("~/Library/Application Support/Claude/claude_desktop_config.json"),
                expand_user_path("~/.config/Claude/claude_desktop_config.json"),
            ];
            if let Some(appdata) = std::env::var_os("APPDATA") {
                paths.push(PathBuf::from(&appdata).join("Claude/settings.json"));
                paths.push(PathBuf::from(appdata).join("Claude/claude_desktop_config.json"));
            }
            unique_paths(paths)
        }
        McpImportSourceKind::Vscode => {
            let mut paths = vec![root_dir.join(".vscode/mcp.json")];
            if cfg!(target_os = "macos") {
                paths.push(expand_user_path(
                    "~/Library/Application Support/Code/User/mcp.json",
                ));
                paths.push(expand_user_path(
                    "~/Library/Application Support/Code - Insiders/User/mcp.json",
                ));
            } else if cfg!(windows) {
                if let Some(appdata) = std::env::var_os("APPDATA") {
                    let base = PathBuf::from(appdata);
                    paths.push(base.join("Code/User/mcp.json"));
                    paths.push(base.join("Code - Insiders/User/mcp.json"));
                }
            } else {
                paths.push(expand_user_path("~/.config/Code/User/mcp.json"));
                paths.push(expand_user_path("~/.config/Code - Insiders/User/mcp.json"));
            }
            unique_paths(paths)
        }
        McpImportSourceKind::Codex => unique_paths(vec![
            root_dir.join(".codex/config.toml"),
            expand_user_path("~/.codex/config.toml"),
            expand_user_path("~/.config/codex/config.toml"),
        ]),
        McpImportSourceKind::Windsurf => {
            let mut paths = vec![
                expand_user_path("~/.codeium/windsurf/mcp_config.json"),
                expand_user_path("~/.codeium/windsurf-next/mcp_config.json"),
                expand_user_path("~/.windsurf/mcp_config.json"),
                expand_user_path("~/.config/.codeium/windsurf/mcp_config.json"),
            ];
            if let Some(appdata) = std::env::var_os("APPDATA") {
                paths.push(PathBuf::from(appdata).join("Codeium/windsurf/mcp_config.json"));
            }
            unique_paths(paths)
        }
        McpImportSourceKind::OpenCode => {
            let mut paths = Vec::new();
            if let Some(value) = std::env::var_os("OPENCODE_CONFIG") {
                paths.push(PathBuf::from(value));
            }
            paths.push(root_dir.join("opencode.jsonc"));
            paths.push(root_dir.join("opencode.json"));
            if let Some(value) = std::env::var_os("OPENCODE_CONFIG_DIR") {
                let dir = PathBuf::from(value);
                paths.push(dir.join("opencode.jsonc"));
                paths.push(dir.join("opencode.json"));
            }
            paths.push(root_dir.join(".openai/config.json"));
            if let Some(value) = std::env::var_os("OPENAI_WORKDIR") {
                paths.push(PathBuf::from(value).join(".openai/config.json"));
            }
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
                let base = PathBuf::from(xdg);
                paths.push(base.join("openai/config.json"));
                paths.push(base.join("opencode/opencode.jsonc"));
                paths.push(base.join("opencode/opencode.json"));
            } else {
                paths.push(expand_user_path("~/.config/openai/config.json"));
                paths.push(expand_user_path("~/.config/opencode/opencode.jsonc"));
                paths.push(expand_user_path("~/.config/opencode/opencode.json"));
            }
            unique_paths(paths)
        }
        McpImportSourceKind::Auto | McpImportSourceKind::Path => Vec::new(),
    }
}

fn expand_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn unique_paths(input: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    input
        .into_iter()
        .filter(|path| seen.insert(path.display().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_placeholder_supports_common_forms() {
        assert_eq!(
            parse_env_placeholder("${THEGRAPH_API_KEY}"),
            Some("THEGRAPH_API_KEY".to_string())
        );
        assert_eq!(
            parse_env_placeholder("$THEGRAPH_API_KEY"),
            Some("THEGRAPH_API_KEY".to_string())
        );
        assert_eq!(
            parse_env_placeholder("{{env:THEGRAPH_API_KEY}}"),
            Some("THEGRAPH_API_KEY".to_string())
        );
    }

    #[test]
    fn parse_bearer_env_placeholder_supports_authorization() {
        assert_eq!(
            parse_bearer_env_placeholder("Bearer ${TOKEN}"),
            Some("TOKEN".to_string())
        );
        assert_eq!(
            parse_bearer_literal("Bearer plain-token"),
            Some("plain-token".to_string())
        );
    }

    #[test]
    fn sanitize_link_name_keeps_allowed_chars() {
        assert_eq!(sanitize_link_name("My MCP/Server"), "my-mcp-server");
        assert_eq!(sanitize_link_name("a_b.c-1"), "a_b.c-1");
    }

    #[test]
    fn stdio_env_derivation_ignores_non_secret_like_env_keys() {
        let mut env_map = BTreeMap::new();
        env_map.insert("NODE_OPTIONS".to_string(), "${NODE_OPTIONS}".to_string());
        env_map.insert("MY_API_KEY".to_string(), "${MY_API_KEY}".to_string());
        assert_eq!(
            detect_stdio_secret_env_mapping(&env_map, "npx", &[String::from("server.js")]),
            Some(("MY_API_KEY".to_string(), "MY_API_KEY".to_string()))
        );
    }

    #[test]
    fn stdio_env_derivation_requires_command_reference_or_same_name() {
        let mut env_map = BTreeMap::new();
        env_map.insert("API_KEY".to_string(), "${REAL_SECRET}".to_string());
        assert_eq!(
            detect_stdio_secret_env_mapping(
                &env_map,
                "node",
                &[
                    String::from("server.js"),
                    String::from("--mode"),
                    String::from("prod")
                ]
            ),
            None
        );
        assert_eq!(
            detect_stdio_secret_env_mapping(
                &env_map,
                "node",
                &[
                    String::from("server.js"),
                    String::from("--token"),
                    String::from("${REAL_SECRET}")
                ]
            ),
            Some(("API_KEY".to_string(), "REAL_SECRET".to_string()))
        );
    }

    #[test]
    fn source_kind_parser_supports_new_presets() {
        assert_eq!(
            source_kind_from_name("claude-code"),
            Some(McpImportSourceKind::ClaudeCode)
        );
        assert_eq!(
            source_kind_from_name("windsurf"),
            Some(McpImportSourceKind::Windsurf)
        );
        assert_eq!(
            source_kind_from_name("opencode"),
            Some(McpImportSourceKind::OpenCode)
        );
    }

    #[test]
    fn opencode_descriptor_only_allows_mcp_container() {
        let source = McpImportSourceResolution {
            kind: McpImportSourceKind::OpenCode,
            input: "opencode".to_string(),
            resolved_path: PathBuf::from("opencode.jsonc"),
        };
        let descriptor = source_container_descriptor(&source);
        assert!(!descriptor.allow_mcp_servers);
        assert!(!descriptor.allow_servers);
        assert!(descriptor.allow_mcp);
        assert!(!descriptor.allow_root_fallback);
    }
}
