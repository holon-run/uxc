//! Cache configuration tests

use std::fs;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

use uxc::cache::CacheConfig;

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

fn setup_test_env() -> TestEnv {
    let guard = home_env_lock()
        .lock()
        .expect("Failed to lock HOME env guard");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let previous_home = std::env::var_os("HOME");

    std::env::set_var("HOME", temp_dir.path());

    TestEnv {
        temp_dir,
        _home_guard: guard,
        previous_home,
    }
}

#[test]
fn test_load_from_file_not_exists() {
    let _env = setup_test_env();

    let result = CacheConfig::load_from_file();
    assert!(
        result.is_ok(),
        "Should return default config when file doesn't exist"
    );

    let config = result.unwrap();
    assert!(config.enabled, "Default config should be enabled");
}

#[test]
fn test_load_from_file_valid_config() {
    let env = setup_test_env();

    let config_dir = env.temp_dir.path().join(".uxc");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("config.toml");

    fs::write(
        &config_file,
        r#"
[cache]
enabled = true
ttl = 3600
max_size = 1048576
"#,
    )
    .expect("Failed to write config");

    let result = CacheConfig::load_from_file();
    assert!(result.is_ok(), "Should successfully load valid config");

    let config = result.unwrap();
    assert!(config.enabled, "Config should be enabled");
    assert_eq!(config.ttl, 3600, "TTL should be 3600");
    assert_eq!(config.max_size, 1048576, "Max size should be 1048576");
}

#[test]
fn test_load_from_file_invalid_values_fallback() {
    let env = setup_test_env();

    let config_dir = env.temp_dir.path().join(".uxc");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("config.toml");

    fs::write(
        &config_file,
        r#"
[cache]
enabled = "not_a_bool"
ttl = "not_a_number"
max_size = -100
"#,
    )
    .expect("Failed to write config");

    let result = CacheConfig::load_from_file();
    assert!(result.is_ok(), "Should succeed with fallback defaults");

    let config = result.unwrap();
    let default_config = CacheConfig::default();
    assert_eq!(
        config.enabled, default_config.enabled,
        "Invalid 'enabled' should fall back to default"
    );
    assert_eq!(
        config.ttl, default_config.ttl,
        "Invalid 'ttl' should fall back to default"
    );
    assert_eq!(
        config.max_size, default_config.max_size,
        "Invalid 'max_size' should fall back to default"
    );
}

#[test]
fn test_load_from_file_with_comments_and_empty_lines() {
    let env = setup_test_env();

    let config_dir = env.temp_dir.path().join(".uxc");
    fs::create_dir_all(&config_dir).expect("Failed to create config dir");
    let config_file = config_dir.join("config.toml");

    fs::write(
        &config_file,
        r#"
# UXC Configuration File
# This file controls cache behavior

[cache]
enabled = true
ttl = 7200

# Max cache size in bytes
max_size = 2097152

# Additional settings can go here
"#,
    )
    .expect("Failed to write config");

    let result = CacheConfig::load_from_file();
    assert!(
        result.is_ok(),
        "Should successfully load config with comments"
    );

    let config = result.unwrap();
    assert!(config.enabled, "Config should be enabled");
    assert_eq!(config.ttl, 7200, "TTL should be 7200");
    assert_eq!(config.max_size, 2097152, "max size should be parsed");
}

#[test]
fn test_ensure_cache_dir_creates_directory() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let config = CacheConfig::new(true, 3600, 1024, temp_dir.path().to_path_buf());
    let result = config.ensure_cache_dir();

    assert!(result.is_ok(), "Should successfully create cache directory");
    assert!(
        temp_dir.path().exists(),
        "Cache directory should be created"
    );
}

#[test]
fn test_ensure_cache_dir_handles_existing_directory() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let cache_dir = temp_dir.path().join("cache");
    fs::create_dir(&cache_dir).expect("Failed to create cache dir");

    let config = CacheConfig::new(true, 3600, 1024, temp_dir.path().to_path_buf());
    let result = config.ensure_cache_dir();

    assert!(
        result.is_ok(),
        "Should succeed when directory already exists"
    );
}
