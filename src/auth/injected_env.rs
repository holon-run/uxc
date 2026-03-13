use crate::auth::Profile;
use crate::error::UxcError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Placeholder used in stdio child env templates so secrets stay out of argv.
const SECRET_PLACEHOLDER: &str = "{{secret}}";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InjectEnvSpec {
    pub name: String,
    pub template: String,
}

impl InjectEnvSpec {
    pub fn parse(spec: &str) -> Result<Self> {
        let Some((name, template)) = spec.split_once('=') else {
            return Err(UxcError::InvalidArguments(format!(
                "Invalid --inject-env '{}'. Expected format: NAME=<template>",
                spec
            ))
            .into());
        };
        Self::new(name, template)
    }

    pub fn new(name: &str, template: &str) -> Result<Self> {
        let normalized_name = validate_env_name(name)?;
        validate_template(template)?;
        Ok(Self {
            name: normalized_name,
            template: template.to_string(),
        })
    }

    pub fn render_with_profile(&self, profile: &Profile) -> Result<String> {
        let secret = profile.resolve_secret()?.ok_or_else(|| {
            UxcError::InvalidArguments(
                "Credential does not have a usable secret for --inject-env".to_string(),
            )
        })?;
        Ok(self.template.replace(SECRET_PLACEHOLDER, &secret))
    }

    pub fn as_cli_arg(&self) -> String {
        format!("{}={}", self.name, self.template)
    }
}

pub fn parse_inject_env_specs(raw_specs: &[String]) -> Result<Vec<InjectEnvSpec>> {
    raw_specs
        .iter()
        .map(|spec| InjectEnvSpec::parse(spec))
        .collect()
}

pub fn normalize_inject_env_specs(specs: &[InjectEnvSpec]) -> Vec<InjectEnvSpec> {
    let mut dedup = HashMap::new();
    for spec in specs {
        dedup.insert(spec.name.clone(), spec.clone());
    }
    let mut out: Vec<_> = dedup.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn render_injected_env(
    specs: &[InjectEnvSpec],
    profile: &Profile,
) -> Result<Vec<(String, String)>> {
    let specs = normalize_inject_env_specs(specs);
    let mut rendered = Vec::with_capacity(specs.len());
    for spec in specs {
        rendered.push((spec.name.clone(), spec.render_with_profile(profile)?));
    }
    Ok(rendered)
}

pub fn fingerprint_injected_env(specs: &[InjectEnvSpec], profile: &Profile) -> Result<String> {
    let rendered = render_injected_env(specs, profile)?;
    let mut hasher = Sha256::new();
    for (name, value) in rendered {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0xff]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn validate_env_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(UxcError::InvalidArguments(
            "Invalid --inject-env: env var name cannot be empty".to_string(),
        )
        .into());
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err(UxcError::InvalidArguments(
            "Invalid --inject-env: env var name cannot be empty".to_string(),
        )
        .into());
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(UxcError::InvalidArguments(format!(
            "Invalid --inject-env '{}': env var name must match [A-Za-z_][A-Za-z0-9_]*",
            trimmed
        ))
        .into());
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(UxcError::InvalidArguments(format!(
            "Invalid --inject-env '{}': env var name must match [A-Za-z_][A-Za-z0-9_]*",
            trimmed
        ))
        .into());
    }
    Ok(trimmed.to_string())
}

pub fn validate_template(template: &str) -> Result<()> {
    let occurrences = template.matches(SECRET_PLACEHOLDER).count();
    if occurrences == 0 {
        return Err(UxcError::InvalidArguments(format!(
            "Invalid --inject-env template '{}': must contain {}",
            template, SECRET_PLACEHOLDER
        ))
        .into());
    }
    if occurrences > 1 {
        return Err(UxcError::InvalidArguments(format!(
            "Invalid --inject-env template '{}': only one {} placeholder is supported",
            template, SECRET_PLACEHOLDER
        ))
        .into());
    }
    let replaced = template.replace(SECRET_PLACEHOLDER, "");
    if replaced.contains('{') || replaced.contains('}') {
        return Err(UxcError::InvalidArguments(format!(
            "Invalid --inject-env template '{}': only {} is supported",
            template, SECRET_PLACEHOLDER
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthType, OAuthProfile, Profile, SecretSource};

    #[test]
    fn parse_spec_accepts_literal_prefix() {
        let spec = InjectEnvSpec::parse("AUTH_HEADER=Bearer {{secret}}").unwrap();
        assert_eq!(spec.name, "AUTH_HEADER");
        assert_eq!(spec.template, "Bearer {{secret}}");
    }

    #[test]
    fn parse_spec_rejects_invalid_name() {
        assert!(InjectEnvSpec::parse("BAD-NAME={{secret}}").is_err());
    }

    #[test]
    fn parse_spec_rejects_missing_placeholder() {
        assert!(InjectEnvSpec::parse("TOKEN=plain").is_err());
    }

    #[test]
    fn parse_spec_rejects_multiple_placeholders() {
        assert!(InjectEnvSpec::parse("TOKEN={{secret}}:{{secret}}").is_err());
    }

    #[test]
    fn parse_spec_rejects_other_braces() {
        assert!(InjectEnvSpec::parse("TOKEN=prefix_{{secret}}_{bad}").is_err());
    }

    #[test]
    fn normalize_specs_keeps_last_value() {
        let specs = vec![
            InjectEnvSpec::parse("TOKEN={{secret}}").unwrap(),
            InjectEnvSpec::parse("OTHER=Bearer {{secret}}").unwrap(),
            InjectEnvSpec::parse("TOKEN=Bearer {{secret}}").unwrap(),
        ];
        let normalized = normalize_inject_env_specs(&specs);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[1].template, "Bearer {{secret}}");
    }

    #[test]
    fn render_with_literal_secret_profile() {
        let profile = Profile {
            api_key: "test-token".to_string(),
            auth_type: AuthType::Bearer,
            description: None,
            oauth: None,
            name: Some("demo".to_string()),
            secret_source: Some(SecretSource::Literal {
                value: "test-token".to_string(),
            }),
            fields: std::collections::HashMap::new(),
            auth_headers: None,
            auth_query_params: None,
            auth_path_prefix: None,
            signer: None,
        };
        let rendered = InjectEnvSpec::parse("TOKEN=Bearer {{secret}}")
            .unwrap()
            .render_with_profile(&profile)
            .unwrap();
        assert_eq!(rendered, "Bearer test-token");
    }

    #[test]
    fn render_with_oauth_profile_uses_access_token() {
        let mut profile = Profile::new(String::new(), AuthType::OAuth);
        profile.name = Some("oauth-demo".to_string());
        profile.oauth = Some(OAuthProfile {
            access_token: Some("oauth-token".to_string()),
            ..Default::default()
        });
        let rendered = InjectEnvSpec::parse("TOKEN={{secret}}")
            .unwrap()
            .render_with_profile(&profile)
            .unwrap();
        assert_eq!(rendered, "oauth-token");
    }
}
