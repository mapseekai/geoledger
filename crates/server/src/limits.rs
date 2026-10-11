//! Configurable execution limits and token-bucket rate limiting.
use std::{
    collections::{BTreeSet, HashMap},
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
        (per_second.is_finite() && per_second > 0.0 && burst.is_finite()).then(|| Self {
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
    /// Maximum encoded request, including a complete SaveStream upload.
    pub max_request_bytes: usize,
    /// Maximum encoded response. Lower page sizes can be used for large features.
    pub max_response_bytes: usize,
    /// Dynamically growing reservations cover receive/decode until the operation ends.
    pub request_memory_bytes: usize,
    /// Dynamic reservations cover query/encoding and remain with transport-owned bytes.
    pub response_memory_bytes: usize,
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
            max_request_bytes: 64 * 1024 * 1024,
            max_response_bytes: 64 * 1024 * 1024,
            request_memory_bytes: 256 * 1024 * 1024,
            response_memory_bytes: 256 * 1024 * 1024,
            per_subject: None,
            per_ip: None,
            trust_forwarded_for: false,
        }
    }
}
const MAX_KEYS: usize = 100_000;
struct Bucket {
    tokens: f64,
    at: Instant,
    expires: Option<Instant>,
}
#[derive(Default)]
struct Buckets {
    values: HashMap<String, Bucket>,
    // Exactly one expiry entry per expiring key: no stale heap entries can accumulate.
    expirations: BTreeSet<(Instant, String)>,
}
pub(crate) struct Limiter {
    rate: Rate,
    buckets: Mutex<Buckets>,
}
impl Limiter {
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            buckets: Mutex::default(),
        }
    }
    pub fn allow(&self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }
    fn allow_at(&self, key: &str, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        let Rate { per_second, burst } = self.rate;
        if !per_second.is_finite() || per_second <= 0.0 || !burst.is_finite() || burst < 1.0 {
            return false;
        }
        // Bounded retirement work, including when full. No full-map scan on rejection.
        for _ in 0..16 {
            if buckets.expirations.first().is_none_or(|(at, _)| *at > now) {
                break;
            }
            if let Some((_, key)) = buckets.expirations.pop_first() {
                buckets.values.remove(&key);
            }
        }
        if buckets.values.len() >= MAX_KEYS && !buckets.values.contains_key(key) {
            return false;
        }
        let mut bucket = buckets.values.remove(key).unwrap_or(Bucket {
            tokens: burst,
            at: now,
            expires: None,
        });
        if let Some(at) = bucket.expires {
            buckets.expirations.remove(&(at, key.to_owned()));
        }
        bucket.tokens = (bucket.tokens
            + now.saturating_duration_since(bucket.at).as_secs_f64() * per_second)
            .min(burst);
        bucket.at = now;
        let allowed = bucket.tokens >= 1.0;
        if allowed {
            bucket.tokens -= 1.0;
        }
        bucket.expires = Duration::try_from_secs_f64((burst - bucket.tokens) / per_second)
            .ok()
            .and_then(|duration| now.checked_add(duration));
        if let Some(at) = bucket.expires {
            buckets.expirations.insert((at, key.to_owned()));
        }
        buckets.values.insert(key.to_owned(), bucket);
        allowed
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
    fn saturated_unknown_keys_do_not_scan_or_reset_existing_buckets() {
        let limiter = Limiter::new(Rate {
            per_second: 0.001,
            burst: 1.0,
        });
        let now = Instant::now();
        for i in 0..MAX_KEYS {
            assert!(limiter.allow_at(&i.to_string(), now));
        }
        for _ in 0..1000 {
            assert!(!limiter.allow_at("absent", now));
        }
        assert!(!limiter.allow_at("0", now));
        assert_eq!(
            limiter
                .buckets
                .lock()
                .map(|b| (b.values.len(), b.expirations.len()))
                .ok(),
            Some((MAX_KEYS, MAX_KEYS))
        );
        assert!(limiter.allow_at("new", now + Duration::from_secs(1001)));
        assert!(Rate::new(f64::INFINITY, 1.0).is_none());
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
