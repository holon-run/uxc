//! Cache storage implementation

use super::config::CacheConfig;
use super::stats::{CacheStats, ProtocolStats};
use super::{Cache, CacheResult};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Cache entry containing the schema and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
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
    pub fn new(schema: Value, ttl: u64, protocol: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
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
        if !self.storage.config.enabled {
            debug!("Cache is disabled, bypassing");
            return Ok(CacheResult::Bypassed);
        }

        let key = self.storage.generate_cache_key(url);

        match self.storage.load_entry(&key) {
            Ok(Some(entry)) => {
                if entry.is_expired() {
                    debug!("Cache entry expired: {}", key);
                    self.storage.delete_entry(&key)?;
                    self.storage.record_miss();
                    Ok(CacheResult::Miss)
                } else {
                    debug!("Cache hit: {}", key);
                    self.storage.record_hit(&entry.protocol);
                    Ok(CacheResult::Hit(entry.schema))
                }
            }
            Ok(None) => {
                debug!("Cache miss: entry not found");
                self.storage.record_miss();
                Ok(CacheResult::Miss)
            }
            Err(e) => {
                warn!("Failed to load cache entry: {}", e);
                self.storage.record_miss();
                Ok(CacheResult::Miss)
            }
        }
    }

    fn put(&self, url: &str, schema: &Value) -> Result<()> {
        if !self.storage.config.enabled {
            return Ok(());
        }

        let key = self.storage.generate_cache_key(url);
        let protocol = self.detect_protocol(url);
        let entry = CacheEntry::new(schema.clone(), self.storage.config.ttl, protocol);

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
        let entry = CacheEntry::new(schema.clone(), 3600, "openapi".to_string());

        assert_eq!(entry.schema, schema);
        assert_eq!(entry.protocol, "openapi");
        assert!(!entry.is_expired());
    }

    #[test]
    fn test_cache_entry_expired() {
        let schema = serde_json::json!({"test": "data"});
        let entry = CacheEntry::new(schema, 0, "openapi".to_string());

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
}
