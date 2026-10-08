//! Bounded-cardinality Prometheus metrics. Never label by subject, project,
//! feature ID, arbitrary path, token or request ID.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
const BUCKETS: [u64; 6] = [10_000, 100_000, 500_000, 1_000_000, 5_000_000, 30_000_000];
#[derive(Default)]
pub(crate) struct Metrics {
    requests: AtomicU64,
    errors: AtomicU64,
    conflicts: AtomicU64,
    busy: AtomicU64,
    micros: AtomicU64,
    buckets: [AtomicU64; 6],
}
impl Metrics {
    pub fn observe(&self, status: u16, micros: u64) {
        self.requests.fetch_add(1, Relaxed);
        self.micros.fetch_add(micros, Relaxed);
        if status >= 400 {
            self.errors.fetch_add(1, Relaxed);
        }
        if status == 409 {
            self.conflicts.fetch_add(1, Relaxed);
        }
        if status == 429 {
            self.busy.fetch_add(1, Relaxed);
        }
        for (bound, count) in BUCKETS.iter().zip(&self.buckets) {
            if micros <= *bound {
                count.fetch_add(1, Relaxed);
            }
        }
    }
    pub fn render(&self, available: usize) -> String {
        let count = self.requests.load(Relaxed);
        let mut out = String::new();
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
                "conflicts_total",
                "Version, merge or idempotency conflicts",
                self.conflicts.load(Relaxed),
            ),
            (
                "busy_total",
                "Application capacity or database lock rejections",
                self.busy.load(Relaxed),
            ),
        ] {
            out.push_str(&format!("# HELP geoledger_{name} {help}\n# TYPE geoledger_{name} counter\ngeoledger_{name} {value}\n"));
        }
        out.push_str("# HELP geoledger_request_duration_seconds Application execution duration\n# TYPE geoledger_request_duration_seconds histogram\n");
        for (bound, value) in BUCKETS.iter().zip(&self.buckets) {
            out.push_str(&format!(
                "geoledger_request_duration_seconds_bucket{{le=\"{}\"}} {}\n",
                *bound as f64 / 1e6,
                value.load(Relaxed)
            ));
        }
        out.push_str(&format!("geoledger_request_duration_seconds_bucket{{le=\"+Inf\"}} {count}\ngeoledger_request_duration_seconds_count {count}\ngeoledger_request_duration_seconds_sum {}\n# HELP geoledger_execution_slots_available Free execution slots\n# TYPE geoledger_execution_slots_available gauge\ngeoledger_execution_slots_available {available}\n", self.micros.load(Relaxed) as f64/1e6));
        out
    }
}
