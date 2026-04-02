use crate::auth::injected_env::InjectEnvSpec;
use crate::error::UxcError;
use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpImportSourceKind {
    Path,
    Cursor,
    ClaudeDesktop,
    Vscode,
    Codex,
}

impl McpImportSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Cursor => "cursor",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Vscode => "vscode",
            Self::Codex => "codex",
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

pub fn resolve_source(input: &str) -> Result<McpImportSourceResolution> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(UxcError::InvalidArguments("--from cannot be empty".to_string()).into());
    }

    let lower = trimmed.to_ascii_lowercase();
    let preset = match lower.as_str() {
        "cursor" => Some((McpImportSourceKind::Cursor, preset_paths_cursor())),
        "claude-desktop" => Some((McpImportSourceKind::ClaudeDesktop, preset_paths_claude())),
        "vscode" => Some((McpImportSourceKind::Vscode, preset_paths_vscode())),
        "codex" => Some((McpImportSourceKind::Codex, preset_paths_codex())),
        _ => None,
    };

    if let Some((kind, candidates)) = preset {
        if let Some(path) = candidates.iter().find(|path| path.exists()).cloned() {
            return Ok(McpImportSourceResolution {
                kind,
                input: trimmed.to_string(),
                resolved_path: path,
            });
        }
        let display = candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(UxcError::InvalidArguments(format!(
            "Could not resolve preset source '{}'. Tried: {}",
            trimmed, display
        ))
        .into());
    }

    let path = expand_user_path(trimmed);
    if !path.exists() {
        return Err(UxcError::InvalidArguments(format!(
            "Import source '{}' not found at {}",
            trimmed,
            path.display()
        ))
        .into());
    }
    Ok(McpImportSourceResolution {
        kind: McpImportSourceKind::Path,
        input: trimmed.to_string(),
        resolved_path: path,
    })
}

pub fn build_mcp_import_plan(
    source: &McpImportSourceResolution,
    server_filter: Option<&str>,
    prefix: Option<&str>,
) -> Result<McpImportPlan> {
    let raw = std::fs::read_to_string(&source.resolved_path)?;
    let extension = source
        .resolved_path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let root = parse_config_text(&raw, &extension, &source.resolved_path)?;

    let discovered = collect_mcp_server_entries(&root);
    let discovered_count = discovered.len();
    let normalized_prefix = prefix
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mut plans = Vec::new();
    for (name, value) in discovered {
        if let Some(filter) = server_filter {
            if name != filter {
                continue;
            }
        }
        plans.push(build_server_plan(
            &name,
            value,
            normalized_prefix.as_deref(),
        ));
    }

    if let Some(filter) = server_filter {
        if plans.is_empty() {
            return Err(UxcError::InvalidArguments(format!(
                "Server '{}' not found in import source",
                filter
            ))
            .into());
        }
    }

    plans.sort_by(|a, b| a.original_name.cmp(&b.original_name));
    Ok(McpImportPlan {
        source: source.clone(),
        discovered_count,
        servers: plans,
    })
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
        _ => {
            let value: Value = serde_json::from_str(raw).map_err(|err| {
                UxcError::InvalidArguments(format!(
                    "Failed to parse JSON source {}: {}",
                    path.display(),
                    err
                ))
            })?;
            Ok(value)
        }
    }
}

fn collect_mcp_server_entries(root: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if let Some(obj) = root.as_object() {
        for key in ["mcpServers", "servers", "mcp_servers"] {
            if let Some(servers) = obj.get(key).and_then(Value::as_object) {
                for (name, value) in servers {
                    out.insert(name.to_string(), value.clone());
                }
            }
        }
    }
    out
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

    let host = if let Some(command) = raw
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
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
        let host = build_stdio_command(command, &args);
        if let Some((var_name, env_key)) = detect_stdio_secret_env_mapping(&env_map) {
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
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        if let Some(endpoint) = endpoint.clone() {
            let headers = value_string_map(raw.get("headers"));
            let env_http_headers = value_string_map(raw.get("env_http_headers"));
            let merged_headers = merge_headers(headers, env_http_headers);

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
            } else if let Some(candidate) = detect_http_credential_candidate(&merged_headers) {
                credential = Some(candidate);
            } else if !merged_headers.is_empty() {
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

fn detect_stdio_secret_env_mapping(env_map: &BTreeMap<String, String>) -> Option<(String, String)> {
    let mut candidates = env_map
        .iter()
        .filter_map(|(name, value)| parse_env_placeholder(value).map(|key| (name.clone(), key)))
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
            let upper = name.to_ascii_uppercase();
            if upper.contains("TOKEN") || upper.contains("SECRET") || upper.contains("API_KEY") {
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

fn preset_paths_cursor() -> Vec<PathBuf> {
    vec![expand_user_path("~/.cursor/mcp.json")]
}

fn preset_paths_claude() -> Vec<PathBuf> {
    let mut paths = vec![expand_user_path(
        "~/Library/Application Support/Claude/claude_desktop_config.json",
    )];
    paths.push(expand_user_path(
        "~/.config/Claude/claude_desktop_config.json",
    ));
    if let Some(appdata) = std::env::var_os("APPDATA") {
        paths.push(
            PathBuf::from(appdata)
                .join("Claude")
                .join("claude_desktop_config.json"),
        );
    }
    unique_paths(paths)
}

fn preset_paths_vscode() -> Vec<PathBuf> {
    let mut paths = vec![expand_user_path(
        "~/Library/Application Support/Code/User/mcp.json",
    )];
    paths.push(expand_user_path("~/.config/Code/User/mcp.json"));
    if let Some(appdata) = std::env::var_os("APPDATA") {
        paths.push(
            PathBuf::from(appdata)
                .join("Code")
                .join("User")
                .join("mcp.json"),
        );
    }
    unique_paths(paths)
}

fn preset_paths_codex() -> Vec<PathBuf> {
    unique_paths(vec![
        expand_user_path("~/.codex/config.toml"),
        expand_user_path("~/.config/codex/config.toml"),
    ])
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
}
