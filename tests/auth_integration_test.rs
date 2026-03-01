//! Integration tests for authentication credential storage
//!
//! These tests verify that the credential storage system works correctly
//! including file I/O, JSON parsing, and credential management.

use std::fs;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;
use uxc::auth::{AuthType, Profile, Profiles};

fn home_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct TestEnv {
    temp_dir: TempDir,
    _home_guard: MutexGuard<'static, ()>,
    previous_home: Option<std::ffi::OsString>,
    previous_path: Option<std::ffi::OsString>,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        match &self.previous_home {
            Some(prev) => std::env::set_var("HOME", prev),
            None => std::env::remove_var("HOME"),
        }
        match &self.previous_path {
            Some(prev) => std::env::set_var("PATH", prev),
            None => std::env::remove_var("PATH"),
        }
    }
}

/// Helper function to create a test environment with a temporary directory.
/// Uses a process-wide lock to avoid concurrent HOME mutations across tests.
fn setup_test_env() -> TestEnv {
    let guard = home_env_lock()
        .lock()
        .expect("Failed to lock HOME env guard");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let previous_home = std::env::var_os("HOME");
    let previous_path = std::env::var_os("PATH");

    // Set HOME to the temp directory for testing
    std::env::set_var("HOME", temp_dir.path());

    TestEnv {
        temp_dir,
        _home_guard: guard,
        previous_home,
        previous_path,
    }
}

#[test]
fn test_load_empty_profiles() {
    let _temp_dir = setup_test_env();

    // Loading profiles when none exist should return empty collection
    let profiles = Profiles::load_profiles().expect("Failed to load profiles");
    assert_eq!(profiles.count(), 0);
}

#[test]
fn test_save_and_load_profiles() {
    let temp_dir = setup_test_env();

    // Create and save profiles
    let mut profiles = Profiles::new();

    let default_profile = Profile::new("sk-test-default-key".to_string(), AuthType::Bearer);
    profiles
        .set_profile("default".to_string(), default_profile)
        .expect("Failed to set profile");

    let prod_profile = Profile::new("sk-prod-key-12345".to_string(), AuthType::Bearer)
        .with_description("Production environment".to_string());
    profiles
        .set_profile("production".to_string(), prod_profile)
        .expect("Failed to set profile");

    profiles.save_profiles().expect("Failed to save profiles");

    // Verify the file was created
    let profiles_path = temp_dir.temp_dir.path().join(".uxc/credentials.json");
    assert!(profiles_path.exists(), "Credentials file should exist");

    // Load profiles and verify
    let loaded_profiles = Profiles::load_profiles().expect("Failed to load profiles");
    assert_eq!(loaded_profiles.count(), 2);
    assert!(loaded_profiles.has_profile("default"));
    assert!(loaded_profiles.has_profile("production"));

    // Verify profile content
    let default = loaded_profiles
        .get_profile("default")
        .expect("Failed to get default profile");
    assert_eq!(default.api_key, "sk-test-default-key");
    assert_eq!(default.auth_type, AuthType::Bearer);

    let production = loaded_profiles
        .get_profile("production")
        .expect("Failed to get production profile");
    assert_eq!(production.api_key, "sk-prod-key-12345");
    assert_eq!(
        production.description,
        Some("Production environment".to_string())
    );
}

#[test]
fn test_set_and_get_profile() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("my-api-key".to_string(), AuthType::ApiKey);

    profiles
        .set_profile("test".to_string(), profile)
        .expect("Failed to set profile");

    assert!(profiles.has_profile("test"));
    assert_eq!(profiles.count(), 1);

    let retrieved = profiles.get_profile("test").expect("Failed to get profile");
    assert_eq!(retrieved.api_key, "my-api-key");
    assert_eq!(retrieved.auth_type, AuthType::ApiKey);
}

#[test]
fn test_remove_profile() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("test-key".to_string(), AuthType::Bearer);

    profiles
        .set_profile("temp".to_string(), profile)
        .expect("Failed to set profile");
    assert!(profiles.has_profile("temp"));

    profiles
        .remove_profile("temp")
        .expect("Failed to remove profile");
    assert!(!profiles.has_profile("temp"));
    assert_eq!(profiles.count(), 0);
}

#[test]
fn test_remove_nonexistent_profile() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let result = profiles.remove_profile("nonexistent");

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_get_nonexistent_profile() {
    let _temp_dir = setup_test_env();

    let profiles = Profiles::new();
    let result = profiles.get_profile("nonexistent");

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_profile_names_list() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();

    profiles
        .set_profile(
            "zebra".to_string(),
            Profile::new("key1".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set profile");
    profiles
        .set_profile(
            "alpha".to_string(),
            Profile::new("key2".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set profile");
    profiles
        .set_profile(
            "beta".to_string(),
            Profile::new("key3".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set profile");

    let names = profiles.profile_names();
    assert_eq!(names, vec!["alpha", "beta", "zebra"]);
    assert_eq!(profiles.list_names(), "alpha, beta, zebra");
}

#[test]
fn test_profile_auth_types() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();

    profiles
        .set_profile(
            "bearer_profile".to_string(),
            Profile::new("bearer-key".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set profile");

    profiles
        .set_profile(
            "apikey_profile".to_string(),
            Profile::new("apikey-key".to_string(), AuthType::ApiKey),
        )
        .expect("Failed to set profile");

    profiles
        .set_profile(
            "basic_profile".to_string(),
            Profile::new("basic-key".to_string(), AuthType::Basic),
        )
        .expect("Failed to set profile");

    profiles.save_profiles().expect("Failed to save profiles");

    // Reload and verify
    let loaded = Profiles::load_profiles().expect("Failed to load profiles");

    assert_eq!(
        loaded.get_profile("bearer_profile").unwrap().auth_type,
        AuthType::Bearer
    );
    assert_eq!(
        loaded.get_profile("apikey_profile").unwrap().auth_type,
        AuthType::ApiKey
    );
    assert_eq!(
        loaded.get_profile("basic_profile").unwrap().auth_type,
        AuthType::Basic
    );
}

#[test]
fn test_json_format() {
    let temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("sk-test-1234".to_string(), AuthType::Bearer);
    profiles
        .set_profile("default".to_string(), profile)
        .expect("Failed to set profile");

    profiles.save_profiles().expect("Failed to save profiles");

    // Read and verify JSON format
    let profiles_path = temp_dir.temp_dir.path().join(".uxc/credentials.json");
    let contents = fs::read_to_string(&profiles_path).expect("Failed to read profiles file");

    // Verify the format contains expected elements
    assert!(contents.contains("\"version\": 1"));
    assert!(contents.contains("\"credentials\""));
    assert!(contents.contains("\"default\""));
    assert!(contents.contains("\"auth_type\": \"bearer\""));
    assert!(contents.contains("\"secret_source\""));
    assert!(contents.contains("\"kind\": \"literal\""));
    assert!(contents.contains("\"value\": \"sk-test-1234\""));
}

#[test]
fn test_load_legacy_api_key_and_migrate_on_save() {
    let temp_dir = setup_test_env();
    let auth_dir = temp_dir.temp_dir.path().join(".uxc");
    fs::create_dir_all(&auth_dir).expect("auth dir should be created");
    let profiles_path = auth_dir.join("credentials.json");

    let legacy = r#"
{
  "version": 1,
  "credentials": {
    "legacy": {
      "auth_type": "bearer",
      "api_key": "legacy-secret"
    }
  }
}
"#;
    fs::write(&profiles_path, legacy).expect("legacy credentials should be written");

    let loaded = Profiles::load_profiles().expect("legacy credentials should load");
    let profile = loaded
        .get_profile("legacy")
        .expect("legacy profile should exist");
    assert_eq!(profile.api_key, "legacy-secret");
    assert!(profile.secret_source.is_some());

    loaded
        .save_profiles()
        .expect("saving migrated profiles should work");
    let contents = fs::read_to_string(&profiles_path).expect("should read migrated file");
    assert!(contents.contains("\"secret_source\""));
    assert!(contents.contains("\"kind\": \"literal\""));
    assert!(contents.contains("\"value\": \"legacy-secret\""));
    assert!(!contents.contains("\"api_key\": \"legacy-secret\""));
}

#[cfg(unix)]
#[test]
fn test_credentials_file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    profiles
        .set_profile(
            "secure".to_string(),
            Profile::new("secret-token".to_string(), AuthType::Bearer),
        )
        .expect("set profile should succeed");
    profiles.save_profiles().expect("save should succeed");

    let profiles_path = temp_dir.temp_dir.path().join(".uxc/credentials.json");
    let mode = fs::metadata(&profiles_path)
        .expect("metadata should be readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "credentials file should be mode 0600");
}

#[test]
fn test_mask_api_key() {
    let profile = Profile::new("sk-1234567890abcdefgh".to_string(), AuthType::Bearer);
    assert_eq!(profile.mask_api_key(), "sk-12345...efgh");

    let short_profile = Profile::new("short".to_string(), AuthType::Bearer);
    assert_eq!(short_profile.mask_api_key(), "*****");
}

#[test]
fn test_update_existing_profile() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();

    // Create initial profile
    let profile1 = Profile::new("old-key".to_string(), AuthType::Bearer);
    profiles
        .set_profile("test".to_string(), profile1)
        .expect("Failed to set profile");
    profiles.save_profiles().expect("Failed to save profiles");

    // Update the profile
    let profile2 = Profile::new("new-key".to_string(), AuthType::ApiKey)
        .with_description("Updated profile".to_string());
    profiles
        .set_profile("test".to_string(), profile2)
        .expect("Failed to set profile");
    profiles.save_profiles().expect("Failed to save profiles");

    // Reload and verify update
    let loaded = Profiles::load_profiles().expect("Failed to load profiles");
    assert_eq!(loaded.count(), 1); // Still only one profile

    let test_profile = loaded.get_profile("test").expect("Failed to get profile");
    assert_eq!(test_profile.api_key, "new-key");
    assert_eq!(test_profile.auth_type, AuthType::ApiKey);
    assert_eq!(
        test_profile.description,
        Some("Updated profile".to_string())
    );
}

#[test]
fn test_parsing_auth_type_from_string() {
    assert_eq!("bearer".parse::<AuthType>().unwrap(), AuthType::Bearer);
    assert_eq!("BEARER".parse::<AuthType>().unwrap(), AuthType::Bearer);
    assert_eq!("api_key".parse::<AuthType>().unwrap(), AuthType::ApiKey);
    assert_eq!("basic".parse::<AuthType>().unwrap(), AuthType::Basic);
    assert!("invalid".parse::<AuthType>().is_err());
}

#[test]
fn test_auth_type_display() {
    assert_eq!(AuthType::Bearer.to_string(), "bearer");
    assert_eq!(AuthType::ApiKey.to_string(), "api_key");
    assert_eq!(AuthType::Basic.to_string(), "basic");
}

#[cfg(unix)]
#[test]
fn test_resolve_secret_from_op_provider() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = setup_test_env();
    let bin_dir = temp_dir.temp_dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir should be created");
    let op_path = bin_dir.join("op");
    let script = r#"#!/bin/sh
if [ "$1" = "read" ] && [ "$2" = "op://Vault/Item/field" ]; then
  printf "token-from-op"
  exit 0
fi
echo "not found" >&2
exit 1
"#;
    fs::write(&op_path, script).expect("op script should be written");
    let mut perms = fs::metadata(&op_path)
        .expect("op script metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&op_path, perms).expect("op script should be executable");

    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), old_path));

    let mut profiles = Profiles::new();
    let profile = Profile::new(String::new(), AuthType::Bearer)
        .with_secret_op("op://Vault/Item/field".to_string());
    profiles
        .set_profile("op-cred".to_string(), profile)
        .expect("set profile should succeed");
    profiles
        .save_profiles()
        .expect("save profiles should succeed");

    let resolved = uxc::auth::resolve_auth_for_endpoint(
        "https://api.example.com",
        Some("op-cred".to_string()),
    )
    .expect("op secret should resolve")
    .expect("profile should be present");
    assert_eq!(resolved.api_key, "token-from-op");
}

#[cfg(unix)]
#[test]
fn test_resolve_secret_from_op_provider_missing_binary() {
    let _temp_dir = setup_test_env();

    std::env::set_var("PATH", "/definitely-missing-op-bin");

    let mut profiles = Profiles::new();
    let profile = Profile::new(String::new(), AuthType::Bearer)
        .with_secret_op("op://Vault/Item/field".to_string());
    profiles
        .set_profile("op-cred".to_string(), profile)
        .expect("set profile should succeed");
    profiles
        .save_profiles()
        .expect("save profiles should succeed");

    let err = uxc::auth::resolve_auth_for_endpoint(
        "https://api.example.com",
        Some("op-cred".to_string()),
    )
    .expect_err("resolution should fail without op binary");
    assert!(err.to_string().contains("'op' CLI was not found"));
}

#[test]
fn test_profile_name_validation_valid_names() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("sk-test-key".to_string(), AuthType::Bearer);

    // Valid names: letters, numbers, underscores, hyphens
    assert!(profiles
        .set_profile("default".to_string(), profile.clone())
        .is_ok());
    assert!(profiles
        .set_profile("production".to_string(), profile.clone())
        .is_ok());
    assert!(profiles
        .set_profile("test_profile".to_string(), profile.clone())
        .is_ok());
    assert!(profiles
        .set_profile("test-profile".to_string(), profile.clone())
        .is_ok());
    assert!(profiles
        .set_profile("test123".to_string(), profile.clone())
        .is_ok());
    assert!(profiles
        .set_profile("my_prod_2024".to_string(), profile)
        .is_ok());
}

#[test]
fn test_profile_name_validation_invalid_characters() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("sk-test-key".to_string(), AuthType::Bearer);

    // Invalid characters: spaces, dots, special characters
    let result = profiles.set_profile("test profile".to_string(), profile.clone());
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("invalid characters"));

    let result = profiles.set_profile("test.profile".to_string(), profile.clone());
    assert!(result.is_err());

    let result = profiles.set_profile("test/profile".to_string(), profile.clone());
    assert!(result.is_err());

    let result = profiles.set_profile("test+profile".to_string(), profile.clone());
    assert!(result.is_err());

    let result = profiles.set_profile("test@profile".to_string(), profile.clone());
    assert!(result.is_err());
}

#[test]
fn test_profile_name_validation_empty_name() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("sk-test-key".to_string(), AuthType::Bearer);

    // Empty name should be rejected
    let result = profiles.set_profile("".to_string(), profile);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cannot be empty"));
}

#[test]
fn test_profile_name_validation_starts_with_digit() {
    let _temp_dir = setup_test_env();

    let mut profiles = Profiles::new();
    let profile = Profile::new("sk-test-key".to_string(), AuthType::Bearer);

    // Name starting with digit should be rejected
    let result = profiles.set_profile("123profile".to_string(), profile);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("cannot start with a digit"));
}
