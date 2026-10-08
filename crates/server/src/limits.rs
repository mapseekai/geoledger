//! Configurable execution limits and token-bucket rate limiting.
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
/// A sustained rate with a burst allowance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub per_second: f64,
    pub burst: f64,
}
impl Rate {
    /// `None` when rate is zero (disabled).
    pub fn new(per_second: f64, burst: f64) -> Option<Self> {
        (per_second > 0.0).then(|| Self {
            per_second,
            burst: burst.max(1.0),
        })
    }
}
/// Server-wide execution limits. Defaults match previous fixed behaviour.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Concurrent application operations; extra calls are rejected with 429.
    pub max_concurrency: usize,
    /// Upper bound on any operation, including client-supplied gRPC deadlines.
    pub request_timeout: Duration,
    /// Per authenticated subject.
    pub per_subject: Option<Rate>,
    /// Per client IP, applied before authentication.
    pub per_ip: Option<Rate>,
    /// Use the right-most X-Forwarded-For address as the client IP (only behind a trusted gateway).
    pub trust_forwarded_for: bool,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_concurrency: 20,
            request_timeout: Duration::from_secs(30),
            per_subject: None,
            per_ip: None,
            trust_forwarded_for: false,
        }
    }
}
const MAX_KEYS: usize = 100_000;
pub(crate) struct Limiter {
    rate: Rate,
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}
impl Limiter {
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            buckets: Mutex::new(HashMap::new()),
        }
    }
    /// Take one token for `key`; false when the bucket is empty.
    pub fn allow(&self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }
    fn allow_at(&self, key: &str, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return true;
        };
        let Rate { per_second, burst } = self.rate;
        if buckets.len() >= MAX_KEYS && !buckets.contains_key(key) {
            // Forget keys whose bucket has refilled; they carry no state worth keeping.
            buckets.retain(|_, (tokens, at)| {
                *tokens + now.saturating_duration_since(*at).as_secs_f64() * per_second < burst
            });
            if buckets.len() >= MAX_KEYS {
                return false;
            }
        }
        let (tokens, at) = buckets.entry(key.to_owned()).or_insert((burst, now));
        *tokens =
            (*tokens + now.saturating_duration_since(*at).as_secs_f64() * per_second).min(burst);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}
/// Client IP for per-IP limits and access logs.
pub(crate) fn client_ip(
    peer: Option<IpAddr>,
    forwarded_for: Option<&str>,
    trust: bool,
) -> Option<IpAddr> {
    if trust
        && let Some(ip) = forwarded_for
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
    {
        return Some(ip);
    }
    peer
}
/// Accept a caller-supplied request ID when it is short and URL-safe, else mint one.
pub(crate) fn request_id(incoming: Option<&str>) -> String {
    match incoming {
        Some(id)
            if !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)) =>
        {
            id.to_owned()
        }
        _ => uuid::Uuid::new_v4().to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bucket_refills_and_bounds_burst() {
        let limiter = Limiter::new(Rate {
            per_second: 2.0,
            burst: 3.0,
        });
        let start = Instant::now();
        assert!((0..3).all(|_| limiter.allow_at("a", start)));
        assert!(!limiter.allow_at("a", start));
        assert!(limiter.allow_at("b", start));
        let later = start + Duration::from_millis(500);
        assert!(limiter.allow_at("a", later));
        assert!(!limiter.allow_at("a", later));
        let much_later = start + Duration::from_secs(60);
        assert!((0..3).all(|_| limiter.allow_at("a", much_later)));
        assert!(!limiter.allow_at("a", much_later));
    }
    #[test]
    fn request_ids_and_forwarded_addresses() {
        assert_eq!(request_id(Some("gw-123.abc")), "gw-123.abc");
        assert_ne!(request_id(Some("bad id\n")), "bad id\n");
        assert_eq!(request_id(Some(&"x".repeat(129))).len(), 36);
        let peer: IpAddr = [10, 0, 0, 1].into();
        assert_eq!(
            client_ip(Some(peer), Some("1.2.3.4, 5.6.7.8"), true),
            Some([5, 6, 7, 8].into())
        );
        assert_eq!(client_ip(Some(peer), Some("5.6.7.8"), false), Some(peer));
        assert_eq!(client_ip(Some(peer), Some("junk"), true), Some(peer));
    }
}
