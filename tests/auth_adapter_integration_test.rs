//! Integration tests for authentication with protocol adapters
//!
//! These tests verify that authentication profiles are correctly applied
//! to HTTP requests for different protocol adapters.

use std::env;
use std::ffi::OsString;
use tempfile::TempDir;
use uxc::auth::{AuthHeader, AuthType, Profile, Profiles};

struct TestEnv {
    _temp_dir: TempDir,
    prev_home: Option<OsString>,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(prev) => env::set_var("HOME", prev),
            None => env::remove_var("HOME"),
        }
    }
}

/// Helper function to create a test environment with a temporary directory
fn setup_test_env() -> TestEnv {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Capture the previous HOME value and set HOME to the temp directory for testing
    let prev_home = env::var_os("HOME");
    env::set_var("HOME", temp_dir.path());

    TestEnv {
        _temp_dir: temp_dir,
        prev_home,
    }
}

#[test]
fn test_profile_storage() {
    // Test profile storage and retrieval
    let _test_env = setup_test_env();

    // Create test profiles
    let mut profiles = Profiles::new();
    profiles
        .set_profile(
            "default".to_string(),
            Profile::new("key-default".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set default profile");
    profiles
        .set_profile(
            "production".to_string(),
            Profile::new("key-prod".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set production profile");
    profiles
        .set_profile(
            "staging".to_string(),
            Profile::new("key-staging".to_string(), AuthType::Bearer),
        )
        .expect("Failed to set staging profile");

    profiles.save_profiles().expect("Failed to save profiles");

    // Note: This test only verifies profile storage works correctly.
    // Actual CLI flag testing would require running the binary with different arguments.
    assert!(profiles.has_profile("default"));
    assert!(profiles.has_profile("production"));
    assert!(profiles.has_profile("staging"));

    // Verify profile content
    let default = profiles.get_profile("default").unwrap();
    assert_eq!(default.api_key, "key-default");
    assert_eq!(default.auth_type, AuthType::Bearer);
}

#[test]
fn test_auth_apply_to_request_bearer() {
    use reqwest::Client;

    let profile = Profile::new("test-token-12345".to_string(), AuthType::Bearer);
    let client = Client::new();

    let req = client.get("http://example.com");
    let req = uxc::auth::apply_auth_to_request(req, &profile.auth_type, &profile.api_key);

    // Build the request and verify headers
    let built_req = req.build().expect("Failed to build request");
    assert_eq!(
        built_req.headers().get("authorization"),
        Some(&"Bearer test-token-12345".parse().unwrap())
    );
}

#[test]
fn test_auth_apply_to_request_api_key() {
    use reqwest::Client;

    let profile = Profile::new("test-api-key".to_string(), AuthType::ApiKey);
    let client = Client::new();

    let req = client.post("http://example.com");
    let req = uxc::auth::apply_auth_to_request(req, &profile.auth_type, &profile.api_key);

    // Build the request and verify headers
    let built_req = req.build().expect("Failed to build request");
    assert_eq!(
        built_req.headers().get("x-api-key"),
        Some(&"test-api-key".parse().unwrap())
    );
}

#[test]
fn test_auth_apply_to_request_basic() {
    use reqwest::Client;

    let profile = Profile::new("user:password".to_string(), AuthType::Basic);
    let client = Client::new();

    let req = client.get("http://example.com");
    let req = uxc::auth::apply_auth_to_request(req, &profile.auth_type, &profile.api_key);

    // Build the request and verify headers
    let built_req = req.build().expect("Failed to build request");
    // "user:password" Base64-encoded is "dXNlcjpwYXNzd29yZA=="
    assert_eq!(
        built_req.headers().get("authorization"),
        Some(&"Basic dXNlcjpwYXNzd29yZA==".parse().unwrap())
    );
}

#[test]
fn test_auth_to_metadata_bearer() {
    let metadata = uxc::auth::auth_to_metadata(&AuthType::Bearer, "test-token")
        .expect("Failed to create metadata");

    let auth_value = metadata
        .get("authorization")
        .expect("Authorization header not found");
    assert_eq!(auth_value, "Bearer test-token");
}

#[test]
fn test_auth_to_metadata_api_key() {
    let metadata = uxc::auth::auth_to_metadata(&AuthType::ApiKey, "test-api-key")
        .expect("Failed to create metadata");

    let api_key_value = metadata
        .get("x-api-key")
        .expect("x-api-key header not found");
    assert_eq!(api_key_value, "test-api-key");
}

#[test]
fn test_auth_to_metadata_basic() {
    let metadata = uxc::auth::auth_to_metadata(&AuthType::Basic, "user:password")
        .expect("Failed to create metadata");

    // "user:password" Base64-encoded is "dXNlcjpwYXNzd29yZA=="
    let auth_value = metadata
        .get("authorization")
        .expect("Authorization header not found");
    assert_eq!(auth_value, "Basic dXNlcjpwYXNzd29yZA==");
}

#[test]
fn test_auth_to_metadata_invalid_token() {
    // Test with invalid metadata characters (e.g., newline)
    let result = uxc::auth::auth_to_metadata(&AuthType::Bearer, "test\n token");

    assert!(result.is_err());
}

#[test]
fn test_auth_apply_profile_to_request_api_key_custom_headers() {
    use reqwest::Client;

    let mut profile = Profile::new("test-secret".to_string(), AuthType::ApiKey);
    profile.auth_headers = Some(vec![
        AuthHeader::new("ok-access-key", "{{secret}}").unwrap(),
        AuthHeader::new("x-client", "uxc").unwrap(),
    ]);

    let client = Client::new();
    let req = client.post("http://example.com");
    let req = uxc::auth::apply_profile_auth_to_request(req, &profile)
        .expect("custom auth headers should render");

    let built_req = req.build().expect("Failed to build request");
    assert_eq!(
        built_req.headers().get("ok-access-key"),
        Some(&"test-secret".parse().unwrap())
    );
    assert_eq!(
        built_req.headers().get("x-client"),
        Some(&"uxc".parse().unwrap())
    );
    assert!(built_req.headers().get("x-api-key").is_none());
}

#[test]
fn test_auth_apply_profile_to_request_api_key_env_template() {
    use reqwest::Client;

    env::set_var("UXC_TEST_HEADER_ENV", "tenant-a");
    let mut profile = Profile::new("test-secret".to_string(), AuthType::ApiKey);
    profile.auth_headers = Some(vec![AuthHeader::new(
        "x-tenant",
        "{{env:UXC_TEST_HEADER_ENV}}",
    )
    .unwrap()]);

    let client = Client::new();
    let req = client.post("http://example.com");
    let req = uxc::auth::apply_profile_auth_to_request(req, &profile)
        .expect("env template should render");

    let built_req = req.build().expect("Failed to build request");
    assert_eq!(
        built_req.headers().get("x-tenant"),
        Some(&"tenant-a".parse().unwrap())
    );
    env::remove_var("UXC_TEST_HEADER_ENV");
}
