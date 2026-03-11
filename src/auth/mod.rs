//! Authentication storage and resolution.
//!
//! Credentials are stored in `~/.uxc/credentials.json`.
//! Endpoint-to-credential bindings are stored in `~/.uxc/auth_bindings.json`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod injected_env;
pub mod oauth;
pub mod oauth_sessions;

/// Default auth directory relative to home directory.
pub const DEFAULT_AUTH_DIR: &str = ".uxc";
/// Credentials file name.
pub const CREDENTIALS_FILE: &str = "credentials.json";
/// Endpoint binding file name.
pub const AUTH_BINDINGS_FILE: &str = "auth_bindings.json";

const CREDENTIALS_FILE_ENV: &str = "UXC_CREDENTIALS_FILE";
const AUTH_BINDINGS_FILE_ENV: &str = "UXC_AUTH_BINDINGS_FILE";
pub const PRIMARY_SECRET_FIELD: &str = "secret";

/// Authentication type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthType {
    /// Bearer token authentication
    Bearer,
    /// API key authentication
    ApiKey,
    /// Basic authentication
    Basic,
    /// OAuth2 authentication
    OAuth,
}

impl serde::Serialize for AuthType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = match self {
            AuthType::Bearer => "bearer",
            AuthType::ApiKey => "api_key",
            AuthType::Basic => "basic",
            AuthType::OAuth => "oauth",
        };
        serializer.serialize_str(s)
    }
}

impl<'de> serde::Deserialize<'de> for AuthType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "bearer" => Ok(AuthType::Bearer),
            "api_key" => Ok(AuthType::ApiKey),
            "basic" => Ok(AuthType::Basic),
            "oauth" => Ok(AuthType::OAuth),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid auth type: {}. Valid values: bearer, api_key, basic, oauth",
                s
            ))),
        }
    }
}

impl Default for AuthType {
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self::Bearer
    }
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthType::Bearer => write!(f, "bearer"),
            AuthType::ApiKey => write!(f, "api_key"),
            AuthType::Basic => write!(f, "basic"),
            AuthType::OAuth => write!(f, "oauth"),
        }
    }
}

impl std::str::FromStr for AuthType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bearer" => Ok(AuthType::Bearer),
            "api_key" => Ok(AuthType::ApiKey),
            "basic" => Ok(AuthType::Basic),
            "oauth" => Ok(AuthType::OAuth),
            _ => anyhow::bail!(
                "Invalid auth type: {}. Valid values: bearer, api_key, basic, oauth",
                s
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OAuthFlow {
    #[serde(rename = "device_code")]
    DeviceCode,
    #[serde(rename = "authorization_code")]
    AuthorizationCode,
    #[serde(rename = "client_credentials")]
    ClientCredentials,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_metadata_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_authorization_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_flow: Option<OAuthFlow>,
}

/// Secret source for non-OAuth credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretSource {
    Literal { value: String },
    Env { key: String },
    Op { reference: String },
}

impl SecretSource {
    pub fn kind(&self) -> &'static str {
        match self {
            SecretSource::Literal { .. } => "literal",
            SecretSource::Env { .. } => "env",
            SecretSource::Op { .. } => "op",
        }
    }

    fn resolve(&self, credential_name: Option<&str>) -> Result<String> {
        match self {
            SecretSource::Literal { value } => Ok(value.clone()),
            SecretSource::Env { key } => std::env::var(key).map_err(|_| {
                anyhow::anyhow!(
                    "Credential '{}' expects env var '{}' but it is not set",
                    credential_name.unwrap_or("unknown"),
                    key
                )
            }),
            SecretSource::Op { reference } => resolve_op_secret(reference, credential_name),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignerAlgorithm {
    HmacSha256,
    Ed25519,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignerValuePlacement {
    Header,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignatureEncoding {
    Hex,
    Base64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimestampUnit {
    Seconds,
    Milliseconds,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryCanonicalizationMode {
    PreserveOrder,
    SortByKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryCanonicalization {
    pub mode: QueryCanonicalizationMode,
}

impl Default for QueryCanonicalization {
    fn default() -> Self {
        Self {
            mode: QueryCanonicalizationMode::PreserveOrder,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HmacQuerySignerConfig {
    pub algorithm: SignerAlgorithm,
    pub signing_field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_placement: Option<SignerValuePlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    pub signature_param: String,
    pub signature_encoding: SignatureEncoding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_unit: Option<TimestampUnit>,
    #[serde(default)]
    pub canonicalization: QueryCanonicalization,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ed25519QuerySignerConfig {
    pub algorithm: SignerAlgorithm,
    pub signing_field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_placement: Option<SignerValuePlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    pub signature_param: String,
    pub signature_encoding: SignatureEncoding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_unit: Option<TimestampUnit>,
    #[serde(default)]
    pub canonicalization: QueryCanonicalization,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthSignerConfig {
    HmacQueryV1(HmacQuerySignerConfig),
    Ed25519QueryV1(Ed25519QuerySignerConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthHeader {
    pub name: String,
    pub template: String,
}

impl AuthHeader {
    pub fn parse(spec: &str) -> Result<Self> {
        let Some((name, template)) = spec.split_once('=') else {
            anyhow::bail!(
                "Invalid --header '{}'. Expected format: <header-name>=<template>",
                spec
            );
        };
        Self::new(name, template)
    }

    pub fn new(name: &str, template: &str) -> Result<Self> {
        let normalized_name = validate_header_name(name)?;
        validate_header_template(template)?;
        Ok(Self {
            name: normalized_name,
            template: template.to_string(),
        })
    }

    pub fn requires_primary_secret(&self) -> bool {
        template_has_secret(&self.template)
    }

    pub fn render_value(&self, profile: &Profile) -> Result<String> {
        render_template(&self.template, profile, "auth header")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthQueryParam {
    pub name: String,
    pub template: String,
}

impl AuthQueryParam {
    pub fn parse(spec: &str) -> Result<Self> {
        let Some((name, template)) = spec.split_once('=') else {
            anyhow::bail!(
                "Invalid --query-param '{}'. Expected format: <query-param-name>=<template>",
                spec
            );
        };
        Self::new(name, template)
    }

    pub fn new(name: &str, template: &str) -> Result<Self> {
        let normalized_name = validate_query_param_name(name)?;
        validate_template(template)?;
        Ok(Self {
            name: normalized_name,
            template: template.to_string(),
        })
    }

    pub fn requires_primary_secret(&self) -> bool {
        template_has_secret(&self.template)
    }

    pub fn render_value(&self, profile: &Profile) -> Result<String> {
        render_template(&self.template, profile, "auth query param")
    }
}

/// Runtime authentication credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Active API key/token used for request execution.
    #[serde(default)]
    pub api_key: String,

    /// Authentication type.
    #[serde(default)]
    pub auth_type: AuthType,

    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthProfile>,

    /// Runtime-only identifier.
    #[serde(skip)]
    pub name: Option<String>,

    /// Optional secret source for non-OAuth credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_source: Option<SecretSource>,

    /// Optional named secret fields for non-OAuth credentials.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, SecretSource>,

    /// Optional custom auth headers used by api_key auth type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_headers: Option<Vec<AuthHeader>>,

    /// Optional custom auth query params used by api_key auth type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_query_params: Option<Vec<AuthQueryParam>>,

    /// Optional binding-level signer attached at runtime.
    #[serde(skip)]
    pub signer: Option<AuthSignerConfig>,
}

impl Profile {
    /// Create a new credential with literal secret.
    pub fn new(api_key: String, auth_type: AuthType) -> Self {
        let secret_source = if auth_type == AuthType::OAuth {
            None
        } else {
            Some(SecretSource::Literal {
                value: api_key.clone(),
            })
        };

        Self {
            api_key,
            auth_type,
            description: None,
            oauth: None,
            name: None,
            secret_source,
            fields: HashMap::new(),
            auth_headers: None,
            auth_query_params: None,
            signer: None,
        }
    }

    /// Create a new credential with description.
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    pub fn with_oauth(mut self, oauth: OAuthProfile) -> Self {
        self.oauth = Some(oauth);
        self
    }

    pub fn with_secret_env(mut self, key: String) -> Self {
        self.secret_source = Some(SecretSource::Env { key });
        self.api_key.clear();
        self
    }

    pub fn with_secret_op(mut self, reference: String) -> Self {
        self.secret_source = Some(SecretSource::Op { reference });
        self.api_key.clear();
        self
    }

    pub fn set_field_source(&mut self, name: String, source: SecretSource) -> Result<()> {
        let normalized = validate_field_name(&name)?;
        self.fields.insert(normalized, source);
        Ok(())
    }

    pub fn clear_fields(&mut self) {
        self.fields.clear();
    }

    pub fn resolve_field_value(&self, name: &str) -> Result<Option<String>> {
        if let Some(source) = self.fields.get(name) {
            return Ok(Some(source.resolve(self.name.as_deref())?));
        }

        if name == PRIMARY_SECRET_FIELD {
            if self.auth_type == AuthType::OAuth {
                return Ok(self
                    .oauth
                    .as_ref()
                    .and_then(|oauth| oauth.access_token.clone()));
            }

            if let Some(source) = &self.secret_source {
                return Ok(Some(source.resolve(self.name.as_deref())?));
            }

            if !self.api_key.is_empty() {
                return Ok(Some(self.api_key.clone()));
            }
        }

        Ok(None)
    }

    /// Resolve runtime secret from secret_source.
    pub fn resolve_secret(&self) -> Result<Option<String>> {
        self.resolve_field_value(PRIMARY_SECRET_FIELD)
    }

    /// Materialize secret into `api_key` for runtime execution.
    pub fn materialize_runtime(mut self) -> Result<Self> {
        if let Some(secret) = self.resolve_secret()? {
            self.api_key = secret;
        }
        Ok(self)
    }

    /// Mask the API key for display (show only first 8 and last 4 characters)
    pub fn mask_api_key(&self) -> String {
        let key = self.active_secret_for_masking();
        if key.len() <= 12 {
            return "*".repeat(key.len());
        }
        format!("{}...{}", &key[..8], &key[key.len() - 4..])
    }

    pub fn bearer_token(&self) -> Option<&str> {
        match self.auth_type {
            AuthType::Bearer => {
                if self.api_key.is_empty() {
                    None
                } else {
                    Some(self.api_key.as_str())
                }
            }
            AuthType::OAuth => self.oauth.as_ref()?.access_token.as_deref(),
            _ => None,
        }
    }

    fn active_secret_for_masking(&self) -> String {
        if let Some(token) = self.bearer_token() {
            return token.to_string();
        }

        if let Some(source) = self.fields.get(PRIMARY_SECRET_FIELD) {
            return match source {
                SecretSource::Literal { value } => value.clone(),
                SecretSource::Env { .. } | SecretSource::Op { .. } => "********".to_string(),
            };
        }

        if !self.api_key.is_empty() {
            return self.api_key.clone();
        }

        if let Some(source) = &self.secret_source {
            return match source {
                SecretSource::Literal { value } => value.clone(),
                SecretSource::Env { .. } | SecretSource::Op { .. } => "********".to_string(),
            };
        }

        String::new()
    }

    pub fn has_custom_api_key_headers(&self) -> bool {
        self.auth_type == AuthType::ApiKey
            && self
                .auth_headers
                .as_ref()
                .is_some_and(|headers| !headers.is_empty())
    }

    pub fn has_custom_api_key_query_params(&self) -> bool {
        self.auth_type == AuthType::ApiKey
            && self
                .auth_query_params
                .as_ref()
                .is_some_and(|params| !params.is_empty())
    }

    pub fn api_key_injections_require_secret(&self) -> bool {
        self.auth_type == AuthType::ApiKey
            && (self.auth_headers.as_ref().is_some_and(|headers| {
                !headers.is_empty() && headers.iter().any(AuthHeader::requires_primary_secret)
            }) || self.auth_query_params.as_ref().is_some_and(|params| {
                !params.is_empty() && params.iter().any(AuthQueryParam::requires_primary_secret)
            }))
    }

    pub fn field_source_kinds(&self) -> Vec<(String, String)> {
        let mut items: Vec<(String, String)> = self
            .fields
            .iter()
            .map(|(name, source)| (name.clone(), source.kind().to_string()))
            .collect();

        if !self.fields.contains_key(PRIMARY_SECRET_FIELD) {
            if let Some(source) = &self.secret_source {
                items.push((PRIMARY_SECRET_FIELD.to_string(), source.kind().to_string()));
            } else if !self.api_key.is_empty() && self.auth_type != AuthType::OAuth {
                items.push((PRIMARY_SECRET_FIELD.to_string(), "literal".to_string()));
            }
        }

        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    pub fn resolved_api_key_headers(&self) -> Result<Vec<(String, String)>> {
        if self.auth_type != AuthType::ApiKey {
            return Ok(Vec::new());
        }

        if let Some(headers) = &self.auth_headers {
            if !headers.is_empty() {
                let mut values = Vec::with_capacity(headers.len());
                for header in headers {
                    values.push((header.name.clone(), header.render_value(self)?));
                }
                return Ok(values);
            }
        }

        if self.has_custom_api_key_query_params() {
            return Ok(Vec::new());
        }

        if self.signer.is_some() || self.api_key.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![("x-api-key".to_string(), self.api_key.clone())])
    }

    pub fn resolved_api_key_query_params(&self) -> Result<Vec<(String, String)>> {
        if self.auth_type != AuthType::ApiKey {
            return Ok(Vec::new());
        }

        if let Some(params) = &self.auth_query_params {
            if !params.is_empty() {
                let mut values = Vec::with_capacity(params.len());
                for param in params {
                    values.push((param.name.clone(), param.render_value(self)?));
                }
                return Ok(values);
            }
        }

        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CredentialsDocument {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    credentials: HashMap<String, StoredCredential>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCredential {
    #[serde(default)]
    auth_type: AuthType,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth: Option<OAuthProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret_source: Option<SecretSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    fields: HashMap<String, SecretSource>,
    // auth_headers persistence is handled by serde derives on AuthHeader.
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_headers: Option<Vec<AuthHeader>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_query_params: Option<Vec<AuthQueryParam>>,
}

impl StoredCredential {
    fn from_runtime(profile: &Profile) -> Self {
        let secret_source = if profile.auth_type == AuthType::OAuth {
            None
        } else {
            profile.secret_source.clone().or_else(|| {
                Some(SecretSource::Literal {
                    value: profile.api_key.clone(),
                })
            })
        };

        let api_key = if profile.auth_type == AuthType::OAuth {
            profile
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.access_token.clone())
        } else {
            None
        };

        Self {
            auth_type: profile.auth_type.clone(),
            description: profile.description.clone(),
            oauth: profile.oauth.clone(),
            secret_source,
            api_key,
            fields: profile.fields.clone(),
            auth_headers: profile.auth_headers.clone(),
            auth_query_params: profile.auth_query_params.clone(),
        }
    }

    fn to_runtime(&self, name: &str) -> Profile {
        let secret_source = if self.auth_type == AuthType::OAuth {
            None
        } else {
            self.secret_source.clone().or_else(|| {
                self.api_key.as_ref().map(|value| SecretSource::Literal {
                    value: value.clone(),
                })
            })
        };

        let mut profile = Profile {
            api_key: String::new(),
            auth_type: self.auth_type.clone(),
            description: self.description.clone(),
            oauth: self.oauth.clone(),
            name: Some(name.to_string()),
            secret_source,
            fields: self.fields.clone(),
            auth_headers: self.auth_headers.clone(),
            auth_query_params: self.auth_query_params.clone(),
            signer: None,
        };

        if profile.auth_type == AuthType::OAuth {
            profile.api_key = profile
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.access_token.clone())
                .or_else(|| self.api_key.clone())
                .unwrap_or_default();
        } else if let Some(SecretSource::Literal { value }) = &profile.secret_source {
            profile.api_key = value.clone();
        }

        profile
    }
}

/// Credentials collection.
#[derive(Debug, Clone)]
pub struct Profiles {
    pub profiles: HashMap<String, Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self::new()
    }
}

impl Profiles {
    /// Create a new empty credentials collection.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Resolve credentials file path.
    fn profiles_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(CREDENTIALS_FILE_ENV) {
            return Ok(PathBuf::from(path));
        }

        Ok(auth_base_dir()?.join(CREDENTIALS_FILE))
    }

    /// Load credentials from disk.
    pub fn load_profiles() -> Result<Self> {
        let path = Self::profiles_path()?;

        if !path.exists() {
            return Ok(Self::new());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read credentials file: {:?}", path))?;

        let document: CredentialsDocument = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse credentials file: {:?}", path))?;

        let profiles = document
            .credentials
            .iter()
            .map(|(name, stored)| (name.clone(), stored.to_runtime(name)))
            .collect();

        Ok(Self { profiles })
    }

    /// Save credentials to disk.
    pub fn save_profiles(&self) -> Result<()> {
        let path = Self::profiles_path()?;

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create auth directory: {:?}", parent))?;
            }
        }

        let credentials = self
            .profiles
            .iter()
            .map(|(name, profile)| (name.clone(), StoredCredential::from_runtime(profile)))
            .collect();

        let document = CredentialsDocument {
            version: 1,
            credentials,
        };

        let json = serde_json::to_string_pretty(&document)
            .context("Failed to serialize credentials to JSON")?;

        write_secure_auth_file(&path, &json, "credentials")?;

        Ok(())
    }

    /// Get a credential by ID.
    pub fn get_profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).context(format!(
            "Credential '{}' not found. Available credentials: {}",
            name,
            self.list_names()
        ))
    }

    /// Validate a credential ID.
    pub fn validate_profile_name(name: &str) -> Result<()> {
        if name.is_empty() {
            anyhow::bail!("Credential ID cannot be empty");
        }

        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            anyhow::bail!(
                "Credential ID '{}' contains invalid characters. Allowed: letters, digits, '_', '-'",
                name
            );
        }

        if name
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            anyhow::bail!("Credential ID '{}' cannot start with a digit", name);
        }

        Ok(())
    }

    /// Set a credential.
    pub fn set_profile(&mut self, name: String, mut profile: Profile) -> Result<()> {
        Self::validate_profile_name(&name)?;
        profile.name = Some(name.clone());
        self.profiles.insert(name, profile);
        Ok(())
    }

    /// Remove a credential.
    pub fn remove_profile(&mut self, name: &str) -> Result<()> {
        self.profiles
            .remove(name)
            .context(format!("Credential '{}' not found", name))?;
        Ok(())
    }

    /// List all credential IDs.
    pub fn list_names(&self) -> String {
        let mut names: Vec<_> = self.profiles.keys().cloned().collect();
        names.sort();
        names.join(", ")
    }

    /// Get all credential IDs.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.profiles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Check if a credential exists.
    pub fn has_profile(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    /// Get credential count.
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.profiles.len()
    }
}

/// Endpoint auth binding rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthBindingRule {
    pub id: String,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    pub credential: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer: Option<AuthSignerConfig>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthBindingsDocument {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    bindings: Vec<AuthBindingRule>,
}

/// Endpoint binding collection.
#[derive(Debug, Clone, Default)]
pub struct AuthBindings {
    pub bindings: Vec<AuthBindingRule>,
}

impl AuthBindings {
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    fn bindings_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os(AUTH_BINDINGS_FILE_ENV) {
            return Ok(PathBuf::from(path));
        }

        Ok(auth_base_dir()?.join(AUTH_BINDINGS_FILE))
    }

    pub fn load_bindings() -> Result<Self> {
        let path = Self::bindings_path()?;
        if !path.exists() {
            return Ok(Self::new());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read auth bindings file: {:?}", path))?;

        let document: AuthBindingsDocument = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse auth bindings file: {:?}", path))?;

        Ok(Self {
            bindings: document.bindings,
        })
    }

    pub fn save_bindings(&self) -> Result<()> {
        let path = Self::bindings_path()?;

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create auth directory: {:?}", parent))?;
            }
        }

        let document = AuthBindingsDocument {
            version: 1,
            bindings: self.bindings.clone(),
        };
        let json = serde_json::to_string_pretty(&document)
            .context("Failed to serialize auth bindings to JSON")?;

        write_secure_auth_file(&path, &json, "auth bindings")?;

        Ok(())
    }

    pub fn add_binding(&mut self, mut rule: AuthBindingRule) -> Result<()> {
        validate_binding_id(&rule.id)?;
        if self.bindings.iter().any(|item| item.id == rule.id) {
            anyhow::bail!("Binding '{}' already exists", rule.id);
        }
        if let Some(signer) = &rule.signer {
            validate_signer_config(signer)?;
        }

        rule.host = rule.host.trim().to_ascii_lowercase();
        if let Some(scheme) = &rule.scheme {
            rule.scheme = Some(scheme.trim().to_ascii_lowercase());
        }
        if let Some(prefix) = &rule.path_prefix {
            rule.path_prefix = Some(normalize_path_prefix(prefix));
        }

        self.bindings.push(rule);
        Ok(())
    }

    pub fn remove_binding(&mut self, id: &str) -> Result<()> {
        let before = self.bindings.len();
        self.bindings.retain(|rule| rule.id != id);
        if self.bindings.len() == before {
            anyhow::bail!("Binding '{}' not found", id);
        }
        Ok(())
    }

    pub fn matching_rule(&self, endpoint: &str) -> Option<&AuthBindingRule> {
        let target = url::Url::parse(endpoint).ok()?;
        let host = target.host_str()?.to_ascii_lowercase();
        let scheme = target.scheme().to_ascii_lowercase();
        let path = target.path();

        self.bindings
            .iter()
            .filter(|rule| rule.enabled)
            .filter(|rule| rule.host.eq_ignore_ascii_case(&host))
            .filter(|rule| {
                rule.scheme
                    .as_ref()
                    .map(|s| s.eq_ignore_ascii_case(&scheme))
                    .unwrap_or(true)
            })
            .filter(|rule| {
                rule.path_prefix
                    .as_ref()
                    .map(|prefix| path.starts_with(prefix))
                    .unwrap_or(true)
            })
            .max_by(compare_binding_rules)
    }
}

fn compare_binding_rules(a: &&AuthBindingRule, b: &&AuthBindingRule) -> Ordering {
    let pa = a.priority;
    let pb = b.priority;
    if pa != pb {
        return pa.cmp(&pb);
    }

    let la = a.path_prefix.as_ref().map_or(0usize, |value| value.len());
    let lb = b.path_prefix.as_ref().map_or(0usize, |value| value.len());
    if la != lb {
        return la.cmp(&lb);
    }

    a.id.cmp(&b.id)
}

fn normalize_path_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    }
}

fn validate_binding_id(id: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("Binding ID cannot be empty");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!(
            "Binding ID '{}' contains invalid characters. Allowed: letters, digits, '_', '-'",
            id
        );
    }
    Ok(())
}

/// Resolve auth credential for endpoint.
pub fn resolve_auth_for_endpoint(
    endpoint: &str,
    explicit_credential: Option<String>,
) -> Result<Option<Profile>> {
    let profiles = Profiles::load_profiles()?;

    if let Some(id) = explicit_credential {
        let profile = profiles.get_profile(&id)?.clone().materialize_runtime()?;
        validate_ready(&profile)?;
        return Ok(Some(profile));
    }

    let bindings = AuthBindings::load_bindings()?;
    let Some(rule) = bindings.matching_rule(endpoint) else {
        return Ok(None);
    };

    let profile = profiles
        .get_profile(&rule.credential)
        .with_context(|| {
            format!(
                "Binding '{}' references missing credential '{}'",
                rule.id, rule.credential
            )
        })?
        .clone()
        .materialize_runtime()?;
    let mut profile = profile;
    profile.signer = rule.signer.clone();

    validate_ready(&profile)?;
    Ok(Some(profile))
}

pub fn persist_profile_if_named(profile: &Profile) -> Result<()> {
    let Some(name) = profile
        .name
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    let mut profiles = Profiles::load_profiles()?;
    if !profiles.has_profile(name) {
        return Ok(());
    }

    profiles.set_profile(name.to_string(), profile.clone())?;
    profiles.save_profiles()?;
    Ok(())
}

fn validate_ready(profile: &Profile) -> Result<()> {
    match profile.auth_type {
        AuthType::OAuth => {
            if profile
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.access_token.as_ref())
                .is_none()
            {
                anyhow::bail!("OAuth credential is missing access token. Run `uxc auth oauth login <credential_id> ...`");
            }
        }
        AuthType::ApiKey => {
            let requires_primary_secret = if profile.has_custom_api_key_headers()
                || profile.has_custom_api_key_query_params()
            {
                profile.api_key_injections_require_secret()
            } else {
                profile.signer.is_none()
            };
            if !requires_primary_secret {
                return Ok(());
            }
            if profile.api_key.is_empty() {
                anyhow::bail!(
                    "Credential '{}' does not have a usable secret. Set it with --secret, --secret-env, or --secret-op.",
                    profile.name.as_deref().unwrap_or("unknown"),
                );
            }
        }
        _ => {
            if profile.api_key.is_empty() {
                anyhow::bail!(
                    "Credential '{}' does not have a usable secret. Set it with --secret, --secret-env, or --secret-op.",
                    profile.name.as_deref().unwrap_or("unknown"),
                );
            }
        }
    }

    Ok(())
}

pub fn validate_signer_config(config: &AuthSignerConfig) -> Result<()> {
    match config {
        AuthSignerConfig::HmacQueryV1(hmac) => {
            if hmac.algorithm != SignerAlgorithm::HmacSha256 {
                anyhow::bail!("hmac_query_v1 signer requires algorithm=hmac_sha256");
            }
            validate_field_name(&hmac.signing_field)?;
            validate_query_param_name(&hmac.signature_param)?;
            if let Some(timestamp_param) = &hmac.timestamp_param {
                validate_query_param_name(timestamp_param)?;
            }

            match (&hmac.key_field, &hmac.key_placement, &hmac.key_name) {
                (None, None, None) => {}
                (Some(field), Some(_), Some(name)) => {
                    validate_field_name(field)?;
                    match hmac.key_placement {
                        Some(SignerValuePlacement::Query) => {
                            validate_query_param_name(name)?;
                        }
                        Some(SignerValuePlacement::Header) => {
                            validate_header_name(name)?;
                        }
                        None => unreachable!(),
                    };
                }
                _ => {
                    anyhow::bail!(
                        "hmac_query_v1 signer requires key_field, key_placement, and key_name to be set together"
                    );
                }
            }
        }
        AuthSignerConfig::Ed25519QueryV1(ed25519) => {
            if ed25519.algorithm != SignerAlgorithm::Ed25519 {
                anyhow::bail!("ed25519_query_v1 signer requires algorithm=ed25519");
            }
            validate_field_name(&ed25519.signing_field)?;
            validate_query_param_name(&ed25519.signature_param)?;
            if let Some(timestamp_param) = &ed25519.timestamp_param {
                validate_query_param_name(timestamp_param)?;
            }

            match (
                &ed25519.key_field,
                &ed25519.key_placement,
                &ed25519.key_name,
            ) {
                (None, None, None) => {}
                (Some(field), Some(_), Some(name)) => {
                    validate_field_name(field)?;
                    match ed25519.key_placement {
                        Some(SignerValuePlacement::Query) => {
                            validate_query_param_name(name)?;
                        }
                        Some(SignerValuePlacement::Header) => {
                            validate_header_name(name)?;
                        }
                        None => unreachable!(),
                    };
                }
                _ => {
                    anyhow::bail!(
                        "ed25519_query_v1 signer requires key_field, key_placement, and key_name to be set together"
                    );
                }
            }

            if ed25519.signature_encoding != SignatureEncoding::Base64 {
                anyhow::bail!("ed25519_query_v1 signer requires signature_encoding=base64");
            }
        }
    }

    Ok(())
}

pub fn validate_auth_headers(headers: &[AuthHeader]) -> Result<()> {
    if headers.is_empty() {
        anyhow::bail!("Custom auth headers cannot be empty");
    }

    let mut seen = HashSet::new();
    for header in headers {
        validate_header_name(&header.name)?;
        validate_header_template(&header.template)?;
        let key = header.name.to_ascii_lowercase();
        if !seen.insert(key) {
            anyhow::bail!("Duplicate auth header '{}'", header.name);
        }
    }
    Ok(())
}

pub fn validate_auth_query_params(params: &[AuthQueryParam]) -> Result<()> {
    if params.is_empty() {
        anyhow::bail!("Custom auth query params cannot be empty");
    }

    let mut seen = HashSet::new();
    for param in params {
        validate_query_param_name(&param.name)?;
        validate_template(&param.template)?;
        if !seen.insert(param.name.clone()) {
            anyhow::bail!("Duplicate auth query param '{}'", param.name);
        }
    }
    Ok(())
}

fn validate_header_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Header name cannot be empty");
    }

    reqwest::header::HeaderName::from_bytes(trimmed.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid header name '{}'", trimmed))?;
    Ok(trimmed.to_string())
}

fn validate_template(template: &str) -> Result<()> {
    parse_template_tokens(template).map(|_| ())
}

fn validate_header_template(template: &str) -> Result<()> {
    validate_template(template)
}

fn validate_query_param_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Query param name cannot be empty");
    }
    if trimmed.contains('=') {
        anyhow::bail!("Invalid query param name '{}'", trimmed);
    }
    Ok(trimmed.to_string())
}

pub fn validate_field_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Field name cannot be empty");
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        anyhow::bail!(
            "Invalid field name '{}'. Allowed: letters, digits, '_', '-'",
            trimmed
        );
    }
    Ok(trimmed.to_string())
}

pub fn parse_field_source(spec: &str) -> Result<SecretSource> {
    if let Some(value) = spec.strip_prefix("literal:") {
        return Ok(SecretSource::Literal {
            value: value.to_string(),
        });
    }
    if let Some(key) = spec.strip_prefix("env:") {
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("Invalid field source '{}': env name cannot be empty", spec);
        }
        return Ok(SecretSource::Env {
            key: key.to_string(),
        });
    }
    if spec.starts_with("op://") {
        return Ok(SecretSource::Op {
            reference: spec.to_string(),
        });
    }

    anyhow::bail!(
        "Invalid field source '{}'. Use literal:<value>, env:<VAR>, or op://...",
        spec
    )
}

pub fn parse_field_spec(spec: &str) -> Result<(String, SecretSource)> {
    let Some((name, source)) = spec.split_once('=') else {
        anyhow::bail!(
            "Invalid --field '{}'. Expected format: <field-name>=<source>",
            spec
        );
    };
    Ok((
        validate_field_name(name)?,
        parse_field_source(source.trim())?,
    ))
}

fn template_has_secret(template: &str) -> bool {
    match parse_template_tokens(template) {
        Ok(tokens) => tokens
            .iter()
            .any(|token| matches!(token, TemplateToken::Secret)),
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplateToken {
    Secret,
    Field(String),
    Env(String),
    Op(String),
}

fn parse_template_tokens(template: &str) -> Result<Vec<TemplateToken>> {
    let mut tokens = Vec::new();
    let mut cursor = 0usize;
    while let Some(start_rel) = template[cursor..].find("{{") {
        let start = cursor + start_rel;
        let rest = &template[start + 2..];
        let Some(end_rel) = rest.find("}}") else {
            anyhow::bail!("Unclosed template token in '{}'", template);
        };
        let end = start + 2 + end_rel;
        let raw = template[start + 2..end].trim();

        let token = if raw == "secret" {
            TemplateToken::Secret
        } else if let Some(field_name) = raw.strip_prefix("field:") {
            TemplateToken::Field(validate_field_name(field_name.trim())?)
        } else if let Some(env_key) = raw.strip_prefix("env:") {
            let env_key = env_key.trim();
            if env_key.is_empty() {
                anyhow::bail!("Invalid template token '{{{{{}}}}}'", raw);
            }
            TemplateToken::Env(env_key.to_string())
        } else if raw.starts_with("op://") {
            TemplateToken::Op(raw.to_string())
        } else {
            anyhow::bail!("Unsupported template token '{{{{{}}}}}'", raw);
        };
        tokens.push(token);
        cursor = end + 2;
    }
    if template[cursor..].contains("}}") {
        anyhow::bail!("Unexpected closing template token in '{}'", template);
    }
    Ok(tokens)
}

fn render_template(template: &str, profile: &Profile, target: &str) -> Result<String> {
    let _ = parse_template_tokens(template)?;

    let mut rendered = String::new();
    let mut cursor = 0usize;
    while let Some(start_rel) = template[cursor..].find("{{") {
        let start = cursor + start_rel;
        rendered.push_str(&template[cursor..start]);
        let rest = &template[start + 2..];
        let end_rel = rest
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("Unclosed template token in '{}'", template))?;
        let end = start + 2 + end_rel;
        let raw = template[start + 2..end].trim();

        if raw == "secret" {
            let Some(value) = profile.resolve_secret()? else {
                anyhow::bail!(
                    "Credential '{}' requires a secret for '{{{{secret}}}}' {} template",
                    profile.name.as_deref().unwrap_or("unknown"),
                    target
                );
            };
            if value.is_empty() {
                anyhow::bail!(
                    "Credential '{}' requires a non-empty secret for '{{{{secret}}}}' {} template",
                    profile.name.as_deref().unwrap_or("unknown"),
                    target
                );
            }
            rendered.push_str(&value);
        } else if let Some(field_name) = raw.strip_prefix("field:") {
            let field_name = validate_field_name(field_name.trim())?;
            debug_assert!(validate_field_name(&field_name).is_ok());
            let Some(value) = profile.resolve_field_value(&field_name)? else {
                anyhow::bail!(
                    "Credential '{}' is missing field '{}' required by {} template",
                    profile.name.as_deref().unwrap_or("unknown"),
                    field_name,
                    target
                );
            };
            rendered.push_str(&value);
        } else if let Some(env_key) = raw.strip_prefix("env:") {
            let env_key = env_key.trim();
            let value = std::env::var(env_key).map_err(|_| {
                anyhow::anyhow!(
                    "Credential '{}' expects env var '{}' for {} template but it is not set",
                    profile.name.as_deref().unwrap_or("unknown"),
                    env_key,
                    target
                )
            })?;
            rendered.push_str(&value);
        } else if raw.starts_with("op://") {
            let value = resolve_op_secret(raw, profile.name.as_deref())?;
            rendered.push_str(&value);
        } else {
            anyhow::bail!("Unsupported template token '{{{{{}}}}}'", raw);
        }

        cursor = end + 2;
    }
    rendered.push_str(&template[cursor..]);
    Ok(rendered)
}

fn resolve_op_secret(reference: &str, credential_name: Option<&str>) -> Result<String> {
    let output = match std::process::Command::new("op")
        .arg("read")
        .arg(reference)
        .arg("--no-newline")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "Credential '{}' uses 1Password secret source, but 'op' CLI was not found in PATH. Install it from https://developer.1password.com/docs/cli/",
                credential_name.unwrap_or("unknown")
            );
        }
        Err(err) => {
            anyhow::bail!(
                "Failed to execute 'op read' for credential '{}': {}",
                credential_name.unwrap_or("unknown"),
                err
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = sanitize_command_error(&stderr);
        anyhow::bail!(
            "Failed to resolve 1Password secret for credential '{}': {}",
            credential_name.unwrap_or("unknown"),
            message
        );
    }

    let value = String::from_utf8(output.stdout)
        .context("Failed to decode 1Password CLI output as UTF-8")?
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if value.is_empty() {
        anyhow::bail!(
            "1Password secret reference returned empty value for credential '{}'",
            credential_name.unwrap_or("unknown")
        );
    }
    Ok(value)
}

fn sanitize_command_error(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return "unknown error".to_string();
    }

    let single_line = trimmed.lines().next().unwrap_or(trimmed).trim();
    let mut truncated = String::new();
    let mut char_count = 0usize;
    for ch in single_line.chars() {
        if char_count >= 240 {
            break;
        }
        truncated.push(ch);
        char_count += 1;
    }
    if single_line.chars().count() > char_count {
        truncated.push_str("...");
    }
    truncated
}

pub(crate) fn auth_base_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(DEFAULT_AUTH_DIR))
}

pub(crate) fn write_secure_auth_file(
    path: &std::path::Path,
    contents: &str,
    label: &str,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create auth directory: {:?}", parent))?;
        }
    }

    let temp_path = temporary_auth_file_path(path);
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(&temp_path)
        .with_context(|| format!("Failed to create temporary {} file: {:?}", label, temp_path))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("Failed to write temporary {} file: {:?}", label, temp_path))?;
    file.sync_all()
        .with_context(|| format!("Failed to sync temporary {} file: {:?}", label, temp_path))?;
    drop(file);

    if let Err(rename_err) = fs::rename(&temp_path, path) {
        #[cfg(windows)]
        {
            if path.exists() {
                fs::remove_file(path).with_context(|| {
                    format!(
                        "Failed to remove existing {} file before replace: {:?}",
                        label, path
                    )
                })?;
            }
            if let Err(retry_err) = fs::rename(&temp_path, path) {
                let _ = fs::remove_file(&temp_path);
                return Err(retry_err).with_context(|| {
                    format!(
                        "Failed to replace {} file on Windows: temp={:?}, target={:?}",
                        label, temp_path, path
                    )
                });
            }
        }
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&temp_path);
            return Err(rename_err).with_context(|| {
                format!(
                    "Failed to atomically replace {} file: temp={:?}, target={:?}",
                    label, temp_path, path
                )
            });
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "Failed to set secure permissions on {} file: {:?}",
                label, path
            )
        })?;
    }

    Ok(())
}

fn temporary_auth_file_path(path: &std::path::Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("auth-file");
    let pid = std::process::id();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for nonce in 0..64u32 {
        let candidate = parent.join(format!(".{}.{}.{}.{}.tmp", filename, pid, now, nonce));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(format!(".{}.{}.{}.tmp", filename, pid, now))
}

fn upsert_query_pair(pairs: &mut Vec<(String, String)>, name: &str, value: String) {
    pairs.retain(|(key, _)| key != name);
    pairs.push((name.to_string(), value));
}

fn encode_query_pairs(pairs: &[(String, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

fn canonicalize_pairs(
    pairs: &[(String, String)],
    mode: &QueryCanonicalizationMode,
) -> Vec<(String, String)> {
    let mut canonical = pairs.to_vec();
    if matches!(mode, QueryCanonicalizationMode::SortByKey) {
        canonical.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    }
    canonical
}

fn current_timestamp(unit: &TimestampUnit) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    match unit {
        TimestampUnit::Seconds => now.as_secs().to_string(),
        TimestampUnit::Milliseconds => now.as_millis().to_string(),
    }
}

fn hmac_sha256(secret: &str, data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC-SHA256 accepts arbitrary key sizes");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn ed25519_sign(private_key_pem: &str, data: &[u8]) -> Result<Vec<u8>> {
    use ed25519_dalek::pkcs8::DecodePrivateKey;
    use ed25519_dalek::Signer;

    let signing_key =
        ed25519_dalek::SigningKey::from_pkcs8_pem(private_key_pem.trim()).map_err(|err| {
            anyhow::anyhow!("Failed to parse Ed25519 private key as PKCS#8 PEM: {}", err)
        })?;
    Ok(signing_key.sign(data).to_bytes().to_vec())
}

fn encode_signature(bytes: &[u8], encoding: &SignatureEncoding) -> String {
    match encoding {
        SignatureEncoding::Hex => bytes.iter().map(|byte| format!("{:02x}", byte)).collect(),
        SignatureEncoding::Base64 => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(bytes)
        }
    }
}

fn signer_key_header(
    key_field: Option<&String>,
    key_placement: Option<&SignerValuePlacement>,
    key_name: Option<&String>,
    profile: &Profile,
) -> Result<Option<(String, String)>> {
    let (Some(field_name), Some(SignerValuePlacement::Header), Some(header_name)) =
        (key_field, key_placement, key_name)
    else {
        return Ok(None);
    };

    let Some(value) = profile.resolve_field_value(field_name)? else {
        anyhow::bail!(
            "Credential '{}' is missing field '{}' required by signer header injection",
            profile.name.as_deref().unwrap_or("unknown"),
            field_name
        );
    };
    Ok(Some((header_name.clone(), value)))
}

fn signer_header(profile: &Profile) -> Result<Option<(String, String)>> {
    match profile.signer.as_ref() {
        Some(AuthSignerConfig::HmacQueryV1(config)) => signer_key_header(
            config.key_field.as_ref(),
            config.key_placement.as_ref(),
            config.key_name.as_ref(),
            profile,
        ),
        Some(AuthSignerConfig::Ed25519QueryV1(config)) => signer_key_header(
            config.key_field.as_ref(),
            config.key_placement.as_ref(),
            config.key_name.as_ref(),
            profile,
        ),
        None => Ok(None),
    }
}

fn apply_signer_to_url(url: &str, profile: &Profile) -> Result<String> {
    let Some(config) = profile.signer.as_ref() else {
        return Ok(url.to_string());
    };

    let mut parsed = url::Url::parse(url)
        .with_context(|| format!("Invalid endpoint URL for signer: {}", url))?;
    let mut pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();

    let (
        signature_param,
        timestamp_param,
        timestamp_unit,
        key_field,
        key_placement,
        key_name,
        signing_field,
        signature_encoding,
        canonicalization_mode,
    ) = match config {
        AuthSignerConfig::HmacQueryV1(config) => (
            &config.signature_param,
            config.timestamp_param.as_ref(),
            config
                .timestamp_unit
                .as_ref()
                .unwrap_or(&TimestampUnit::Milliseconds),
            config.key_field.as_ref(),
            config.key_placement.as_ref(),
            config.key_name.as_ref(),
            &config.signing_field,
            &config.signature_encoding,
            &config.canonicalization.mode,
        ),
        AuthSignerConfig::Ed25519QueryV1(config) => (
            &config.signature_param,
            config.timestamp_param.as_ref(),
            config
                .timestamp_unit
                .as_ref()
                .unwrap_or(&TimestampUnit::Milliseconds),
            config.key_field.as_ref(),
            config.key_placement.as_ref(),
            config.key_name.as_ref(),
            &config.signing_field,
            &config.signature_encoding,
            &config.canonicalization.mode,
        ),
    };

    pairs.retain(|(name, _)| name != signature_param);

    if let Some(timestamp_param) = timestamp_param {
        upsert_query_pair(
            &mut pairs,
            timestamp_param,
            current_timestamp(timestamp_unit),
        );
    }

    if let (Some(field_name), Some(SignerValuePlacement::Query), Some(param_name)) =
        (key_field, key_placement, key_name)
    {
        let Some(value) = profile.resolve_field_value(field_name)? else {
            anyhow::bail!(
                "Credential '{}' is missing field '{}' required by signer query injection",
                profile.name.as_deref().unwrap_or("unknown"),
                field_name
            );
        };
        upsert_query_pair(&mut pairs, param_name, value);
    }

    let canonical_pairs = canonicalize_pairs(&pairs, canonicalization_mode);
    let canonical_query = encode_query_pairs(&canonical_pairs);
    let Some(signing_secret) = profile.resolve_field_value(signing_field)? else {
        anyhow::bail!(
            "Credential '{}' is missing signing field '{}'",
            profile.name.as_deref().unwrap_or("unknown"),
            signing_field
        );
    };
    let signature_bytes = match config {
        AuthSignerConfig::HmacQueryV1(_) => {
            hmac_sha256(&signing_secret, canonical_query.as_bytes())
        }
        AuthSignerConfig::Ed25519QueryV1(_) => {
            ed25519_sign(&signing_secret, canonical_query.as_bytes())?
        }
    };
    let signature = encode_signature(&signature_bytes, signature_encoding);

    pairs.push((signature_param.clone(), signature));
    let final_pairs = canonicalize_pairs(&pairs, canonicalization_mode);
    let final_query = encode_query_pairs(&final_pairs);
    parsed.set_query(Some(&final_query));
    Ok(parsed.to_string())
}

/// Apply authentication to a reqwest request builder.
#[allow(dead_code)]
pub fn apply_auth_to_request(
    request_builder: reqwest::RequestBuilder,
    auth_type: &AuthType,
    api_key: &str,
) -> reqwest::RequestBuilder {
    match auth_type {
        AuthType::Bearer => request_builder.bearer_auth(api_key),
        AuthType::ApiKey => request_builder.header("X-API-Key", api_key),
        AuthType::Basic => {
            let parts: Vec<&str> = api_key.splitn(2, ':').collect();
            if parts.len() == 2 {
                request_builder.basic_auth(parts[0], Some(parts[1]))
            } else {
                request_builder.basic_auth(api_key, Option::<&str>::None)
            }
        }
        AuthType::OAuth => request_builder.bearer_auth(api_key),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRequestAuth {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

fn resolved_profile_auth_headers(profile: &Profile) -> Result<Vec<(String, String)>> {
    let mut headers = match profile.auth_type {
        AuthType::Bearer | AuthType::OAuth => {
            vec![("authorization".to_string(), format!("Bearer {}", profile.api_key))]
        }
        AuthType::ApiKey => profile.resolved_api_key_headers()?,
        AuthType::Basic => {
            use base64::Engine;

            let encoded = base64::engine::general_purpose::STANDARD.encode(&profile.api_key);
            vec![("authorization".to_string(), format!("Basic {}", encoded))]
        }
    };

    if let Some((name, value)) = signer_header(profile)? {
        headers.push((name, value));
    }

    Ok(headers)
}

pub fn resolve_profile_request_auth(url: &str, profile: &Profile) -> Result<ResolvedRequestAuth> {
    Ok(ResolvedRequestAuth {
        url: apply_profile_auth_to_url(url, profile)?,
        headers: resolved_profile_auth_headers(profile)?,
    })
}

/// Apply authentication from a credential profile to a reqwest request builder.
pub fn apply_profile_auth_to_request(
    mut request_builder: reqwest::RequestBuilder,
    profile: &Profile,
) -> Result<reqwest::RequestBuilder> {
    for (name, value) in resolved_profile_auth_headers(profile)? {
        request_builder = request_builder.header(name, value);
    }

    Ok(request_builder)
}

pub fn apply_profile_auth_to_url(url: &str, profile: &Profile) -> Result<String> {
    let mut authed_url = url.to_string();
    if profile.auth_type == AuthType::ApiKey && profile.has_custom_api_key_query_params() {
        let mut parsed = url::Url::parse(&authed_url)
            .with_context(|| format!("Invalid endpoint URL for auth query params: {}", url))?;
        let rendered = profile.resolved_api_key_query_params()?;
        if !rendered.is_empty() {
            let mut pairs: Vec<(String, String)> = parsed
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            pairs.retain(|(key, _)| !rendered.iter().any(|(name, _)| name == key));
            pairs.extend(rendered);

            parsed.query_pairs_mut().clear().extend_pairs(pairs);
            authed_url = parsed.to_string();
        }
    }

    apply_signer_to_url(&authed_url, profile)
}

pub fn auth_profile_to_metadata(
    profile: &Profile,
) -> Result<tonic::metadata::MetadataMap, anyhow::Error> {
    use base64::Engine;

    let mut metadata = tonic::metadata::MetadataMap::new();

    match profile.auth_type {
        AuthType::Bearer | AuthType::OAuth => {
            let value =
                tonic::metadata::MetadataValue::try_from(&format!("Bearer {}", profile.api_key))
                    .map_err(|_| {
                        anyhow::anyhow!("Invalid token: contains invalid metadata characters")
                    })?;
            metadata.insert("authorization", value);
        }
        AuthType::ApiKey => {
            for (name, value) in profile.resolved_api_key_headers()? {
                let key =
                    tonic::metadata::MetadataKey::from_bytes(name.as_bytes()).map_err(|_| {
                        anyhow::anyhow!("Invalid metadata key '{}' for api_key auth", name)
                    })?;
                let value =
                    tonic::metadata::MetadataValue::try_from(value.as_str()).map_err(|_| {
                        anyhow::anyhow!(
                            "Invalid API key value: contains invalid metadata characters"
                        )
                    })?;
                metadata.insert(key, value);
            }
        }
        AuthType::Basic => {
            let parts: Vec<&str> = profile.api_key.splitn(2, ':').collect();
            let creds = if parts.len() == 2 {
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", parts[0], parts[1]))
            } else {
                base64::engine::general_purpose::STANDARD.encode(format!("{}:", profile.api_key))
            };
            let value = tonic::metadata::MetadataValue::try_from(&format!("Basic {}", creds))
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Invalid Basic auth credentials: contains invalid metadata characters"
                    )
                })?;
            metadata.insert("authorization", value);
        }
    }

    Ok(metadata)
}

/// Convert auth credential to tonic metadata map for gRPC.
#[allow(dead_code)]
pub fn auth_to_metadata(
    auth_type: &AuthType,
    api_key: &str,
) -> Result<tonic::metadata::MetadataMap, anyhow::Error> {
    let profile = Profile::new(api_key.to_string(), auth_type.clone());
    auth_profile_to_metadata(&profile)
}

/// Get home directory.
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home));
        }

        #[cfg(windows)]
        {
            if let Some(user_profile) = std::env::var_os("USERPROFILE") {
                return Some(PathBuf::from(user_profile));
            }

            if let Some(home_drive) = std::env::var_os("HOMEDRIVE") {
                if let Some(home_path) = std::env::var_os("HOMEPATH") {
                    let mut path = PathBuf::from(&home_drive);
                    path.push(&home_path);
                    return Some(path);
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // Test-only Ed25519 key for signer unit tests. Never used in production.
    const TEST_ED25519_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIFM839r6dC+FQ4zIFCwogFaLq3ugAG4cxQWjSZZu6Cxl\n-----END PRIVATE KEY-----";

    #[test]
    fn test_profile_default() {
        let profile = Profile::new("test-key".to_string(), AuthType::Bearer);
        assert_eq!(profile.api_key, "test-key");
        assert_eq!(profile.auth_type, AuthType::Bearer);
        assert!(profile.description.is_none());
    }

    #[test]
    fn test_profile_with_description() {
        let profile = Profile::new("test-key".to_string(), AuthType::ApiKey)
            .with_description("Test profile".to_string());
        assert_eq!(profile.description, Some("Test profile".to_string()));
    }

    #[test]
    fn test_mask_api_key() {
        let profile = Profile::new("sk-12345678abcdefgh".to_string(), AuthType::Bearer);
        assert_eq!(profile.mask_api_key(), "sk-12345...efgh");
    }

    #[test]
    fn test_mask_short_api_key() {
        let profile = Profile::new("short".to_string(), AuthType::Bearer);
        assert_eq!(profile.mask_api_key(), "*****");
    }

    #[test]
    fn test_auth_type_from_str() {
        assert_eq!(AuthType::from_str("bearer").unwrap(), AuthType::Bearer);
        assert_eq!(AuthType::from_str("BEARER").unwrap(), AuthType::Bearer);
        assert_eq!(AuthType::from_str("api_key").unwrap(), AuthType::ApiKey);
        assert_eq!(AuthType::from_str("basic").unwrap(), AuthType::Basic);
        assert_eq!(AuthType::from_str("oauth").unwrap(), AuthType::OAuth);
        assert!(AuthType::from_str("invalid").is_err());
    }

    #[test]
    fn test_profiles_new() {
        let profiles = Profiles::new();
        assert_eq!(profiles.count(), 0);
        assert!(!profiles.has_profile("default"));
    }

    #[test]
    fn test_profiles_set_get() {
        let mut profiles = Profiles::new();
        let profile = Profile::new("test-key".to_string(), AuthType::Bearer);

        profiles
            .set_profile("default".to_string(), profile)
            .unwrap();
        assert!(profiles.has_profile("default"));
        assert_eq!(profiles.count(), 1);

        let retrieved = profiles.get_profile("default").unwrap();
        assert_eq!(retrieved.api_key, "test-key");
    }

    #[test]
    fn test_profiles_remove() {
        let mut profiles = Profiles::new();
        let profile = Profile::new("test-key".to_string(), AuthType::Bearer);

        profiles
            .set_profile("default".to_string(), profile)
            .unwrap();
        assert!(profiles.has_profile("default"));

        profiles.remove_profile("default").unwrap();
        assert!(!profiles.has_profile("default"));
        assert_eq!(profiles.count(), 0);
    }

    #[test]
    fn test_profiles_list_names() {
        let mut profiles = Profiles::new();
        profiles
            .set_profile(
                "dev".to_string(),
                Profile::new("key1".to_string(), AuthType::Bearer),
            )
            .unwrap();
        profiles
            .set_profile(
                "prod".to_string(),
                Profile::new("key2".to_string(), AuthType::Bearer),
            )
            .unwrap();

        let names = profiles.profile_names();
        assert_eq!(names, vec!["dev", "prod"]);
        assert_eq!(profiles.list_names(), "dev, prod");
    }

    #[test]
    fn binding_priority_prefers_higher_priority_then_longer_path() {
        let bindings = AuthBindings {
            bindings: vec![
                AuthBindingRule {
                    id: "root".to_string(),
                    host: "api.example.com".to_string(),
                    path_prefix: Some("/".to_string()),
                    scheme: Some("https".to_string()),
                    credential: "a".to_string(),
                    signer: None,
                    priority: 10,
                    enabled: true,
                },
                AuthBindingRule {
                    id: "admin".to_string(),
                    host: "api.example.com".to_string(),
                    path_prefix: Some("/admin".to_string()),
                    scheme: Some("https".to_string()),
                    credential: "b".to_string(),
                    signer: None,
                    priority: 10,
                    enabled: true,
                },
                AuthBindingRule {
                    id: "priority".to_string(),
                    host: "api.example.com".to_string(),
                    path_prefix: Some("/admin".to_string()),
                    scheme: Some("https".to_string()),
                    credential: "c".to_string(),
                    signer: None,
                    priority: 20,
                    enabled: true,
                },
            ],
        };

        let matched = bindings
            .matching_rule("https://api.example.com/admin/users")
            .unwrap();
        assert_eq!(matched.id, "priority");
    }

    #[test]
    fn auth_header_parse_and_render_secret_template() {
        let header = AuthHeader::parse("OK-ACCESS-KEY={{secret}}").unwrap();
        let profile = Profile::new("secret-value".to_string(), AuthType::ApiKey);
        assert_eq!(header.name, "OK-ACCESS-KEY");
        assert_eq!(header.render_value(&profile).unwrap(), "secret-value");
    }

    #[test]
    fn auth_header_render_env_template() {
        std::env::set_var("UXC_TEST_ENV_TEMPLATE", "tenant-1");
        let header = AuthHeader::parse("X-Tenant={{env:UXC_TEST_ENV_TEMPLATE}}").unwrap();
        let profile = Profile::new("unused".to_string(), AuthType::ApiKey);
        assert_eq!(header.render_value(&profile).unwrap(), "tenant-1");
        std::env::remove_var("UXC_TEST_ENV_TEMPLATE");
    }

    #[test]
    fn auth_header_render_field_template() {
        let header = AuthHeader::parse("X-Signing-Key={{field:secret_key}}").unwrap();
        let mut profile = Profile::new("unused".to_string(), AuthType::ApiKey);
        profile
            .set_field_source(
                "secret_key".to_string(),
                SecretSource::Literal {
                    value: "signing-secret".to_string(),
                },
            )
            .unwrap();
        assert_eq!(header.render_value(&profile).unwrap(), "signing-secret");
    }

    #[test]
    fn auth_header_invalid_template_token() {
        let err = AuthHeader::parse("x-test={{unknown}}").unwrap_err();
        assert!(err.to_string().contains("Unsupported template token"));
    }

    #[test]
    fn auth_query_param_parse_and_render_secret_template() {
        let param = AuthQueryParam::parse("apiKey={{secret}}").unwrap();
        let profile = Profile::new("secret-value".to_string(), AuthType::ApiKey);
        assert_eq!(param.name, "apiKey");
        assert_eq!(param.render_value(&profile).unwrap(), "secret-value");
    }

    #[test]
    fn auth_query_param_render_env_template() {
        std::env::set_var("UXC_TEST_QUERY_ENV", "workspace-1");
        let param = AuthQueryParam::parse("workspace={{env:UXC_TEST_QUERY_ENV}}").unwrap();
        let profile = Profile::new("unused".to_string(), AuthType::ApiKey);
        assert_eq!(param.render_value(&profile).unwrap(), "workspace-1");
        std::env::remove_var("UXC_TEST_QUERY_ENV");
    }

    #[test]
    fn apply_profile_auth_to_url_replaces_existing_query_param() {
        let mut profile = Profile::new("secret-value".to_string(), AuthType::ApiKey);
        profile.auth_query_params = Some(vec![AuthQueryParam::parse("apiKey={{secret}}").unwrap()]);

        let url = apply_profile_auth_to_url(
            "https://example.com/mcp?apiKey=old&network=ethereum",
            &profile,
        )
        .unwrap();
        assert!(url.contains("apiKey=secret-value"));
        assert!(url.contains("network=ethereum"));
        assert!(!url.contains("apiKey=old"));
    }

    #[test]
    fn resolved_api_key_headers_skip_default_when_query_params_present() {
        let mut profile = Profile::new("secret-value".to_string(), AuthType::ApiKey);
        profile.auth_query_params = Some(vec![AuthQueryParam::parse("apiKey={{secret}}").unwrap()]);
        assert!(profile.resolved_api_key_headers().unwrap().is_empty());
    }

    #[test]
    fn resolved_api_key_headers_skip_default_when_signer_is_present() {
        let mut profile = Profile::new(String::new(), AuthType::ApiKey);
        profile.signer = Some(AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret_key".to_string(),
            key_field: Some("api_key".to_string()),
            key_placement: Some(SignerValuePlacement::Header),
            key_name: Some("X-MBX-APIKEY".to_string()),
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization::default(),
        }));

        assert!(profile.resolved_api_key_headers().unwrap().is_empty());
    }

    #[test]
    fn parse_field_spec_supports_explicit_sources() {
        let (name, source) = parse_field_spec("api_key=env:BINANCE_API_KEY").unwrap();
        assert_eq!(name, "api_key");
        assert_eq!(
            source,
            SecretSource::Env {
                key: "BINANCE_API_KEY".to_string()
            }
        );
    }

    #[test]
    fn apply_profile_auth_to_url_signs_hmac_query_requests() {
        let mut profile = Profile::new("unused".to_string(), AuthType::ApiKey);
        profile
            .set_field_source(
                "secret_key".to_string(),
                SecretSource::Literal {
                    value: "super-secret".to_string(),
                },
            )
            .unwrap();
        profile.signer = Some(AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret_key".to_string(),
            key_field: Some("api_key".to_string()),
            key_placement: Some(SignerValuePlacement::Header),
            key_name: Some("X-MBX-APIKEY".to_string()),
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization {
                mode: QueryCanonicalizationMode::SortByKey,
            },
        }));

        let url = apply_profile_auth_to_url(
            "https://api.binance.com/api/v3/account?recvWindow=5000&symbol=BTCUSDT",
            &profile,
        )
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        let canonical = canonicalize_pairs(
            &pairs
                .iter()
                .filter(|(name, _)| name != "signature")
                .cloned()
                .collect::<Vec<_>>(),
            &QueryCanonicalizationMode::SortByKey,
        );
        let expected = encode_signature(
            &hmac_sha256("super-secret", encode_query_pairs(&canonical).as_bytes()),
            &SignatureEncoding::Hex,
        );
        assert!(url.contains(&format!("signature={}", expected)));
    }

    #[test]
    fn render_secret_template_rejects_empty_secret() {
        let header = AuthHeader::parse("X-API-Key={{secret}}").unwrap();
        let mut profile = Profile::new(String::new(), AuthType::ApiKey);
        profile.secret_source = Some(SecretSource::Literal {
            value: String::new(),
        });

        let err = header.render_value(&profile).unwrap_err();
        assert!(err.to_string().contains("non-empty secret"));
    }

    #[test]
    fn validate_signer_config_rejects_invalid_field_name() {
        let err = validate_signer_config(&AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret key".to_string(),
            key_field: None,
            key_placement: None,
            key_name: None,
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization::default(),
        }))
        .unwrap_err();

        assert!(err.to_string().contains("Invalid field name"));
    }

    #[test]
    fn validate_signer_config_rejects_invalid_signature_param() {
        let err = validate_signer_config(&AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret_key".to_string(),
            key_field: None,
            key_placement: None,
            key_name: None,
            signature_param: "bad=name".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization::default(),
        }))
        .unwrap_err();

        assert!(err.to_string().contains("Invalid query param name"));
    }

    #[test]
    fn validate_signer_config_requires_complete_key_injection_triplet() {
        let err = validate_signer_config(&AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret_key".to_string(),
            key_field: Some("api_key".to_string()),
            key_placement: Some(SignerValuePlacement::Header),
            key_name: None,
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization::default(),
        }))
        .unwrap_err();

        assert!(err.to_string().contains("set together"));
    }

    #[test]
    fn validate_ready_allows_signer_only_api_key_credentials() {
        let mut profile = Profile::new(String::new(), AuthType::ApiKey);
        profile
            .set_field_source(
                "secret_key".to_string(),
                SecretSource::Literal {
                    value: "super-secret".to_string(),
                },
            )
            .unwrap();
        profile.signer = Some(AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret_key".to_string(),
            key_field: None,
            key_placement: None,
            key_name: None,
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: Some("timestamp".to_string()),
            timestamp_unit: Some(TimestampUnit::Milliseconds),
            canonicalization: QueryCanonicalization::default(),
        }));

        validate_ready(&profile).unwrap();
    }

    #[test]
    fn apply_profile_auth_to_url_signs_ed25519_query_requests() {
        let mut profile = Profile::new("unused".to_string(), AuthType::ApiKey);
        profile
            .set_field_source(
                "private_key".to_string(),
                SecretSource::Literal {
                    value: TEST_ED25519_PRIVATE_KEY_PEM.to_string(),
                },
            )
            .unwrap();
        profile.signer = Some(AuthSignerConfig::Ed25519QueryV1(Ed25519QuerySignerConfig {
            algorithm: SignerAlgorithm::Ed25519,
            signing_field: "private_key".to_string(),
            key_field: Some("api_key".to_string()),
            key_placement: Some(SignerValuePlacement::Header),
            key_name: Some("X-MBX-APIKEY".to_string()),
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Base64,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization {
                mode: QueryCanonicalizationMode::SortByKey,
            },
        }));

        let url = apply_profile_auth_to_url(
            "https://api.binance.com/api/v3/account?recvWindow=5000&symbol=BTCUSDT",
            &profile,
        )
        .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        let canonical = canonicalize_pairs(
            &pairs
                .iter()
                .filter(|(name, _)| name != "signature")
                .cloned()
                .collect::<Vec<_>>(),
            &QueryCanonicalizationMode::SortByKey,
        );
        let expected = encode_signature(
            &ed25519_sign(
                TEST_ED25519_PRIVATE_KEY_PEM,
                encode_query_pairs(&canonical).as_bytes(),
            )
            .unwrap(),
            &SignatureEncoding::Base64,
        );
        let actual = pairs
            .iter()
            .find(|(name, _)| name == "signature")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn validate_signer_config_rejects_ed25519_non_base64_encoding() {
        let err = validate_signer_config(&AuthSignerConfig::Ed25519QueryV1(
            Ed25519QuerySignerConfig {
                algorithm: SignerAlgorithm::Ed25519,
                signing_field: "private_key".to_string(),
                key_field: Some("api_key".to_string()),
                key_placement: Some(SignerValuePlacement::Header),
                key_name: Some("X-MBX-APIKEY".to_string()),
                signature_param: "signature".to_string(),
                signature_encoding: SignatureEncoding::Hex,
                timestamp_param: None,
                timestamp_unit: None,
                canonicalization: QueryCanonicalization::default(),
            },
        ))
        .unwrap_err();

        assert!(err.to_string().contains("signature_encoding=base64"));
    }

    #[test]
    fn resolved_api_key_headers_skip_default_when_ed25519_signer_is_present() {
        let mut profile = Profile::new(String::new(), AuthType::ApiKey);
        profile.signer = Some(AuthSignerConfig::Ed25519QueryV1(Ed25519QuerySignerConfig {
            algorithm: SignerAlgorithm::Ed25519,
            signing_field: "private_key".to_string(),
            key_field: Some("api_key".to_string()),
            key_placement: Some(SignerValuePlacement::Header),
            key_name: Some("X-MBX-APIKEY".to_string()),
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Base64,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization::default(),
        }));

        assert!(profile.resolved_api_key_headers().unwrap().is_empty());
    }

    #[test]
    fn resolve_profile_request_auth_combines_query_and_headers() {
        let mut profile = Profile::new("token-123".to_string(), AuthType::ApiKey);
        profile.auth_headers = Some(vec![AuthHeader::new("X-Test-Key", "{{secret}}").unwrap()]);
        profile.auth_query_params = Some(vec![AuthQueryParam::new("api_key", "{{secret}}").unwrap()]);

        let resolved =
            resolve_profile_request_auth("https://example.com/ws?existing=1", &profile).unwrap();

        assert!(resolved.url.contains("existing=1"));
        assert!(resolved.url.contains("api_key=token-123"));
        assert_eq!(
            resolved.headers,
            vec![("X-Test-Key".to_string(), "token-123".to_string())]
        );
    }

    #[test]
    fn resolve_profile_request_auth_includes_signer_header_and_signed_query() {
        let mut profile = Profile::new("unused".to_string(), AuthType::ApiKey);
        profile
            .set_field_source(
                "secret_key".to_string(),
                SecretSource::Literal {
                    value: "super-secret".to_string(),
                },
            )
            .unwrap();
        profile
            .set_field_source(
                "api_key".to_string(),
                SecretSource::Literal {
                    value: "key-123".to_string(),
                },
            )
            .unwrap();
        profile.signer = Some(AuthSignerConfig::HmacQueryV1(HmacQuerySignerConfig {
            algorithm: SignerAlgorithm::HmacSha256,
            signing_field: "secret_key".to_string(),
            key_field: Some("api_key".to_string()),
            key_placement: Some(SignerValuePlacement::Header),
            key_name: Some("X-MBX-APIKEY".to_string()),
            signature_param: "signature".to_string(),
            signature_encoding: SignatureEncoding::Hex,
            timestamp_param: None,
            timestamp_unit: None,
            canonicalization: QueryCanonicalization::default(),
        }));

        let resolved = resolve_profile_request_auth(
            "https://api.binance.com/ws?symbol=BTCUSDT",
            &profile,
        )
        .unwrap();

        assert!(resolved.url.contains("signature="));
        assert_eq!(
            resolved.headers,
            vec![("X-MBX-APIKEY".to_string(), "key-123".to_string())]
        );
    }
}
