//! Cache storage implementation

use super::config::CacheConfig;
use super::stats::{CacheStats, ProtocolStats};
use super::{Cache, CacheHit, CacheLookup, CacheReadPolicy, CacheResult};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Component;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Cache list entry for CLI/API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheListEntry {
    pub key: String,
    pub url: String,
    pub protocol: String,
    pub fetched_at: u64,
    pub expires_at: u64,
    pub stale: bool,
    pub size_bytes: u64,
}

/// Cache entry containing the schema and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Original endpoint URL
    #[serde(default)]
    pub url: String,

    /// The cached schema data
    pub schema: Value,

    /// When the schema was cached (Unix timestamp)
    pub fetched_at: u64,

    /// When the cache entry expires (Unix timestamp)
    pub expires_at: u64,

    /// ETag for validation (optional)
    pub etag: Option<String>,

    /// Protocol type (openapi, grpc, graphql, mcp)
    pub protocol: String,
}

impl CacheEntry {
    /// Create a new cache entry
    pub fn new(url: String, schema: Value, ttl: u64, protocol: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            url,
            schema,
            fetched_at: now,
            expires_at: now + ttl,
            etag: None,
            protocol,
        }
    }

    /// Check if the cache entry is expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now >= self.expires_at
    }

    /// Get the size of this cache entry in bytes
    pub fn size(&self) -> u64 {
        // Approximate size based on JSON serialization
        serde_json::to_string(self).map_or(0, |s| s.len() as u64)
    }
}

/// Filesystem-based cache storage
pub struct CacheStorage {
    config: CacheConfig,
    cache_dir: PathBuf,

    // In-memory stats tracking
    stats: Arc<RwLock<CacheStats>>,
}

impl CacheStorage {
    /// Create a new cache storage instance
    pub fn new(config: CacheConfig) -> Result<Self> {
        let cache_dir = config.location.clone();

        // Ensure cache directory exists
        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)
                .with_context(|| format!("Failed to create cache directory: {:?}", cache_dir))?;
            info!("Created cache directory: {:?}", cache_dir);
        }

        Ok(Self {
            config,
            cache_dir,
            stats: Arc::new(RwLock::new(CacheStats::new())),
        })
    }

    /// Generate a cache key from a URL
    ///
    /// Uses a hash of the URL to generate a unique filename.
    fn generate_cache_key(&self, url: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        format!("{:x}.json", hasher.finish())
    }

    /// Get the full path for a cache key
    fn cache_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(key)
    }

    fn cache_key_to_id(&self, key: &str) -> String {
        key.strip_suffix(".json").unwrap_or(key).to_string()
    }

    fn normalize_cache_key_input(&self, key: &str) -> Result<String> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            anyhow::bail!("cache key cannot be empty");
        }
        if trimmed.contains("..") {
            anyhow::bail!("cache key must not contain '..'");
        }
        if trimmed.contains('/') || trimmed.contains('\\') {
            anyhow::bail!("cache key must not contain path separators");
        }
        if trimmed.contains(':') {
            anyhow::bail!("cache key must not contain ':'");
        }
        if !std::path::Path::new(trimmed)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            anyhow::bail!("cache key must be a single filename");
        }
        let normalized = if trimmed.ends_with(".json") {
            trimmed.to_string()
        } else {
            format!("{trimmed}.json")
        };
        Ok(normalized)
    }

    /// Load a cache entry from disk
    fn load_entry(&self, key: &str) -> Result<Option<CacheEntry>> {
        let path = self.cache_path(key);

        if !path.exists() {
            return Ok(None);
        }

        let file =
            File::open(&path).with_context(|| format!("Failed to open cache file: {:?}", path))?;
        let reader = BufReader::new(file);

        let entry: CacheEntry = serde_json::from_reader(reader)
            .with_context(|| format!("Failed to parse cache file: {:?}", path))?;

        Ok(Some(entry))
    }

    /// Save a cache entry to disk
    fn save_entry(&self, key: &str, entry: &CacheEntry) -> Result<()> {
        let path = self.cache_path(key);

        let file = File::create(&path)
            .with_context(|| format!("Failed to create cache file: {:?}", path))?;
        let writer = BufWriter::new(file);

        serde_json::to_writer_pretty(writer, entry)
            .with_context(|| format!("Failed to write cache file: {:?}", path))?;

        debug!("Saved cache entry: {}", key);
        Ok(())
    }

    /// Delete a cache entry from disk
    fn delete_entry(&self, key: &str) -> Result<()> {
        let path = self.cache_path(key);

        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove cache file: {:?}", path))?;
            debug!("Deleted cache entry: {}", key);
        }

        Ok(())
    }

    /// Scan cache directory and collect statistics
    fn scan_cache(&self) -> Result<CacheStats> {
        let mut stats = CacheStats::new();
        let mut by_protocol: HashMap<String, ProtocolStats> = HashMap::new();

        let entries = fs::read_dir(&self.cache_dir)
            .with_context(|| format!("Failed to read cache directory: {:?}", self.cache_dir))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            // Try to load the entry to get metadata
            if let Ok(Some(cache_entry)) =
                self.load_entry(path.file_name().and_then(|s| s.to_str()).unwrap_or(""))
            {
                // Check if expired
                if cache_entry.is_expired() {
                    // Delete expired entries
                    let _ =
                        self.delete_entry(path.file_name().and_then(|s| s.to_str()).unwrap_or(""));
                    continue;
                }

                let size = cache_entry.size();
                stats.total_entries += 1;
                stats.total_size += size;

                by_protocol
                    .entry(cache_entry.protocol.clone())
                    .or_default()
                    .entries += 1;

                by_protocol.get_mut(&cache_entry.protocol).unwrap().size += size;
            }
        }

        stats.by_protocol = by_protocol;
        Ok(stats)
    }

    /// Record a cache hit
    fn record_hit(&self, _protocol: &str) {
        if let Ok(mut stats) = self.stats.write() {
            stats.hits += 1;
            stats.calculate_hit_rate();
        }
    }

    /// Record a cache miss
    fn record_miss(&self) {
        if let Ok(mut stats) = self.stats.write() {
            stats.misses += 1;
            stats.calculate_hit_rate();
        }
    }
}

/// Public schema cache that implements the Cache trait
pub struct SchemaCache {
    storage: CacheStorage,
}

impl SchemaCache {
    /// Create a new schema cache
    pub fn new(config: CacheConfig) -> Result<Self> {
        let storage = CacheStorage::new(config)?;
        Ok(Self { storage })
    }

    /// Create with default configuration
    #[allow(dead_code)]
    pub fn with_default_config() -> Result<Self> {
        Self::new(CacheConfig::default())
    }

    /// Detect protocol from URL
    fn detect_protocol(&self, url: &str) -> String {
        let lower = url.to_lowercase();

        if lower.contains("grpc") || lower.starts_with("grpc://") {
            "grpc".to_string()
        } else if lower.contains("graphql") || lower.ends_with("/graphql") {
            "graphql".to_string()
        } else if lower.contains("mcp") || lower.contains("model-context-protocol") {
            "mcp".to_string()
        } else {
            // Default to openapi for HTTP/HTTPS URLs
            "openapi".to_string()
        }
    }
}

impl Cache for SchemaCache {
    fn get(&self, url: &str) -> Result<CacheResult> {
        match self.get_with_policy(url, CacheReadPolicy::NormalTtl)? {
            CacheLookup::Hit(hit) => Ok(CacheResult::Hit(hit.schema)),
            CacheLookup::Miss => Ok(CacheResult::Miss),
            CacheLookup::Bypassed => Ok(CacheResult::Bypassed),
        }
    }

    fn get_with_policy(&self, url: &str, policy: CacheReadPolicy) -> Result<CacheLookup> {
        if !self.storage.config.enabled {
            debug!("Cache is disabled, bypassing");
            return Ok(CacheLookup::Bypassed);
        }

        let key = self.storage.generate_cache_key(url);

        match self.storage.load_entry(&key) {
            Ok(Some(entry)) => {
                let stale = entry.is_expired();
                if stale && matches!(policy, CacheReadPolicy::NormalTtl) {
                    debug!("Cache entry expired: {}", key);
                    self.storage.record_miss();
                    Ok(CacheLookup::Miss)
                } else {
                    debug!("Cache hit: {} (stale={})", key, stale);
                    self.storage.record_hit(&entry.protocol);
                    Ok(CacheLookup::Hit(CacheHit {
                        schema: entry.schema,
                        fetched_at: entry.fetched_at,
                        stale,
                    }))
                }
            }
            Ok(None) => {
                debug!("Cache miss: entry not found");
                self.storage.record_miss();
                Ok(CacheLookup::Miss)
            }
            Err(e) => {
                warn!("Failed to load cache entry: {}", e);
                self.storage.record_miss();
                Ok(CacheLookup::Miss)
            }
        }
    }

    fn put(&self, url: &str, schema: &Value) -> Result<()> {
        if !self.storage.config.enabled {
            return Ok(());
        }

        let key = self.storage.generate_cache_key(url);
        let protocol = self.detect_protocol(url);
        let entry = CacheEntry::new(
            url.to_string(),
            schema.clone(),
            self.storage.config.ttl,
            protocol,
        );

        self.storage.save_entry(&key, &entry)?;
        info!("Cached schema for: {}", url);

        Ok(())
    }

    fn invalidate(&self, url: &str) -> Result<()> {
        let key = self.storage.generate_cache_key(url);
        self.storage.delete_entry(&key)?;
        info!("Invalidated cache for: {}", url);
        Ok(())
    }

    fn invalidate_by_key(&self, key: &str) -> Result<()> {
        let normalized = self.storage.normalize_cache_key_input(key)?;
        self.storage.delete_entry(&normalized)?;
        info!(
            "Invalidated cache for key: {}",
            self.storage.cache_key_to_id(&normalized)
        );
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let entries = fs::read_dir(&self.storage.cache_dir)
            .with_context(|| "Failed to read cache directory")?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove cache file: {:?}", path))?;
            }
        }

        info!("Cleared all cache entries");

        // Reset stats
        if let Ok(mut stats) = self.storage.stats.write() {
            *stats = CacheStats::new();
        }

        Ok(())
    }

    fn list_entries(&self) -> Result<Vec<CacheListEntry>> {
        let mut results = Vec::new();
        let entries = fs::read_dir(&self.storage.cache_dir).with_context(|| {
            format!(
                "Failed to read cache directory: {:?}",
                self.storage.cache_dir
            )
        })?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let cache_key = self.storage.cache_key_to_id(file_name);

            if let Ok(Some(cache_entry)) = self.storage.load_entry(file_name) {
                let stale = cache_entry.is_expired();
                let size_bytes = cache_entry.size();
                results.push(CacheListEntry {
                    key: cache_key,
                    url: cache_entry.url,
                    protocol: cache_entry.protocol,
                    fetched_at: cache_entry.fetched_at,
                    expires_at: cache_entry.expires_at,
                    stale,
                    size_bytes,
                });
            }
        }

        results.sort_by(|a, b| b.fetched_at.cmp(&a.fetched_at));
        Ok(results)
    }

    fn stats(&self) -> Result<CacheStats> {
        let mut stats = self.storage.scan_cache()?;

        // Add in-memory hit/miss counters
        if let Ok(memory_stats) = self.storage.stats.try_read() {
            stats.hits = memory_stats.hits;
            stats.misses = memory_stats.misses;
            stats.calculate_hit_rate();
        }

        Ok(stats)
    }

    fn is_enabled(&self) -> bool {
        self.storage.config.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_cache() -> (SchemaCache, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            enabled: true,
            ttl: 3600,
            max_size: 0,
            location: temp_dir.path().to_path_buf(),
        };

        (SchemaCache::new(config).unwrap(), temp_dir)
    }

    #[test]
    fn test_cache_entry_new() {
        let schema = serde_json::json!({"test": "data"});
        let entry = CacheEntry::new(
            "https://api.example.com/openapi.json".to_string(),
            schema.clone(),
            3600,
            "openapi".to_string(),
        );

        assert_eq!(entry.url, "https://api.example.com/openapi.json");
        assert_eq!(entry.schema, schema);
        assert_eq!(entry.protocol, "openapi");
        assert!(!entry.is_expired());
    }

    #[test]
    fn test_cache_entry_expired() {
        let schema = serde_json::json!({"test": "data"});
        let entry = CacheEntry::new(
            "https://api.example.com/openapi.json".to_string(),
            schema,
            0,
            "openapi".to_string(),
        );

        // Should be expired immediately with 0 TTL
        assert!(entry.is_expired());
    }

    #[test]
    fn test_cache_put_get() {
        let (cache, _temp) = create_test_cache();

        let url = "https://api.example.com/openapi.json";
        let schema = serde_json::json!({"openapi": "3.0", "info": {"title": "Test API"}});

        // Put schema in cache
        cache.put(url, &schema).unwrap();

        // Get schema from cache
        match cache.get(url).unwrap() {
            CacheResult::Hit(cached_schema) => {
                assert_eq!(cached_schema, schema);
            }
            _ => panic!("Expected cache hit"),
        }
    }

    #[test]
    fn test_cache_miss() {
        let (cache, _temp) = create_test_cache();

        let url = "https://api.example.com/openapi.json";

        // Should miss since we haven't cached anything
        match cache.get(url).unwrap() {
            CacheResult::Miss => {
                // Expected
            }
            _ => panic!("Expected cache miss"),
        }
    }

    #[test]
    fn test_cache_invalidate() {
        let (cache, _temp) = create_test_cache();

        let url = "https://api.example.com/openapi.json";
        let schema = serde_json::json!({"openapi": "3.0"});

        // Put schema in cache
        cache.put(url, &schema).unwrap();

        // Invalidate
        cache.invalidate(url).unwrap();

        // Should now miss
        match cache.get(url).unwrap() {
            CacheResult::Miss => {
                // Expected
            }
            _ => panic!("Expected cache miss after invalidation"),
        }
    }

    #[test]
    fn test_cache_clear() {
        let (cache, _temp) = create_test_cache();

        let url1 = "https://api1.example.com/openapi.json";
        let url2 = "https://api2.example.com/openapi.json";
        let schema = serde_json::json!({"openapi": "3.0"});

        // Put multiple schemas in cache
        cache.put(url1, &schema).unwrap();
        cache.put(url2, &schema).unwrap();

        // Clear all
        cache.clear().unwrap();

        // Both should miss now
        match cache.get(url1).unwrap() {
            CacheResult::Miss => {}
            _ => panic!("Expected cache miss after clear"),
        }

        match cache.get(url2).unwrap() {
            CacheResult::Miss => {}
            _ => panic!("Expected cache miss after clear"),
        }
    }

    #[test]
    fn test_cache_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            enabled: false,
            ttl: 3600,
            max_size: 0,
            location: temp_dir.path().to_path_buf(),
        };

        let cache = SchemaCache::new(config).unwrap();

        let url = "https://api.example.com/openapi.json";

        // Should bypass when disabled
        match cache.get(url).unwrap() {
            CacheResult::Bypassed => {
                // Expected
            }
            _ => panic!("Expected cache bypassed"),
        }

        // Put should be a no-op
        let schema = serde_json::json!({"openapi": "3.0"});
        cache.put(url, &schema).unwrap();

        // Still should bypass
        match cache.get(url).unwrap() {
            CacheResult::Bypassed => {
                // Expected
            }
            _ => panic!("Expected cache bypassed"),
        }
    }

    #[test]
    fn test_protocol_detection() {
        let (cache, _temp) = create_test_cache();

        assert_eq!(
            cache.detect_protocol("https://api.example.com/openapi.json"),
            "openapi"
        );
        assert_eq!(cache.detect_protocol("grpc://api.example.com"), "grpc");
        assert_eq!(
            cache.detect_protocol("https://api.example.com/graphql"),
            "graphql"
        );
        assert_eq!(cache.detect_protocol("https://api.example.com/mcp"), "mcp");
    }

    #[test]
    fn test_cache_invalidate_by_key() {
        let (cache, temp) = create_test_cache();
        let url = "https://api.example.com/openapi.json";
        let schema = serde_json::json!({"openapi": "3.0"});

        cache.put(url, &schema).unwrap();

        let file_name = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .find_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|s| s.to_str()) == Some("json"))
                    .then(|| {
                        path.file_name()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string())
                    })
                    .flatten()
            })
            .expect("cache file should exist");

        let key = file_name.trim_end_matches(".json").to_string();
        cache.invalidate_by_key(&key).unwrap();

        match cache.get(url).unwrap() {
            CacheResult::Miss => {}
            _ => panic!("Expected cache miss after clear by key"),
        }
    }

    #[test]
    fn test_list_entries_includes_url_and_key() {
        let (cache, _temp) = create_test_cache();
        let url = "https://api.example.com/openapi.json";
        let schema = serde_json::json!({"openapi": "3.0"});

        cache.put(url, &schema).unwrap();

        let entries = cache.list_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, url);
        assert_eq!(entries[0].protocol, "openapi");
        assert!(!entries[0].key.is_empty());
        assert!(entries[0].size_bytes > 0);
    }

    #[test]
    fn test_list_entries_legacy_entry_defaults_empty_url() {
        let (cache, temp) = create_test_cache();

        let legacy = serde_json::json!({
            "schema": {"openapi": "3.0"},
            "fetched_at": 100,
            "expires_at": 9_999_999_999u64,
            "etag": null,
            "protocol": "openapi"
        });
        let file = temp.path().join("legacy-entry.json");
        fs::write(file, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let entries = cache.list_entries().unwrap();
        let legacy_entry = entries
            .iter()
            .find(|e| e.key == "legacy-entry")
            .expect("legacy entry should be listed");
        assert_eq!(legacy_entry.url, "");
    }

    #[test]
    fn test_invalidate_by_key_rejects_parent_dir_sequence() {
        let (cache, _temp) = create_test_cache();
        let err = cache.invalidate_by_key("../foo").unwrap_err();
        assert!(err.to_string().contains("must not contain"));
    }

    #[test]
    fn test_invalidate_by_key_rejects_colon() {
        let (cache, _temp) = create_test_cache();
        let err = cache.invalidate_by_key("C:temp").unwrap_err();
        assert!(err.to_string().contains("must not contain ':'"));
    }
}
