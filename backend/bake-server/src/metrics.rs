//! Prometheus metrics for the bake-server — Q097 SLO instrumentation.
//!
//! Exposes SLIs defined in `docs/slos.md`:
//!
//! - `wb_tile_fetch_duration_seconds` — wall-clock for `/v1/tile/*` responses,
//!   labelled by `tier` (`pinned` | `hot` | `cold`) + `universe`.
//! - `wb_bake_duration_seconds` — cold-bake latency (enqueue → manifest emit).
//! - `wb_manifest_size_bytes` — uncompressed manifest size at emit time.
//! - `wb_cache_hits_total` / `wb_cache_misses_total` — counters per tier+universe.
//! - `wb_bake_failures_total` — counter for failed bakes.
//! - `wb_api_requests_total` — total + 5xx counter for availability SLI.
//!
//! All histograms use **exponential 5 ms → 8 s buckets** to match the burn-rate
//! PromQL in `wb-rules.yml`. The cardinality is bounded:
//! - `tier` has 3 values
//! - `universe` is expected ≤ 10 in practice; downstream Prom config drops
//!   high-cardinality labels in `metric_relabel_configs` if needed.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
};
use once_cell::sync::Lazy;
use prometheus::{
    histogram_opts, opts, Encoder, HistogramVec, IntCounterVec, Registry, TextEncoder,
};
use std::sync::Arc;

/// Cache tier classification used as a histogram label. Matches the
/// `X-WB-Cache-Tier` response header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Pre-pinned, always-hot tiles (top-N busiest).
    Pinned,
    /// Recently-served, kept warm by LRU.
    Hot,
    /// Cold miss — required a bake.
    Cold,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Pinned => "pinned",
            Tier::Hot => "hot",
            Tier::Cold => "cold",
        }
    }
}

/// Exponential bucket layout from 5 ms to 8 s, matching the SLO hard upper
/// bounds (cached p95 < 150 ms; cold bake p95 < 8 s).
fn latency_buckets() -> Vec<f64> {
    // 0.005, 0.010, 0.025, 0.050, 0.100, 0.150, 0.250, 0.500, 1.0, 2.0, 4.0, 8.0, 12.0
    vec![
        0.005, 0.010, 0.025, 0.050, 0.100, 0.150, 0.250, 0.500, 1.0, 2.0, 4.0, 8.0, 12.0,
    ]
}

/// Manifest size buckets, 1 KB → 4 MB, with 2 MB (p99 target) + 2.5 MB (hard
/// cap) explicitly bucketed for precise SLO measurement.
fn manifest_size_buckets() -> Vec<f64> {
    vec![
        1_024.0,
        10_240.0,
        102_400.0,
        524_288.0,
        1_048_576.0,
        1_572_864.0,
        2_097_152.0, // 2 MB — p99 SLO target
        2_621_440.0, // 2.5 MB — hard cap
        4_194_304.0,
    ]
}

/// All Prometheus collectors live behind a single [`Metrics`] handle so tests
/// can construct a fresh registry per test (avoiding global-state pollution).
pub struct Metrics {
    pub registry: Registry,
    pub tile_fetch_duration: HistogramVec,
    pub bake_duration: HistogramVec,
    pub manifest_size: HistogramVec,
    pub cache_hits: IntCounterVec,
    pub cache_misses: IntCounterVec,
    pub bake_failures: IntCounterVec,
    pub api_requests: IntCounterVec,
}

impl Metrics {
    /// Build a fresh metrics set. Each call creates a new [`Registry`], so
    /// integration tests can isolate state.
    pub fn new() -> Self {
        let registry = Registry::new();

        let tile_fetch_duration = HistogramVec::new(
            histogram_opts!(
                "wb_tile_fetch_duration_seconds",
                "Tile fetch wall-clock latency (request received → response sent)"
            )
            .buckets(latency_buckets()),
            &["tier", "universe"],
        )
        .expect("histogram");
        registry
            .register(Box::new(tile_fetch_duration.clone()))
            .expect("register tile_fetch_duration");

        let bake_duration = HistogramVec::new(
            histogram_opts!(
                "wb_bake_duration_seconds",
                "Cold-bake latency (enqueue → manifest emit)"
            )
            .buckets(latency_buckets()),
            &["universe"],
        )
        .expect("histogram");
        registry
            .register(Box::new(bake_duration.clone()))
            .expect("register bake_duration");

        let manifest_size = HistogramVec::new(
            histogram_opts!(
                "wb_manifest_size_bytes",
                "Uncompressed manifest size at emit time, in bytes"
            )
            .buckets(manifest_size_buckets()),
            &["universe"],
        )
        .expect("histogram");
        registry
            .register(Box::new(manifest_size.clone()))
            .expect("register manifest_size");

        let cache_hits = IntCounterVec::new(
            opts!("wb_cache_hits_total", "Cache hits by tier"),
            &["tier", "universe"],
        )
        .expect("counter");
        registry
            .register(Box::new(cache_hits.clone()))
            .expect("register cache_hits");

        let cache_misses = IntCounterVec::new(
            opts!("wb_cache_misses_total", "Cache misses (cold bakes)"),
            &["universe"],
        )
        .expect("counter");
        registry
            .register(Box::new(cache_misses.clone()))
            .expect("register cache_misses");

        let bake_failures = IntCounterVec::new(
            opts!("wb_bake_failures_total", "Failed bake jobs"),
            &["universe", "reason"],
        )
        .expect("counter");
        registry
            .register(Box::new(bake_failures.clone()))
            .expect("register bake_failures");

        let api_requests = IntCounterVec::new(
            opts!(
                "wb_api_requests_total",
                "Total API requests, labelled by result class (2xx/4xx/5xx). 5xx counts against SLO 3."
            ),
            &["endpoint", "result"],
        )
        .expect("counter");
        registry
            .register(Box::new(api_requests.clone()))
            .expect("register api_requests");

        Self {
            registry,
            tile_fetch_duration,
            bake_duration,
            manifest_size,
            cache_hits,
            cache_misses,
            bake_failures,
            api_requests,
        }
    }

    /// Record a completed tile fetch.
    pub fn observe_fetch(&self, tier: Tier, universe: &str, secs: f64) {
        self.tile_fetch_duration
            .with_label_values(&[tier.as_str(), universe])
            .observe(secs);
        match tier {
            Tier::Cold => self.cache_misses.with_label_values(&[universe]).inc(),
            _ => self
                .cache_hits
                .with_label_values(&[tier.as_str(), universe])
                .inc(),
        }
    }

    /// Record a completed bake.
    pub fn observe_bake(&self, universe: &str, secs: f64, manifest_bytes: usize) {
        self.bake_duration
            .with_label_values(&[universe])
            .observe(secs);
        self.manifest_size
            .with_label_values(&[universe])
            .observe(manifest_bytes as f64);
    }

    /// Record only manifest size (e.g. when the bake was cached upstream and
    /// we just served the bytes).
    pub fn observe_manifest_size(&self, universe: &str, bytes: usize) {
        self.manifest_size
            .with_label_values(&[universe])
            .observe(bytes as f64);
    }

    /// Record a failed bake.
    pub fn observe_bake_failure(&self, universe: &str, reason: &str) {
        self.bake_failures
            .with_label_values(&[universe, reason])
            .inc();
    }

    /// Record an API response class for the availability SLI.
    pub fn observe_response(&self, endpoint: &str, status: u16) {
        let result = match status {
            200..=299 => "2xx",
            400..=499 => "4xx",
            500..=599 => "5xx",
            _ => "other",
        };
        self.api_requests
            .with_label_values(&[endpoint, result])
            .inc();
    }

    /// Render the registry as Prometheus text exposition.
    pub fn render(&self) -> Vec<u8> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buf = Vec::new();
        encoder
            .encode(&metric_families, &mut buf)
            .expect("encode prometheus");
        buf
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide metrics handle used by the live HTTP server. Tests should
/// construct their own [`Metrics`] rather than poking this.
pub static GLOBAL: Lazy<Arc<Metrics>> = Lazy::new(|| Arc::new(Metrics::new()));

/// Axum handler for `GET /metrics`. Returns Prometheus text exposition.
pub async fn metrics_handler(State(m): State<Arc<Metrics>>) -> impl IntoResponse {
    let body = m.render();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_render_with_one_observation_each() {
        let m = Metrics::new();
        // Prometheus exposition only emits families that have at least one
        // observed labelled child. Touch each family once.
        m.observe_fetch(Tier::Hot, "demo", 0.01);
        m.observe_bake("demo", 1.0, 1024);
        m.observe_bake_failure("demo", "timeout");
        m.observe_response("/v1/tile/:z/:x/:y", 200);
        let body = String::from_utf8(m.render()).unwrap();
        assert!(body.contains("wb_tile_fetch_duration_seconds"));
        assert!(body.contains("wb_bake_duration_seconds"));
        assert!(body.contains("wb_manifest_size_bytes"));
        assert!(body.contains("wb_cache_hits_total"));
        assert!(body.contains("wb_bake_failures_total"));
        assert!(body.contains("wb_api_requests_total"));
    }

    #[test]
    fn observing_fetch_increments_hit_counter() {
        let m = Metrics::new();
        m.observe_fetch(Tier::Hot, "demo", 0.042);
        m.observe_fetch(Tier::Hot, "demo", 0.080);
        m.observe_fetch(Tier::Cold, "demo", 6.0);

        let body = String::from_utf8(m.render()).unwrap();
        assert!(body.contains("wb_cache_hits_total{tier=\"hot\",universe=\"demo\"} 2"));
        assert!(body.contains("wb_cache_misses_total{universe=\"demo\"} 1"));
    }

    #[test]
    fn p95_bucket_boundary_is_150ms_for_fetch() {
        let m = Metrics::new();
        // 100 fast + 5 slow → p95 should land at 0.150 bucket.
        for _ in 0..100 {
            m.observe_fetch(Tier::Pinned, "demo", 0.020);
        }
        for _ in 0..5 {
            m.observe_fetch(Tier::Pinned, "demo", 0.140);
        }
        let body = String::from_utf8(m.render()).unwrap();
        // The 0.15 bucket should be cumulative count >= 105.
        let line = body
            .lines()
            .find(|l| {
                l.contains("wb_tile_fetch_duration_seconds_bucket")
                    && l.contains("tier=\"pinned\"")
                    && l.contains("le=\"0.15\"")
            })
            .expect("0.15 bucket present");
        let count: u64 = line.split_whitespace().last().unwrap().parse().unwrap();
        assert!(count >= 105, "expected >=105 in 150ms bucket, got {count}");
    }

    #[test]
    fn manifest_size_2mb_bucket_present() {
        let m = Metrics::new();
        m.observe_bake("demo", 4.5, 1_900_000); // under 2 MB
        m.observe_bake("demo", 5.5, 2_400_000); // between 2 and 2.5 MB
        let body = String::from_utf8(m.render()).unwrap();
        assert!(body.contains("wb_manifest_size_bytes_bucket"));
        assert!(body.contains("le=\"2097152\""));
    }

    #[test]
    fn response_class_buckets_5xx_for_availability_sli() {
        let m = Metrics::new();
        m.observe_response("/v1/tile/:z/:x/:y", 200);
        m.observe_response("/v1/tile/:z/:x/:y", 200);
        m.observe_response("/v1/tile/:z/:x/:y", 404);
        m.observe_response("/v1/tile/:z/:x/:y", 503);
        let body = String::from_utf8(m.render()).unwrap();
        assert!(body.contains("result=\"2xx\"} 2"));
        assert!(body.contains("result=\"4xx\"} 1"));
        assert!(body.contains("result=\"5xx\"} 1"));
    }

    #[test]
    fn render_is_valid_prometheus_exposition_header() {
        // Touch each family with one observation so it is emitted.
        let m = Metrics::new();
        m.observe_fetch(Tier::Hot, "demo", 0.01);
        m.observe_fetch(Tier::Cold, "demo", 5.0); // populates cache_misses + cold tier
        m.observe_bake("demo", 1.0, 1024);
        m.observe_bake_failure("demo", "timeout");
        m.observe_response("/v1/tile/:z/:x/:y", 200);
        let body = String::from_utf8(m.render()).unwrap();
        for family in [
            "wb_tile_fetch_duration_seconds",
            "wb_bake_duration_seconds",
            "wb_manifest_size_bytes",
            "wb_cache_hits_total",
            "wb_cache_misses_total",
            "wb_bake_failures_total",
            "wb_api_requests_total",
        ] {
            let help_line = format!("# HELP {family}");
            let type_line = format!("# TYPE {family}");
            assert!(body.contains(&help_line), "missing HELP for {family}");
            assert!(body.contains(&type_line), "missing TYPE for {family}");
        }
    }

    /// Simulate a burn-rate alert firing in-process by computing the same
    /// ratio Prometheus would. With a 14.4× burn rate on a 0.1% error budget,
    /// 1.44% of requests must be 5xx.
    #[test]
    fn burn_rate_14_4x_alert_triggers_at_1_44_percent_5xx() {
        let m = Metrics::new();
        // 985 OK + 15 5xx = 1.5% error rate, comfortably above 14.4× burn.
        for _ in 0..985 {
            m.observe_response("/v1/tile/:z/:x/:y", 200);
        }
        for _ in 0..15 {
            m.observe_response("/v1/tile/:z/:x/:y", 503);
        }
        // Sum families ourselves — mimic the alert query.
        let mfs = m.registry.gather();
        let api = mfs
            .iter()
            .find(|f| f.get_name() == "wb_api_requests_total")
            .expect("api family");
        let mut total = 0u64;
        let mut errs = 0u64;
        for metric in api.get_metric() {
            let c = metric.get_counter().get_value() as u64;
            total += c;
            for lp in metric.get_label() {
                if lp.get_name() == "result" && lp.get_value() == "5xx" {
                    errs += c;
                }
            }
        }
        let ratio = errs as f64 / total as f64;
        let threshold = 14.4 * 0.001;
        assert!(
            ratio >= threshold,
            "burn rate {ratio} should exceed alert threshold {threshold}"
        );
    }
}
