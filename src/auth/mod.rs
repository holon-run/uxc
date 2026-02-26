//! Authentication profile storage module
//!
//! Provides filesystem-based storage for authentication profiles.
//! Profiles are stored in ~/.uxc/profiles.toml in plain text (encryption in Phase 2).
//!
//! # Profile Structure
//!
//! ```toml
//! [default]
//! api_key = "sk-..."
//! auth_type = "bearer"
//!
//! [production]
//! api_key = "sk-prod-..."
//! auth_type = "bearer"
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub mod oauth;

/// Default profiles directory relative to home directory
pub const DEFAULT_PROFILES_DIR: &str = ".uxc";

/// Default profiles file name
pub const PROFILES_FILE: &str = "profiles.toml";

/// Authentication type
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

/// Authentication profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// API key or token
    pub api_key: String,

    /// Authentication type
    #[serde(default)]
    pub auth_type: AuthType,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthProfile>,

    #[serde(skip)]
    pub name: Option<String>,
}

impl Profile {
    /// Create a new profile
    pub fn new(api_key: String, auth_type: AuthType) -> Self {
        Self {
            api_key,
            auth_type,
            description: None,
            oauth: None,
            name: None,
        }
    }

    /// Create a new profile with description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    pub fn with_oauth(mut self, oauth: OAuthProfile) -> Self {
        self.oauth = Some(oauth);
        self
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
            AuthType::Bearer => Some(self.api_key.as_str()),
            AuthType::OAuth => self.oauth.as_ref()?.access_token.as_deref(),
            _ => None,
        }
    }

    fn active_secret_for_masking(&self) -> &str {
        if let Some(token) = self.bearer_token() {
            return token;
        }
        &self.api_key
    }
}

/// Profiles collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profiles {
    /// Map of profile name to strongly typed profile values.
    ///
    /// Keeping this as `Profile` (instead of an untyped JSON value) keeps TOML
    /// storage and runtime representation consistent and avoids mixed
    /// serialization semantics.
    #[serde(flatten)]
    pub profiles: HashMap<String, Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self::new()
    }
}

impl Profiles {
    /// Create a new empty profiles collection
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Get the profiles file path
    fn profiles_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(DEFAULT_PROFILES_DIR).join(PROFILES_FILE))
    }

    /// Load profiles from ~/.uxc/profiles.toml
    ///
    /// If the file doesn't exist, returns an empty profiles collection.
    pub fn load_profiles() -> Result<Self> {
        let path = Self::profiles_path()?;

        if !path.exists() {
            return Ok(Self::new());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read profiles file: {:?}", path))?;

        // Deserialize directly from TOML - #[serde(flatten)] handles the structure
        toml::from_str(&contents)
            .with_context(|| format!("Failed to parse profiles file: {:?}", path))
    }

    /// Save profiles to ~/.uxc/profiles.toml
    pub fn save_profiles(&self) -> Result<()> {
        let path = Self::profiles_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create profiles directory: {:?}", parent)
                })?;
            }
        }

        // Serialize directly to TOML - #[serde(flatten)] will handle the structure
        let toml_string =
            toml::to_string_pretty(&self).context("Failed to serialize profiles to TOML")?;

        fs::write(&path, toml_string)
            .with_context(|| format!("Failed to write profiles file: {:?}", path))?;

        Ok(())
    }

    /// Get a profile by name
    pub fn get_profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).context(format!(
            "Profile '{}' not found. Available profiles: {}",
            name,
            self.list_names()
        ))
    }

    /// Validate a profile name for TOML compatibility
    ///
    /// Restricts profile names to safe characters for TOML table names.
    /// Allows only ASCII letters, digits, underscores, and hyphens.
    fn validate_profile_name(name: &str) -> Result<()> {
        if name.is_empty() {
            anyhow::bail!("Profile name cannot be empty");
        }

        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            anyhow::bail!(
                "Profile name '{}' contains invalid characters. Allowed characters: letters, digits, '_', '-'",
                name
            );
        }

        // Ensure name doesn't start with a digit (TOML table names shouldn't)
        if name
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            anyhow::bail!("Profile name '{}' cannot start with a digit", name);
        }

        Ok(())
    }

    /// Set a profile
    pub fn set_profile(&mut self, name: String, profile: Profile) -> Result<()> {
        // Validate profile name before inserting
        Self::validate_profile_name(&name)?;

        self.profiles.insert(name, profile);
        Ok(())
    }

    /// Remove a profile
    pub fn remove_profile(&mut self, name: &str) -> Result<()> {
        self.profiles
            .remove(name)
            .context(format!("Profile '{}' not found", name))?;
        Ok(())
    }

    /// List all profile names
    pub fn list_names(&self) -> String {
        let mut names: Vec<_> = self.profiles.keys().cloned().collect();
        names.sort();
        names.join(", ")
    }

    /// Get all profile names
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.profiles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Check if a profile exists
    pub fn has_profile(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
    }

    /// Get the number of profiles
    pub fn count(&self) -> usize {
        self.profiles.len()
    }
}

/// Apply authentication to a reqwest request builder
///
/// This function applies the appropriate authentication headers based on the AuthType.
pub fn apply_auth_to_request(
    request_builder: reqwest::RequestBuilder,
    auth_type: &AuthType,
    api_key: &str,
) -> reqwest::RequestBuilder {
    match auth_type {
        AuthType::Bearer => request_builder.bearer_auth(api_key),
        AuthType::ApiKey => request_builder.header("X-API-Key", api_key),
        AuthType::Basic => {
            // For basic auth, api_key should be in format "username:password"
            // Split on first colon only
            let parts: Vec<&str> = api_key.splitn(2, ':').collect();
            if parts.len() == 2 {
                request_builder.basic_auth(parts[0], Some(parts[1]))
            } else {
                // If no colon, treat as username with no password
                request_builder.basic_auth(api_key, Option::<&str>::None)
            }
        }
        AuthType::OAuth => request_builder.bearer_auth(api_key),
    }
}

/// Convert auth profile to tonic metadata map for gRPC
///
/// This function creates a metadata map with appropriate authentication headers.
/// Returns an error if the auth credentials cannot be encoded as valid gRPC metadata.
#[allow(dead_code)]
pub fn auth_to_metadata(
    auth_type: &AuthType,
    api_key: &str,
) -> Result<tonic::metadata::MetadataMap, anyhow::Error> {
    use base64::Engine;

    let mut metadata = tonic::metadata::MetadataMap::new();

    match auth_type {
        AuthType::Bearer => {
            let value = tonic::metadata::MetadataValue::try_from(&format!("Bearer {}", api_key))
                .map_err(|_| {
                    anyhow::anyhow!("Invalid Bearer token: contains invalid metadata characters")
                })?;
            metadata.insert("authorization", value);
        }
        AuthType::ApiKey => {
            let value = tonic::metadata::MetadataValue::try_from(api_key).map_err(|_| {
                anyhow::anyhow!("Invalid API key: contains invalid metadata characters")
            })?;
            metadata.insert("x-api-key", value);
        }
        AuthType::Basic => {
            // For basic auth, api_key should be in format "username:password"
            let parts: Vec<&str> = api_key.splitn(2, ':').collect();
            let creds = if parts.len() == 2 {
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", parts[0], parts[1]))
            } else {
                base64::engine::general_purpose::STANDARD.encode(format!("{}:", api_key))
            };
            let value = tonic::metadata::MetadataValue::try_from(&format!("Basic {}", creds))
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Invalid Basic auth credentials: contains invalid metadata characters"
                    )
                })?;
            metadata.insert("authorization", value);
        }
        AuthType::OAuth => {
            let value = tonic::metadata::MetadataValue::try_from(&format!("Bearer {}", api_key))
                .map_err(|_| {
                    anyhow::anyhow!("Invalid OAuth token: contains invalid metadata characters")
                })?;
            metadata.insert("authorization", value);
        }
    }

    Ok(metadata)
}

/// Get the home directory
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        // Try HOME environment variable first (Unix-like systems)
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home));
        }

        // Try USERPROFILE on Windows
        #[cfg(windows)]
        {
            if let Some(user_profile) = std::env::var_os("USERPROFILE") {
                return Some(PathBuf::from(user_profile));
            }

            // Fallback to HOMEDRIVE + HOMEPATH on Windows
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
}
