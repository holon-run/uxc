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
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        match &self.previous_home {
            Some(prev) => std::env::set_var("HOME", prev),
            None => std::env::remove_var("HOME"),
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

    // Set HOME to the temp directory for testing
    std::env::set_var("HOME", temp_dir.path());

    TestEnv {
        temp_dir,
        _home_guard: guard,
        previous_home,
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
