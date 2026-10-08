//! Bounded-cardinality Prometheus metrics. Never label by subject, project,
//! feature ID, arbitrary path, token or request ID: operations are mapped onto
//! a fixed list and statuses onto the small set the application returns.
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicI64, AtomicU64, Ordering::Relaxed},
    },
};
const BUCKETS: [u64; 8] = [
    5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000, 5_000_000, 30_000_000,
];
/// Every application operation name; anything else is reported as `other`.
pub(crate) const OPERATIONS: &[&str] = &[
    "info",
    "create_project",
    "list_projects",
    "get_project",
    "set_member",
    "create_dataset",
    "list_datasets",
    "create_workspace",
    "list_workspaces",
    "get_workspace",
    "save",
    "discard",
    "features",
    "diff",
    "conflicts",
    "history",
    "commit",
    "audit",
    "publish",
    "resolve",
    "rebase",
    "restore",
    "list_members",
    "remove_member",
    "archive_project",
    "delete_project",
];
pub(crate) fn operation_label(op: &str) -> &'static str {
    OPERATIONS
        .iter()
        .find(|known| **known == op)
        .copied()
        .unwrap_or("other")
}
#[derive(Default)]
struct Histogram {
    buckets: [u64; BUCKETS.len()],
    count: u64,
    micros: u64,
}
impl Histogram {
    fn observe(&mut self, micros: u64) {
        self.count += 1;
        self.micros = self.micros.saturating_add(micros);
        for (bound, count) in BUCKETS.iter().zip(&mut self.buckets) {
            if micros <= *bound {
                *count += 1;
            }
        }
    }
}
type OpKey = (&'static str, &'static str);
#[derive(Default)]
struct Labeled {
    calls: BTreeMap<(&'static str, &'static str, u16), u64>,
    durations: BTreeMap<OpKey, Histogram>,
}
#[derive(Default)]
pub(crate) struct Metrics {
    requests: AtomicU64,
    errors: AtomicU64,
    conflicts: AtomicU64,
    busy: AtomicU64,
    overall: Mutex<Histogram>,
    labeled: Mutex<Labeled>,
    auth_failures: AtomicU64,
    rate_limited_subject: AtomicU64,
    rate_limited_ip: AtomicU64,
    internal_errors: AtomicU64,
    in_flight: AtomicI64,
}
/// Values sampled at scrape time.
pub(crate) struct Gauges {
    pub slots_available: usize,
    pub slots_capacity: usize,
    pub draining: bool,
    pub pool: Option<geoledger_engine::PoolStats>,
}
pub(crate) struct InFlight<'a>(&'a Metrics);
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Relaxed);
    }
}
impl Metrics {
    /// Record one completed (or rejected) application call.
    pub fn observe(&self, protocol: &'static str, op: &str, status: u16, micros: u64) {
        self.requests.fetch_add(1, Relaxed);
        if status >= 400 {
            self.errors.fetch_add(1, Relaxed);
        }
        if status >= 500 {
            self.internal_errors.fetch_add(1, Relaxed);
        }
        if status == 409 {
            self.conflicts.fetch_add(1, Relaxed);
        }
        if status == 429 {
            self.busy.fetch_add(1, Relaxed);
        }
        if let Ok(mut h) = self.overall.lock() {
            h.observe(micros);
        }
        let op = operation_label(op);
        let status = match status {
            200 | 400 | 401 | 403 | 404 | 408 | 409 | 413 | 422 | 429 | 500 | 503 | 504 => status,
            s if s >= 500 => 500,
            _ => 400,
        };
        if let Ok(mut l) = self.labeled.lock() {
            *l.calls.entry((protocol, op, status)).or_default() += 1;
            l.durations
                .entry((protocol, op))
                .or_default()
                .observe(micros);
        }
    }
    pub fn auth_failure(&self) {
        self.auth_failures.fetch_add(1, Relaxed);
    }
    pub fn rate_limited(&self, per_ip: bool) {
        if per_ip {
            self.rate_limited_ip.fetch_add(1, Relaxed);
        } else {
            self.rate_limited_subject.fetch_add(1, Relaxed);
        }
    }
    pub fn in_flight(&self) -> InFlight<'_> {
        self.in_flight.fetch_add(1, Relaxed);
        InFlight(self)
    }
    pub fn render(&self, gauges: &Gauges) -> String {
        let mut out = String::new();
        let mut line = |s: String| {
            out.push_str(&s);
            out.push('\n');
        };
        let count = self.requests.load(Relaxed);
        line(format!(
            "# HELP geoledger_build_info Server version\n# TYPE geoledger_build_info gauge\ngeoledger_build_info{{version=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION")
        ));
        for (name, help, value) in [
            (
                "requests_total",
                "Completed application calls and capacity rejections",
                count,
            ),
            (
                "errors_total",
                "Application calls rejected or failed",
                self.errors.load(Relaxed),
            ),
            (
                "internal_errors_total",
                "Application calls that failed with a 5xx status",
                self.internal_errors.load(Relaxed),
            ),
            (
                "conflicts_total",
                "Version, merge or idempotency conflicts",
                self.conflicts.load(Relaxed),
            ),
            (
                "busy_total",
                "Capacity, rate-limit or database lock rejections",
                self.busy.load(Relaxed),
            ),
            (
                "auth_failures_total",
                "Requests rejected because the credential was missing, invalid or expired",
                self.auth_failures.load(Relaxed),
            ),
        ] {
            line(format!(
                "# HELP geoledger_{name} {help}\n# TYPE geoledger_{name} counter\ngeoledger_{name} {value}"
            ));
        }
        line("# HELP geoledger_rate_limited_total Requests rejected by rate limiting\n# TYPE geoledger_rate_limited_total counter".into());
        line(format!(
            "geoledger_rate_limited_total{{scope=\"subject\"}} {}",
            self.rate_limited_subject.load(Relaxed)
        ));
        line(format!(
            "geoledger_rate_limited_total{{scope=\"ip\"}} {}",
            self.rate_limited_ip.load(Relaxed)
        ));
        line("# HELP geoledger_request_duration_seconds Application execution duration\n# TYPE geoledger_request_duration_seconds histogram".into());
        if let Ok(h) = self.overall.lock() {
            histogram(&mut line, "geoledger_request_duration_seconds", "", &h);
        }
        if let Ok(l) = self.labeled.lock() {
            line("# HELP geoledger_operations_total Application calls by protocol, operation and status\n# TYPE geoledger_operations_total counter".into());
            for ((protocol, op, status), value) in &l.calls {
                line(format!(
                    "geoledger_operations_total{{protocol=\"{protocol}\",operation=\"{op}\",status=\"{status}\"}} {value}"
                ));
            }
            line("# HELP geoledger_operation_duration_seconds Application call duration by protocol and operation\n# TYPE geoledger_operation_duration_seconds histogram".into());
            for ((protocol, op), h) in &l.durations {
                histogram(
                    &mut line,
                    "geoledger_operation_duration_seconds",
                    &format!("protocol=\"{protocol}\",operation=\"{op}\","),
                    h,
                );
            }
        }
        line(format!(
            "# HELP geoledger_in_flight_requests Application calls currently executing\n# TYPE geoledger_in_flight_requests gauge\ngeoledger_in_flight_requests {}",
            self.in_flight.load(Relaxed).max(0)
        ));
        line(format!(
            "# HELP geoledger_execution_slots_available Free execution slots\n# TYPE geoledger_execution_slots_available gauge\ngeoledger_execution_slots_available {}",
            gauges.slots_available
        ));
        line(format!(
            "# HELP geoledger_execution_slots_capacity Configured execution slots\n# TYPE geoledger_execution_slots_capacity gauge\ngeoledger_execution_slots_capacity {}",
            gauges.slots_capacity
        ));
        line(format!(
            "# HELP geoledger_draining 1 while the server is shutting down\n# TYPE geoledger_draining gauge\ngeoledger_draining {}",
            u8::from(gauges.draining)
        ));
        if let Some(pool) = gauges.pool {
            line(format!(
                "# HELP geoledger_db_pool_capacity Maximum PostgreSQL connections\n# TYPE geoledger_db_pool_capacity gauge\ngeoledger_db_pool_capacity {}",
                pool.capacity
            ));
            line(format!(
                "# HELP geoledger_db_pool_connections PostgreSQL connections by state\n# TYPE geoledger_db_pool_connections gauge\ngeoledger_db_pool_connections{{state=\"active\"}} {}\ngeoledger_db_pool_connections{{state=\"idle\"}} {}",
                pool.active, pool.idle
            ));
            line(format!(
                "# HELP geoledger_db_pool_wait_timeouts_total Connection acquisitions that hit the operation deadline\n# TYPE geoledger_db_pool_wait_timeouts_total counter\ngeoledger_db_pool_wait_timeouts_total {}",
                pool.wait_timeouts
            ));
        }
        out
    }
}
fn histogram(line: &mut impl FnMut(String), name: &str, labels: &str, h: &Histogram) {
    for (bound, value) in BUCKETS.iter().zip(&h.buckets) {
        line(format!(
            "{name}_bucket{{{labels}le=\"{}\"}} {value}",
            *bound as f64 / 1e6
        ));
    }
    let bare = labels.trim_end_matches(',');
    let braces = if bare.is_empty() {
        String::new()
    } else {
        format!("{{{bare}}}")
    };
    line(format!(
        "{name}_bucket{{{labels}le=\"+Inf\"}} {}\n{name}_count{braces} {}\n{name}_sum{braces} {}",
        h.count,
        h.count,
        h.micros as f64 / 1e6
    ));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_stay_bounded() {
        let m = Metrics::default();
        m.observe("http", "save", 200, 1_000);
        m.observe("grpc", "../../etc/passwd", 418, 1_000);
        m.observe("grpc", "publish", 504, 40_000_000);
        let text = m.render(&Gauges {
            slots_available: 3,
            slots_capacity: 4,
            draining: false,
            pool: None,
        });
        assert!(text.contains(
            "geoledger_operations_total{protocol=\"http\",operation=\"save\",status=\"200\"} 1"
        ));
        assert!(text.contains("operation=\"other\",status=\"400\""));
        assert!(!text.contains("passwd"));
        assert!(text.contains(
            "geoledger_operation_duration_seconds_count{protocol=\"grpc\",operation=\"publish\"} 1"
        ));
        assert!(text.contains("geoledger_requests_total 3"));
        assert!(text.contains("geoledger_internal_errors_total 1"));
    }
}
