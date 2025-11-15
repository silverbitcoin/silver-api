//! Rate limiting for API requests
//!
//! Implements token bucket algorithm for rate limiting per client IP.

use dashmap::DashMap;
use std::{
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::debug;

/// Token bucket for rate limiting
#[derive(Debug, Clone)]
struct TokenBucket {
    /// Current number of tokens
    tokens: f64,
    
    /// Maximum tokens (capacity)
    capacity: f64,
    
    /// Tokens added per second (refill rate)
    refill_rate: f64,
    
    /// Last refill time
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }
    
    /// Try to consume a token
    fn try_consume(&mut self) -> bool {
        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
        
        // Try to consume a token
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
    
    /// Get current token count
    fn tokens(&self) -> f64 {
        self.tokens
    }
}

/// Rate limiter using token bucket algorithm
///
/// Limits requests per second per client IP address.
pub struct RateLimiter {
    /// Token buckets per IP address
    buckets: Arc<DashMap<IpAddr, RwLock<TokenBucket>>>,
    
    /// Maximum requests per second per IP
    rate_limit: u32,
    
    /// Cleanup interval for inactive buckets
    cleanup_interval: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter
    ///
    /// # Arguments
    /// * `rate_limit` - Maximum requests per second per IP (default: 100)
    pub fn new(rate_limit: u32) -> Self {
        let limiter = Self {
            buckets: Arc::new(DashMap::new()),
            rate_limit,
            cleanup_interval: Duration::from_secs(300), // 5 minutes
        };
        
        // Start cleanup task
        limiter.start_cleanup_task();
        
        limiter
    }
    
    /// Check if a request from the given IP should be allowed
    ///
    /// Returns `true` if the request is allowed, `false` if rate limit exceeded.
    pub async fn check_rate_limit(&self, ip: IpAddr) -> bool {
        // Get or create bucket for this IP
        let bucket = self.buckets.entry(ip).or_insert_with(|| {
            RwLock::new(TokenBucket::new(
                self.rate_limit as f64,
                self.rate_limit as f64,
            ))
        });
        
        // Try to consume a token
        let mut bucket = bucket.write().await;
        let allowed = bucket.try_consume();
        
        if !allowed {
            debug!("Rate limit exceeded for IP: {}", ip);
        }
        
        allowed
    }
    
    /// Get current token count for an IP
    pub async fn get_tokens(&self, ip: IpAddr) -> Option<f64> {
        if let Some(bucket) = self.buckets.get(&ip) {
            let bucket = bucket.read().await;
            Some(bucket.tokens())
        } else {
            None
        }
    }
    
    /// Get the number of tracked IPs
    pub fn tracked_ips(&self) -> usize {
        self.buckets.len()
    }
    
    /// Clear all rate limit data
    pub fn clear(&self) {
        self.buckets.clear();
    }
    
    /// Start background task to cleanup inactive buckets
    fn start_cleanup_task(&self) {
        let buckets = self.buckets.clone();
        let interval = self.cleanup_interval;
        
        tokio::spawn(async move {
            let mut cleanup_interval = tokio::time::interval(interval);
            
            loop {
                cleanup_interval.tick().await;
                
                // Remove buckets that haven't been used recently
                let now = Instant::now();
                buckets.retain(|_, bucket| {
                    let bucket = bucket.blocking_read();
                    now.duration_since(bucket.last_refill) < interval
                });
                
                debug!("Rate limiter cleanup: {} IPs tracked", buckets.len());
            }
        });
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(100) // 100 req/s default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    
    #[test]
    fn test_token_bucket_consume() {
        let mut bucket = TokenBucket::new(10.0, 10.0);
        
        // Should be able to consume 10 tokens
        for _ in 0..10 {
            assert!(bucket.try_consume());
        }
        
        // Should fail on 11th token
        assert!(!bucket.try_consume());
    }
    
    #[test]
    fn test_token_bucket_refill() {
        let mut bucket = TokenBucket::new(10.0, 10.0);
        
        // Consume all tokens
        for _ in 0..10 {
            assert!(bucket.try_consume());
        }
        
        // Wait for refill
        std::thread::sleep(Duration::from_millis(200));
        
        // Should have refilled ~2 tokens (10 tokens/sec * 0.2 sec)
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
    }
    
    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(10);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        
        // Should allow 10 requests
        for _ in 0..10 {
            assert!(limiter.check_rate_limit(ip).await);
        }
        
        // Should block 11th request
        assert!(!limiter.check_rate_limit(ip).await);
    }
    
    #[tokio::test]
    async fn test_rate_limiter_multiple_ips() {
        let limiter = RateLimiter::new(5);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
        
        // Each IP should have independent limits
        for _ in 0..5 {
            assert!(limiter.check_rate_limit(ip1).await);
            assert!(limiter.check_rate_limit(ip2).await);
        }
        
        // Both should be blocked now
        assert!(!limiter.check_rate_limit(ip1).await);
        assert!(!limiter.check_rate_limit(ip2).await);
    }
    
    #[tokio::test]
    async fn test_rate_limiter_refill() {
        let limiter = RateLimiter::new(10);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        
        // Consume all tokens
        for _ in 0..10 {
            assert!(limiter.check_rate_limit(ip).await);
        }
        
        // Wait for refill
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // Should have refilled ~2 tokens
        assert!(limiter.check_rate_limit(ip).await);
        assert!(limiter.check_rate_limit(ip).await);
    }
    
    #[tokio::test]
    async fn test_rate_limiter_get_tokens() {
        let limiter = RateLimiter::new(10);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        
        // Initial tokens should be 10
        limiter.check_rate_limit(ip).await;
        let tokens = limiter.get_tokens(ip).await.unwrap();
        assert!((tokens - 9.0).abs() < 0.1);
    }
    
    #[tokio::test]
    async fn test_rate_limiter_clear() {
        let limiter = RateLimiter::new(10);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        
        limiter.check_rate_limit(ip).await;
        assert_eq!(limiter.tracked_ips(), 1);
        
        limiter.clear();
        assert_eq!(limiter.tracked_ips(), 0);
    }
}
