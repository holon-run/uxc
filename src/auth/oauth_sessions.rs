use super::{auth_base_dir, write_secure_auth_file};
use crate::auth::oauth::OAuthProviderMetadata;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const OAUTH_SESSION_DIR_ENV: &str = "UXC_OAUTH_SESSION_DIR";
const OAUTH_SESSION_DIR_NAME: &str = "oauth_sessions";
pub const DEFAULT_SESSION_TTL_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAuthorizationCodeSession {
    pub version: u32,
    pub session_id: String,
    pub credential_id: String,
    pub endpoint: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub metadata: OAuthProviderMetadata,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_registration_issuer: Option<String>,
    pub state: String,
    pub code_verifier: String,
    pub created_at: i64,
    pub expires_at: i64,
}

impl PendingAuthorizationCodeSession {
    pub fn is_expired(&self, now_unix: i64) -> bool {
        now_unix >= self.expires_at
    }
}

pub fn session_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(OAUTH_SESSION_DIR_ENV) {
        return Ok(PathBuf::from(path));
    }

    Ok(auth_base_dir()?.join(OAUTH_SESSION_DIR_NAME))
}

fn session_path_with_dir(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{}.json", session_id))
}

pub fn session_path(session_id: &str) -> Result<PathBuf> {
    validate_session_id(session_id)?;
    Ok(session_path_with_dir(&session_dir()?, session_id))
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() {
        anyhow::bail!("OAuth session ID cannot be empty");
    }

    if !session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!(
            "OAuth session ID '{}' contains invalid characters. Allowed: letters, digits, '_', '-'",
            session_id
        );
    }

    Ok(())
}

pub fn save_session(session: &PendingAuthorizationCodeSession) -> Result<()> {
    let path = session_path(&session.session_id)?;
    let contents =
        serde_json::to_string_pretty(session).context("Failed to serialize OAuth session")?;
    write_secure_auth_file(&path, &contents, "oauth session")
}

pub fn load_session(session_id: &str) -> Result<PendingAuthorizationCodeSession> {
    let path = session_path(session_id)?;
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read OAuth session file: {:?}", path))?;
    serde_json::from_str(&contents).context("Failed to parse OAuth session")
}

pub fn remove_session(session_id: &str) -> Result<()> {
    let path = session_path(session_id)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("Failed to remove OAuth session: {:?}", path)),
    }
}

pub fn purge_expired_sessions(now_unix: i64) -> Result<usize> {
    let dir = session_dir()?;
    if !dir.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("Failed to read OAuth session dir: {:?}", dir))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let should_remove = match fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str::<PendingAuthorizationCodeSession>(&contents) {
                    Ok(session) => session.is_expired(now_unix),
                    Err(_) => true,
                }
            }
            Err(_) => true,
        };

        if should_remove {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("Failed to remove OAuth session: {:?}", path));
                }
            }
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::oauth::OAuthProviderMetadata;
    use serial_test::serial;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct SessionDirGuard {
        previous: Option<OsString>,
        temp_dir: TempDir,
    }

    impl SessionDirGuard {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().expect("temp dir");
            let previous = std::env::var_os(OAUTH_SESSION_DIR_ENV);
            std::env::set_var(OAUTH_SESSION_DIR_ENV, temp_dir.path());
            Self { previous, temp_dir }
        }

        fn path(&self) -> &Path {
            self.temp_dir.path()
        }
    }

    impl Drop for SessionDirGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(OAUTH_SESSION_DIR_ENV, value),
                None => std::env::remove_var(OAUTH_SESSION_DIR_ENV),
            }
        }
    }

    fn sample_session(id: &str, expires_at: i64) -> PendingAuthorizationCodeSession {
        PendingAuthorizationCodeSession {
            version: 1,
            session_id: id.to_string(),
            credential_id: "cred".to_string(),
            endpoint: "https://example.com/mcp".to_string(),
            redirect_uri: "http://127.0.0.1/callback".to_string(),
            scopes: vec!["openid".to_string()],
            metadata: OAuthProviderMetadata {
                provider_issuer: Some("https://issuer.example.com".to_string()),
                authorization_response_iss_parameter_supported: false,
                resource_metadata_url: None,
                authorization_server: None,
                authorization_endpoint: Some("https://issuer.example.com/authorize".to_string()),
                registration_endpoint: None,
                token_endpoint: "https://issuer.example.com/token".to_string(),
                device_authorization_endpoint: None,
            },
            client_id: "client-id".to_string(),
            client_secret: None,
            client_registration_issuer: None,
            state: "state".to_string(),
            code_verifier: "verifier".to_string(),
            created_at: 100,
            expires_at,
        }
    }

    #[test]
    #[serial]
    fn session_dir_uses_env_override() {
        let dir = SessionDirGuard::new();
        assert_eq!(session_dir().unwrap(), dir.path());
    }

    #[test]
    #[serial]
    fn save_and_load_session_round_trip() {
        let _dir = SessionDirGuard::new();
        let session = sample_session("abc", 1000);
        save_session(&session).unwrap();

        let loaded = load_session("abc").unwrap();
        assert_eq!(loaded.session_id, "abc");
        assert_eq!(loaded.client_id, "client-id");
    }

    #[test]
    #[serial]
    fn purge_expired_sessions_removes_only_expired_entries() {
        let _dir = SessionDirGuard::new();
        save_session(&sample_session("expired", 100)).unwrap();
        save_session(&sample_session("active", 1000)).unwrap();

        let removed = purge_expired_sessions(500).unwrap();
        assert_eq!(removed, 1);
        assert!(load_session("expired").is_err());
        assert!(load_session("active").is_ok());
    }

    #[test]
    fn session_path_rejects_invalid_session_ids() {
        assert!(session_path("../escape").is_err());
        assert!(session_path("bad/name").is_err());
        assert!(session_path("bad.name").is_err());
        assert!(session_path("").is_err());
    }
}
