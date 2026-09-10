//! Schema mapping for services whose runtime endpoint and schema URL differ.

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use tracing::warn;
use url::Url;

const DEFAULT_MAPPINGS_DIR: &str = ".uxc";
const DEFAULT_MAPPINGS_FILE: &str = "schema_mappings.json";
const SCHEMA_MAPPINGS_ENV: &str = "UXC_SCHEMA_MAPPINGS_FILE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingSource {
    Builtin,
    UserConfig,
}

impl MappingSource {
    fn rank(&self) -> i32 {
        match self {
            Self::Builtin => 0,
            Self::UserConfig => 1,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin_mapping",
            Self::UserConfig => "user_mapping",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSchemaMapping {
    pub schema_url: String,
    pub source: MappingSource,
}

#[derive(Debug, Clone, Deserialize)]
struct SchemaMappingsConfig {
    #[allow(dead_code)]
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    openapi: Vec<OpenApiMappingRule>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenApiMappingRule {
    host: String,
    #[serde(default)]
    path_prefix: Option<String>,
    schema_url: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    priority: i32,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
struct CandidateRule {
    source: MappingSource,
    rule: OpenApiMappingRule,
}

impl OpenApiMappingRule {
    fn normalized_host(&self) -> String {
        self.host.trim().to_ascii_lowercase()
    }

    fn normalized_path_prefix(&self) -> Option<String> {
        self.path_prefix
            .as_ref()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|prefix| {
                if prefix.starts_with('/') {
                    prefix.to_string()
                } else {
                    format!("/{}", prefix)
                }
            })
    }

    fn matches(&self, target: &Url) -> bool {
        let Some(host) = target.host_str() else {
            return false;
        };

        if self.normalized_host() != host.to_ascii_lowercase() {
            return false;
        }

        if let Some(prefix) = self.normalized_path_prefix() {
            target.path().starts_with(&prefix)
        } else {
            true
        }
    }
}

fn builtin_openapi_rules() -> Vec<OpenApiMappingRule> {
    vec![
        OpenApiMappingRule {
            host: "api.github.com".to_string(),
            path_prefix: Some("/".to_string()),
            schema_url: "https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json".to_string(),
            enabled: true,
            priority: 1000,
        },
        // Telegram Bot API has no official auto-discoverable OpenAPI document.
        // Map it to the curated schema shipped in this repository (#206) so
        // protocol detection succeeds without per-caller schema_url overrides (#460).
        OpenApiMappingRule {
            host: "api.telegram.org".to_string(),
            path_prefix: Some("/".to_string()),
            schema_url: "https://raw.githubusercontent.com/holon-run/uxc/main/skills/telegram-openapi-skill/references/telegram-bot.openapi.json".to_string(),
            enabled: true,
            priority: 1000,
        },
    ]
}

fn resolve_user_mappings_path() -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os(SCHEMA_MAPPINGS_ENV) {
        return Some(PathBuf::from(override_path));
    }

    home_dir().map(|home| home.join(DEFAULT_MAPPINGS_DIR).join(DEFAULT_MAPPINGS_FILE))
}

fn load_user_openapi_rules() -> Vec<OpenApiMappingRule> {
    let Some(path) = resolve_user_mappings_path() else {
        return Vec::new();
    };

    if !path.exists() {
        return Vec::new();
    }

    let raw = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(err) => {
            warn!("Failed to read schema mappings file {:?}: {}", path, err);
            return Vec::new();
        }
    };

    let parsed: SchemaMappingsConfig = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            warn!("Failed to parse schema mappings file {:?}: {}", path, err);
            return Vec::new();
        }
    };

    parsed.openapi
}

fn resolve_from_rules(
    target_url: &str,
    user_rules: Vec<OpenApiMappingRule>,
    builtin_rules: Vec<OpenApiMappingRule>,
) -> Option<ResolvedSchemaMapping> {
    let target = Url::parse(target_url).ok()?;
    let mut candidates = Vec::new();
    candidates.extend(user_rules.into_iter().map(|rule| CandidateRule {
        source: MappingSource::UserConfig,
        rule,
    }));
    candidates.extend(builtin_rules.into_iter().map(|rule| CandidateRule {
        source: MappingSource::Builtin,
        rule,
    }));

    candidates
        .into_iter()
        .filter(|candidate| candidate.rule.enabled && candidate.rule.matches(&target))
        .max_by_key(|candidate| {
            (
                candidate.source.rank(),
                candidate.rule.priority,
                candidate
                    .rule
                    .normalized_path_prefix()
                    .map_or(0usize, |prefix| prefix.len()),
            )
        })
        .map(|candidate| ResolvedSchemaMapping {
            schema_url: candidate.rule.schema_url,
            source: candidate.source,
        })
}

pub fn resolve_openapi_schema_mapping(target_url: &str) -> Option<ResolvedSchemaMapping> {
    resolve_from_rules(
        target_url,
        load_user_openapi_rules(),
        builtin_openapi_rules(),
    )
}

fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }

    #[cfg(windows)]
    {
        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(user_profile));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        host: &str,
        path_prefix: Option<&str>,
        schema_url: &str,
        priority: i32,
    ) -> OpenApiMappingRule {
        OpenApiMappingRule {
            host: host.to_string(),
            path_prefix: path_prefix.map(|value| value.to_string()),
            schema_url: schema_url.to_string(),
            enabled: true,
            priority,
        }
    }

    #[test]
    fn builtin_mapping_matches_github() {
        let resolved = resolve_from_rules(
            "https://api.github.com",
            Vec::new(),
            builtin_openapi_rules(),
        )
        .expect("should resolve github mapping");

        assert_eq!(resolved.source, MappingSource::Builtin);
        assert!(resolved.schema_url.contains("api.github.com.json"));
    }

    #[test]
    fn builtin_mapping_matches_telegram() {
        let resolved = resolve_from_rules(
            "https://api.telegram.org",
            Vec::new(),
            builtin_openapi_rules(),
        )
        .expect("should resolve telegram mapping");

        assert_eq!(resolved.source, MappingSource::Builtin);
        assert_eq!(
            resolved.schema_url,
            "https://raw.githubusercontent.com/holon-run/uxc/main/skills/telegram-openapi-skill/references/telegram-bot.openapi.json"
        );
    }

    #[test]
    fn user_mapping_overrides_builtin() {
        let resolved = resolve_from_rules(
            "https://api.github.com",
            vec![rule(
                "api.github.com",
                Some("/"),
                "https://example.com/custom-github-openapi.json",
                1,
            )],
            builtin_openapi_rules(),
        )
        .expect("should resolve mapping");

        assert_eq!(resolved.source, MappingSource::UserConfig);
        assert_eq!(
            resolved.schema_url,
            "https://example.com/custom-github-openapi.json"
        );
    }

    #[test]
    fn path_prefix_picks_more_specific_mapping() {
        let resolved = resolve_from_rules(
            "https://api.example.com/admin/users",
            vec![
                rule(
                    "api.example.com",
                    Some("/"),
                    "https://example.com/root.json",
                    10,
                ),
                rule(
                    "api.example.com",
                    Some("/admin"),
                    "https://example.com/admin.json",
                    10,
                ),
            ],
            Vec::new(),
        )
        .expect("should resolve mapping");

        assert_eq!(resolved.schema_url, "https://example.com/admin.json");
    }
}
