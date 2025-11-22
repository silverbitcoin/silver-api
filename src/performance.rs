//! Performance optimization module for API endpoints
//!
//! Implements caching, batching, and optimization strategies

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Cache entry with TTL
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    value: T,
    created_at: Instant,
    ttl: Duration,
}

impl<T> CacheEntry<T> {
    /// Check if entry is expired
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }
}

/// Query result cache
pub struct QueryCache<K, V> {
    cache: Arc<RwLock<HashMap<K, CacheEntry<V>>>>,
    ttl: Duration,
    max_entries: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> QueryCache<K, V> {
    /// Create new query cache
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl,
            max_entries,
        }
    }

    /// Get value from cache
    pub fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.write().unwrap();
        
        if let Some(entry) = cache.get(key) {
            if !entry.is_expired() {
                debug!("Cache hit for key");
                return Some(entry.value.clone());
            } else {
                cache.remove(key);
            }
        }
        
        None
    }

    /// Set value in cache
    pub fn set(&self, key: K, value: V) {
        let mut cache = self.cache.write().unwrap();
        
        // Evict oldest entry if cache is full
        if cache.len() >= self.max_entries {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
                debug!("Evicted oldest cache entry");
            }
        }
        
        cache.insert(
            key,
            CacheEntry {
                value,
                created_at: Instant::now(),
                ttl: self.ttl,
            },
        );
    }

    /// Clear cache
    pub fn clear(&self) {
        self.cache.write().unwrap().clear();
        info!("Cache cleared");
    }

    /// Get cache size
    pub fn size(&self) -> usize {
        self.cache.read().unwrap().len()
    }

    /// Get cache stats
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.read().unwrap();
        let total_entries = cache.len();
        let expired_entries = cache
            .values()
            .filter(|entry| entry.is_expired())
            .count();

        CacheStats {
            total_entries,
            expired_entries,
            active_entries: total_entries - expired_entries,
            max_entries: self.max_entries,
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    /// Total entries in cache
    pub total_entries: usize,
    /// Expired entries
    pub expired_entries: usize,
    /// Active entries
    pub active_entries: usize,
    /// Maximum entries allowed
    pub max_entries: usize,
}

/// Batch request processor
pub struct BatchProcessor<T> {
    batch_size: usize,
    timeout: Duration,
}

impl<T> BatchProcessor<T> {
    /// Create new batch processor
    pub fn new(batch_size: usize, timeout: Duration) -> Self {
        Self { batch_size, timeout }
    }

    /// Process batch
    pub fn process_batch(&self, items: Vec<T>) -> Vec<Vec<T>> {
        let mut batches = Vec::new();
        let mut current_batch = Vec::new();

        for item in items {
            current_batch.push(item);
            
            if current_batch.len() >= self.batch_size {
                batches.push(current_batch);
                current_batch = Vec::new();
            }
        }

        if !current_batch.is_empty() {
            batches.push(current_batch);
        }

        debug!("Processed {} items into {} batches", 
            batches.iter().map(|b| b.len()).sum::<usize>(),
            batches.len()
        );

        batches
    }

    /// Get batch size
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Get timeout
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Connection pool for database connections
pub struct ConnectionPool {
    pool_size: usize,
    available: Arc<RwLock<usize>>,
    max_wait_time: Duration,
}

impl ConnectionPool {
    /// Create new connection pool
    pub fn new(pool_size: usize, max_wait_time: Duration) -> Self {
        Self {
            pool_size,
            available: Arc::new(RwLock::new(pool_size)),
            max_wait_time,
        }
    }

    /// Acquire connection
    pub fn acquire(&self) -> Result<Connection, String> {
        let mut available = self.available.write().unwrap();
        
        if *available > 0 {
            *available -= 1;
            debug!("Connection acquired, {} available", *available);
            Ok(Connection {
                pool_available: Arc::clone(&self.available),
            })
        } else {
            Err("No connections available".to_string())
        }
    }

    /// Get pool stats
    pub fn stats(&self) -> PoolStats {
        let available = *self.available.read().unwrap();
        
        PoolStats {
            pool_size: self.pool_size,
            available_connections: available,
            in_use_connections: self.pool_size - available,
            utilization: ((self.pool_size - available) as f64 / self.pool_size as f64) * 100.0,
        }
    }
}

/// Database connection
pub struct Connection {
    pool_available: Arc<RwLock<usize>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let mut available = self.pool_available.write().unwrap();
        *available += 1;
        debug!("Connection released, {} available", *available);
    }
}

/// Pool statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStats {
    /// Total pool size
    pub pool_size: usize,
    /// Available connections
    pub available_connections: usize,
    /// In-use connections
    pub in_use_connections: usize,
    /// Utilization percentage
    pub utilization: f64,
}

/// Query optimizer
pub struct QueryOptimizer {
    index_hints: HashMap<String, String>,
}

impl QueryOptimizer {
    /// Create new query optimizer
    pub fn new() -> Self {
        Self {
            index_hints: HashMap::new(),
        }
    }

    /// Add index hint
    pub fn add_index_hint(&mut self, query: String, index: String) {
        self.index_hints.insert(query, index);
        debug!("Added index hint");
    }

    /// Get index hint
    pub fn get_index_hint(&self, query: &str) -> Option<&str> {
        self.index_hints.get(query).map(|s| s.as_str())
    }

    /// Optimize query
    pub fn optimize_query(&self, query: &str) -> String {
        if let Some(index) = self.get_index_hint(query) {
            format!("{} USE INDEX ({})", query, index)
        } else {
            query.to_string()
        }
    }
}

impl Default for QueryOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory pool for object reuse
pub struct MemoryPool<T> {
    pool: Arc<RwLock<Vec<T>>>,
    max_size: usize,
}

impl<T> MemoryPool<T> {
    /// Create new memory pool
    pub fn new(max_size: usize) -> Self {
        Self {
            pool: Arc::new(RwLock::new(Vec::new())),
            max_size,
        }
    }

    /// Acquire object from pool
    pub fn acquire(&self) -> Option<T> {
        let mut pool = self.pool.write().unwrap();
        pool.pop()
    }

    /// Return object to pool
    pub fn return_object(&self, obj: T) {
        let mut pool = self.pool.write().unwrap();
        if pool.len() < self.max_size {
            pool.push(obj);
            debug!("Object returned to pool");
        }
    }

    /// Get pool size
    pub fn size(&self) -> usize {
        self.pool.read().unwrap().len()
    }
}

/// Performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Average query time (ms)
    pub avg_query_time_ms: f64,
    /// Max query time (ms)
    pub max_query_time_ms: f64,
    /// Min query time (ms)
    pub min_query_time_ms: f64,
    /// Cache hit rate (%)
    pub cache_hit_rate: f64,
    /// Throughput (queries/sec)
    pub throughput: f64,
    /// P95 latency (ms)
    pub p95_latency_ms: f64,
    /// P99 latency (ms)
    pub p99_latency_ms: f64,
}

/// Performance monitor
pub struct PerformanceMonitor {
    query_times: Arc<RwLock<Vec<f64>>>,
    cache_hits: Arc<RwLock<u64>>,
    cache_misses: Arc<RwLock<u64>>,
    start_time: Instant,
}

impl PerformanceMonitor {
    /// Create new performance monitor
    pub fn new() -> Self {
        Self {
            query_times: Arc::new(RwLock::new(Vec::new())),
            cache_hits: Arc::new(RwLock::new(0)),
            cache_misses: Arc::new(RwLock::new(0)),
            start_time: Instant::now(),
        }
    }

    /// Record query time
    pub fn record_query_time(&self, time_ms: f64) {
        let mut times = self.query_times.write().unwrap();
        times.push(time_ms);
        
        // Keep only last 1000 measurements
        if times.len() > 1000 {
            times.remove(0);
        }
    }

    /// Record cache hit
    pub fn record_cache_hit(&self) {
        *self.cache_hits.write().unwrap() += 1;
    }

    /// Record cache miss
    pub fn record_cache_miss(&self) {
        *self.cache_misses.write().unwrap() += 1;
    }

    /// Get performance metrics
    pub fn get_metrics(&self) -> PerformanceMetrics {
        let times = self.query_times.read().unwrap();
        let hits = *self.cache_hits.read().unwrap();
        let misses = *self.cache_misses.read().unwrap();

        let (avg, max, min) = if !times.is_empty() {
            let sum: f64 = times.iter().sum();
            let avg = sum / times.len() as f64;
            let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
            (avg, max, min)
        } else {
            (0.0, 0.0, 0.0)
        };

        let total_requests = hits + misses;
        let cache_hit_rate = if total_requests > 0 {
            (hits as f64 / total_requests as f64) * 100.0
        } else {
            0.0
        };

        let elapsed_secs = self.start_time.elapsed().as_secs_f64();
        let throughput = if elapsed_secs > 0.0 {
            total_requests as f64 / elapsed_secs
        } else {
            0.0
        };

        let (p95, p99) = if !times.is_empty() {
            let mut sorted = times.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            
            let p95_idx = (sorted.len() as f64 * 0.95) as usize;
            let p99_idx = (sorted.len() as f64 * 0.99) as usize;
            
            (
                sorted.get(p95_idx).cloned().unwrap_or(0.0),
                sorted.get(p99_idx).cloned().unwrap_or(0.0),
            )
        } else {
            (0.0, 0.0)
        };

        PerformanceMetrics {
            avg_query_time_ms: avg,
            max_query_time_ms: max,
            min_query_time_ms: min,
            cache_hit_rate,
            throughput,
            p95_latency_ms: p95,
            p99_latency_ms: p99,
        }
    }

    /// Reset metrics
    pub fn reset(&self) {
        self.query_times.write().unwrap().clear();
        *self.cache_hits.write().unwrap() = 0;
        *self.cache_misses.write().unwrap() = 0;
    }
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_cache_creation() {
        let cache: QueryCache<String, String> = QueryCache::new(Duration::from_secs(60), 100);
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn test_query_cache_set_get() {
        let cache: QueryCache<String, String> = QueryCache::new(Duration::from_secs(60), 100);
        
        cache.set("key1".to_string(), "value1".to_string());
        assert_eq!(cache.size(), 1);
        
        let value = cache.get(&"key1".to_string());
        assert_eq!(value, Some("value1".to_string()));
    }

    #[test]
    fn test_query_cache_expiration() {
        let cache: QueryCache<String, String> = QueryCache::new(Duration::from_millis(100), 100);
        
        cache.set("key1".to_string(), "value1".to_string());
        std::thread::sleep(Duration::from_millis(150));
        
        let value = cache.get(&"key1".to_string());
        assert_eq!(value, None);
    }

    #[test]
    fn test_query_cache_eviction() {
        let cache: QueryCache<String, String> = QueryCache::new(Duration::from_secs(60), 2);
        
        cache.set("key1".to_string(), "value1".to_string());
        cache.set("key2".to_string(), "value2".to_string());
        cache.set("key3".to_string(), "value3".to_string());
        
        assert_eq!(cache.size(), 2);
    }

    #[test]
    fn test_batch_processor() {
        let processor = BatchProcessor::new(3, Duration::from_secs(5));
        let items = vec![1, 2, 3, 4, 5, 6, 7];
        
        let batches = processor.process_batch(items);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 3);
        assert_eq!(batches[1].len(), 3);
        assert_eq!(batches[2].len(), 1);
    }

    #[test]
    fn test_connection_pool() {
        let pool = ConnectionPool::new(5, Duration::from_secs(10));
        
        let conn1 = pool.acquire();
        assert!(conn1.is_ok());
        
        let stats = pool.stats();
        assert_eq!(stats.available_connections, 4);
        assert_eq!(stats.in_use_connections, 1);
    }

    #[test]
    fn test_connection_pool_release() {
        let pool = ConnectionPool::new(5, Duration::from_secs(10));
        
        {
            let _conn = pool.acquire().unwrap();
            let stats = pool.stats();
            assert_eq!(stats.available_connections, 4);
        }
        
        let stats = pool.stats();
        assert_eq!(stats.available_connections, 5);
    }

    #[test]
    fn test_query_optimizer() {
        let mut optimizer = QueryOptimizer::new();
        optimizer.add_index_hint("SELECT * FROM validators".to_string(), "idx_validators".to_string());
        
        let optimized = optimizer.optimize_query("SELECT * FROM validators");
        assert!(optimized.contains("USE INDEX"));
    }

    #[test]
    fn test_memory_pool() {
        let pool: MemoryPool<String> = MemoryPool::new(5);
        
        pool.return_object("obj1".to_string());
        assert_eq!(pool.size(), 1);
        
        let obj = pool.acquire();
        assert_eq!(obj, Some("obj1".to_string()));
        assert_eq!(pool.size(), 0);
    }

    #[test]
    fn test_performance_monitor() {
        let monitor = PerformanceMonitor::new();
        
        monitor.record_query_time(100.0);
        monitor.record_query_time(150.0);
        monitor.record_query_time(120.0);
        monitor.record_cache_hit();
        monitor.record_cache_hit();
        monitor.record_cache_miss();
        
        let metrics = monitor.get_metrics();
        assert!(metrics.avg_query_time_ms > 0.0);
        assert!(metrics.cache_hit_rate > 0.0);
    }

    #[test]
    fn test_cache_stats() {
        let cache: QueryCache<String, String> = QueryCache::new(Duration::from_secs(60), 100);
        
        cache.set("key1".to_string(), "value1".to_string());
        cache.set("key2".to_string(), "value2".to_string());
        
        let stats = cache.stats();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.active_entries, 2);
    }

    #[test]
    fn test_pool_stats() {
        let pool = ConnectionPool::new(10, Duration::from_secs(10));
        
        let _conn1 = pool.acquire().unwrap();
        let _conn2 = pool.acquire().unwrap();
        
        let stats = pool.stats();
        assert_eq!(stats.pool_size, 10);
        assert_eq!(stats.in_use_connections, 2);
        assert_eq!(stats.available_connections, 8);
    }
}
