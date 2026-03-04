//! Schema caching module for improved performance
//!
//! Provides filesystem-based caching for schemas across all protocols (OpenAPI, gRPC, GraphQL, MCP).
//! Cache is stored in ~/.uxc/cache/schemas/ with TTL-based expiration.

mod config;
mod stats;
mod storage;

pub use config::CacheConfig;
#[allow(unused_imports)]
pub use config::CacheOptions;
pub use stats::CacheStats;
pub use storage::CacheListEntry;
pub use storage::SchemaCache;
#[allow(unused_imports)]
pub use storage::{CacheEntry, CacheStorage};

use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

/// Default cache TTL in seconds (24 hours)
pub const DEFAULT_CACHE_TTL: u64 = 86400;

/// Default cache directory relative to home directory
pub const DEFAULT_CACHE_DIR: &str = ".uxc/cache/schemas";

/// Cache result indicating whether the value was retrieved from cache
#[derive(Debug, Clone)]
pub enum CacheResult {
    /// Value was retrieved from cache
    Hit(Value),
    /// Value was not in cache or was expired
    Miss,
    /// Cache was bypassed (e.g., --no-cache flag)
    Bypassed,
}

/// Cache read policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheReadPolicy {
    /// Enforce TTL. Expired entries are treated as miss.
    NormalTtl,
    /// Allow returning stale entries.
    AllowStale,
}

/// Cache lookup result with metadata.
#[derive(Debug, Clone)]
pub enum CacheLookup {
    /// Value was retrieved from cache.
    Hit(CacheHit),
    /// Value was not in cache.
    Miss,
    /// Cache read was bypassed.
    Bypassed,
}

/// Cached value with timing metadata.
#[derive(Debug, Clone)]
pub struct CacheHit {
    pub schema: Value,
    pub fetched_at: u64,
    pub stale: bool,
}

impl CacheResult {
    #[allow(dead_code)]
    pub fn is_hit(&self) -> bool {
        matches!(self, CacheResult::Hit(_))
    }

    #[allow(dead_code)]
    pub fn is_miss(&self) -> bool {
        matches!(self, CacheResult::Miss)
    }

    #[allow(dead_code)]
    pub fn is_bypassed(&self) -> bool {
        matches!(self, CacheResult::Bypassed)
    }
}

/// Main cache interface for adapters
///
/// This trait provides a simple interface for protocol adapters to interact
/// with the cache system.
pub trait Cache: Send + Sync {
    /// Get a schema from cache
    ///
    /// Returns `CacheResult::Hit` if the schema is found and valid,
    /// `CacheResult::Miss` if not found or expired, or `CacheResult::Bypassed`
    /// if caching is disabled.
    fn get(&self, url: &str) -> Result<CacheResult>;

    /// Get a schema from cache with explicit read policy and metadata.
    fn get_with_policy(&self, url: &str, policy: CacheReadPolicy) -> Result<CacheLookup>;

    /// Put a schema into cache
    ///
    /// Stores the schema with metadata including timestamp and TTL.
    /// If caching is disabled, this is a no-op.
    fn put(&self, url: &str, schema: &Value) -> Result<()>;

    /// Invalidate a specific cache entry
    fn invalidate(&self, url: &str) -> Result<()>;

    /// Invalidate a specific cache entry by cache key
    fn invalidate_by_key(&self, key: &str) -> Result<()>;

    /// Clear all cache entries
    fn clear(&self) -> Result<()>;

    /// List current cache entries
    fn list_entries(&self) -> Result<Vec<CacheListEntry>>;

    /// Get cache statistics
    fn stats(&self) -> Result<CacheStats>;

    /// Check if caching is enabled
    #[allow(dead_code)]
    fn is_enabled(&self) -> bool;
}

/// Create a new schema cache instance with the given configuration
pub fn create_cache(config: CacheConfig) -> Result<Arc<dyn Cache>> {
    Ok(Arc::new(SchemaCache::new(config)?))
}

/// Create a cache with default settings
#[allow(dead_code)]
pub fn create_default_cache() -> Result<Arc<dyn Cache>> {
    Ok(Arc::new(SchemaCache::with_default_config()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_result() {
        let hit = CacheResult::Hit(serde_json::json!( {"test": "data"} ));
        assert!(hit.is_hit());
        assert!(!hit.is_miss());
        assert!(!hit.is_bypassed());

        let miss = CacheResult::Miss;
        assert!(!miss.is_hit());
        assert!(miss.is_miss());
        assert!(!miss.is_bypassed());

        let bypassed = CacheResult::Bypassed;
        assert!(!bypassed.is_hit());
        assert!(!bypassed.is_miss());
        assert!(bypassed.is_bypassed());
    }
}
