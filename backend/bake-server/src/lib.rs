//! Worldbuilders bake-server library — extracted from `main.rs` so the
//! HTTP layer (router + HMAC verification + handlers) can be unit/integration
//! tested via `axum-test` without binding to a real socket (Q475).

use ::hmac::{Hmac, Mac};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use serde_json::json;
use sha2::Sha256;
use std::{path::PathBuf, sync::Arc, time::SystemTime};
use tracing::{info, warn};

use bake_queue::{producer::Producer, BakeJob};

pub mod cache;
pub mod hmac;
pub mod metrics;
pub mod schema_version;
pub mod sla;

// Q088: re-export the rotation-aware HMAC primitives. The flat
// `sign_payload` / `verify_hmac` below remain the legacy single-key
// path used by Q475 tests and by `BAKE_HMAC_KEY` deployments. New
// callers (per-universe rotation, scripts/rotate-hmac.sh, the systemd
// timer, the verifier in Roblox) should use the keyring API.
pub use crate::hmac::{
    decrypt_key_bytes, encrypt_key_bytes, generate_key_bytes, keyring_from_rows,
    load_from_env_legacy, load_or_init_master_key, new_key_id, unwrap_key_row, HmacKey,
    HmacKeyring, KeyStatus, HEADER_KEY_ID, HMAC_KEY_LEN, MASTER_KEK_LEN, OVERLAP_WINDOW_DAYS,
    ROTATION_INTERVAL_DAYS,
};
pub use cache::TileCacheWriter;
use cache::{CacheTier, LayeredCache, TileCache, TileKey};
use metrics::{metrics_handler, Metrics, Tier};
use sla::{
    run_sync_bake, BakeExecutor, BakeOutcome, JobRegistry, JobState, SYNC_BAKE_SOFT_DEADLINE,
};

type HmacSha256 = Hmac<Sha256>;

/// Skew tolerance window in seconds (clock drift between client and server).
pub const HMAC_TS_SKEW_SECS: u64 = 60;

#[derive(Clone)]
pub struct AppState {
    pub hmac_key: Arc<Vec<u8>>,
    /// Directory containing baked tile manifests as `<z>-<x>-<y>.json`.
    /// When `None`, the legacy mock-manifest path is used (kept so the
    /// Q475 HTTP tests stay green without a filesystem dependency).
    pub cache_dir: Option<Arc<PathBuf>>,
    /// Q081: producer onto the bake-queue. When configured, a cache
    /// miss enqueues a `BakeJob` and returns `202 Accepted` with a
    /// `Retry-After` hint instead of 503. When `None`, the legacy
    /// 503 path is preserved so existing tests keep passing.
    pub bake_producer: Option<Arc<Producer>>,
    /// Style version reported in enqueued bake jobs.
    pub style_version: u32,
    /// OSM snapshot reported in enqueued bake jobs (typically the
    /// snapshot collector's most recent ISO date).
    pub osm_snapshot: Arc<String>,
    /// Retry-After hint (seconds) returned alongside 202.
    pub retry_after_secs: u32,
    /// Q097: SLI/SLO instrumentation. Each AppState carries its own
    /// `Metrics` so integration tests can isolate registries.
    pub metrics: Arc<Metrics>,
    /// Universe label applied to all observations from this server.
    /// Cardinality is bounded — see `metrics.rs`.
    pub universe: Arc<String>,
    /// Q082: layered cache (pinned + hot LRU + cold disk). When set,
    /// supersedes the raw `cache_dir` lookup path. Tests that need only
    /// the legacy behaviour leave it `None`.
    pub layered_cache: Option<Arc<LayeredCache>>,
    /// Q084: synchronous-bake executor. When attached, the
    /// `POST /v1/tile/:z/:x/:y/bake` endpoint attempts a synchronous
    /// bake under [`SYNC_BAKE_SOFT_DEADLINE`] and falls back to
    /// 202 + placeholder + bake_id on breach.
    pub bake_executor: Option<Arc<dyn BakeExecutor>>,
    /// Q084: job registry for placeholder→full bake handoff.
    pub job_registry: Arc<JobRegistry>,
    /// Q084: configurable soft deadline (kept overridable so tests can
    /// trigger breach with millisecond delays).
    pub bake_soft_deadline: std::time::Duration,
}

impl AppState {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            hmac_key: Arc::new(key),
            cache_dir: None,
            bake_producer: None,
            style_version: 1,
            osm_snapshot: Arc::new("unknown".into()),
            retry_after_secs: 8, // Q084 SLA budget
            metrics: Arc::new(Metrics::new()),
            universe: Arc::new("default".into()),
            layered_cache: None,
            bake_executor: None,
            job_registry: Arc::new(JobRegistry::default()),
            bake_soft_deadline: SYNC_BAKE_SOFT_DEADLINE,
        }
    }

    /// Q084: attach a synchronous-bake executor. Enables the
    /// `POST /v1/tile/:z/:x/:y/bake` and `GET .../job/:bake_id`
    /// endpoints.
    pub fn with_bake_executor(mut self, exec: Arc<dyn BakeExecutor>) -> Self {
        self.bake_executor = Some(exec);
        self
    }

    /// Q084: override the soft deadline (tests).
    pub fn with_bake_soft_deadline(mut self, d: std::time::Duration) -> Self {
        self.bake_soft_deadline = d;
        self
    }

    /// Q082: attach a [`LayeredCache`] (pinned + hot LRU + cold disk).
    /// Supersedes [`Self::with_cache_dir`] when both are set; the
    /// `cache_dir` field is preserved so legacy diagnostics keep working.
    pub fn with_layered_cache(mut self, c: Arc<LayeredCache>) -> Self {
        self.layered_cache = Some(c);
        self
    }

    /// Q097: attach a shared metrics handle (e.g. the global one from
    /// `metrics::GLOBAL`). Tests pass per-test handles for isolation.
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Q097: tag observations with the operating universe label.
    pub fn with_universe(mut self, universe: impl Into<String>) -> Self {
        self.universe = Arc::new(universe.into());
        self
    }

    /// Attach a manifest cache directory. The bake-server will serve
    /// `<cache>/<z>-<x>-<y>.json` on hit; on miss returns 503 (or 202
    /// once a bake-queue producer is attached, see [`Self::with_producer`]).
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_dir = Some(Arc::new(dir));
        self
    }

    /// Q081: attach a bake-queue producer. On cache miss the handler
    /// will `XADD` a job to the cold lane and respond `202 Accepted +
    /// Retry-After`.
    pub fn with_producer(mut self, p: Producer, style_version: u32, osm_snapshot: String) -> Self {
        self.bake_producer = Some(Arc::new(p));
        self.style_version = style_version;
        self.osm_snapshot = Arc::new(osm_snapshot);
        self
    }
}

#[derive(Serialize)]
pub struct Health {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
}

/// Build the Axum router with the given app state. Pure builder so tests
/// can mount the same router under `axum-test`.
pub fn build_router(state: AppState) -> Router {
    // /metrics has its own state (the Metrics handle) so it can be scraped
    // without going through HMAC auth.
    let metrics_router = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(state.metrics.clone());

    use axum::routing::{delete, post};
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/tile/:z/:x/:y", get(get_tile))
        .route("/v1/tile/:z/:x/:y/bake", post(sync_bake_tile))
        .route("/v1/tile/:z/:x/:y/job/:bake_id", get(bake_job_status))
        .route("/v1/admin/cache/pin/:z/:x/:y", post(admin_pin))
        .route("/v1/admin/cache/pin/:z/:x/:y", delete(admin_unpin))
        .route("/v1/admin/cache/stats", get(admin_stats))
        .with_state(state)
        .merge(metrics_router)
}

pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        service: "bake-server",
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub async fn get_tile(
    Path((z, x, y)): Path<(u32, u32, u32)>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let started = std::time::Instant::now();
    let endpoint = "/v1/tile/:z/:x/:y";
    let path = format!("/v1/tile/{z}/{x}/{y}");
    if let Err(e) = verify_hmac(&headers, &s.hmac_key, &path) {
        s.metrics.observe_response(endpoint, e.0.as_u16());
        return Err(e);
    }

    // Q102: schema-version negotiation. `?schema_version=X.Y` pins the
    // client to a specific manifest schema. Retired versions return 410
    // immediately so old clients are forced to upgrade. Unknown garbage
    // versions fall back to the oldest supported schema (defensive parse,
    // per the SemVer policy in docs/manifest-schema-evolution.md).
    let requested_version = q.get("schema_version").map(|s| s.as_str());
    let negotiated = schema_version::negotiate(requested_version);
    if matches!(negotiated, schema_version::NegotiationOutcome::Retired) {
        s.metrics.observe_response(endpoint, 410);
        return Err((
            StatusCode::GONE,
            format!(
                "manifest schema version {} has been retired — please upgrade your client",
                requested_version.unwrap_or("?"),
            ),
        ));
    }

    // Q082: layered cache lookup (pinned + hot LRU + cold disk). When
    // attached, this path supersedes the raw `cache_dir` read so reads
    // benefit from in-process tier promotion. On miss we fall through
    // to the same 202/503 enqueue/refuse logic as before.
    if let Some(lc) = &s.layered_cache {
        match lc.get(TileKey::new(z, x, y)).await {
            Ok((Some(bytes), tier)) => {
                let elapsed = started.elapsed().as_secs_f64();
                let metric_tier = match tier {
                    CacheTier::Pinned => Tier::Pinned,
                    CacheTier::Hot => Tier::Hot,
                    CacheTier::Cold => Tier::Cold,
                    CacheTier::Miss => Tier::Cold, // unreachable here
                };
                let (bytes, wire_version) = apply_schema_projection(bytes, negotiated);
                s.metrics.observe_fetch(metric_tier, &s.universe, elapsed);
                s.metrics.observe_manifest_size(&s.universe, bytes.len());
                s.metrics.observe_response(endpoint, 200);
                let mut resp = (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    bytes,
                )
                    .into_response();
                resp.headers_mut().insert(
                    "x-wb-cache-tier",
                    tier.as_str().parse().expect("ascii tier"),
                );
                tag_manifest_version(&mut resp, wire_version);
                return Ok(resp);
            }
            Ok((None, _)) => {
                return miss_response(&s, z, x, y, endpoint, started).await;
            }
            Err(e) => {
                warn!(tile=%format!("{z}/{x}/{y}"), error=%e, "layered cache read failed");
                s.metrics.observe_response(endpoint, 500);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cache read failed: {e}"),
                ));
            }
        }
    }

    // Cache lookup path (Q465): when configured, serve the pre-baked
    // manifest bytes; on miss return 503 so the caller knows it isn't
    // ready (synchronous bake-on-miss is Q084).
    if let Some(dir) = &s.cache_dir {
        let file = dir.join(format!("{z}-{x}-{y}.json"));
        match tokio::fs::read(&file).await {
            Ok(bytes) => {
                let elapsed = started.elapsed().as_secs_f64();
                // Heuristic: pinned vs hot is currently indistinguishable
                // at the bake-server layer (the cache-eviction policy in
                // Q082 tracks pinning state). For now classify all
                // disk-cache hits as `hot`; Q082 will refine.
                let (bytes, wire_version) = apply_schema_projection(bytes, negotiated);
                s.metrics.observe_fetch(Tier::Hot, &s.universe, elapsed);
                s.metrics.observe_manifest_size(&s.universe, bytes.len());
                s.metrics.observe_response(endpoint, 200);
                let mut resp = (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    bytes,
                )
                    .into_response();
                tag_manifest_version(&mut resp, wire_version);
                return Ok(resp);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Q081: enqueue a bake job and return 202 + Retry-After
                // when a producer is configured. Without one, fall
                // back to the legacy 503 so existing deploys + tests
                // keep working.
                if let Some(prod) = &s.bake_producer {
                    let job = BakeJob::new(z, x, y, s.style_version, (*s.osm_snapshot).clone());
                    match prod.submit(&job).await {
                        Ok(Some(id)) => {
                            info!(tile=%job.tile_id, id=%id.as_str(), "cache miss → enqueued");
                        }
                        Ok(None) => {
                            info!(tile=%job.tile_id, "cache miss but idempotency-marker says done");
                        }
                        Err(e) => {
                            warn!(tile=%job.tile_id, error=%e, "enqueue failed");
                            s.metrics
                                .observe_bake_failure(&s.universe, "enqueue_failed");
                            s.metrics.observe_response(endpoint, 503);
                            return Err((
                                StatusCode::SERVICE_UNAVAILABLE,
                                format!("tile {z}/{x}/{y} enqueue failed"),
                            ));
                        }
                    }
                    // Cache miss → cold tier. Record fetch latency so the
                    // synchronous-202 path still feeds the SLO histogram.
                    let elapsed = started.elapsed().as_secs_f64();
                    s.metrics.observe_fetch(Tier::Cold, &s.universe, elapsed);
                    s.metrics.observe_response(endpoint, 202);
                    let body = json!({
                        "status": "accepted",
                        "tile_id": format!("{z}/{x}/{y}"),
                        "retry_after_secs": s.retry_after_secs,
                    });
                    let mut resp = (StatusCode::ACCEPTED, Json(body)).into_response();
                    resp.headers_mut().insert(
                        header::RETRY_AFTER,
                        s.retry_after_secs
                            .to_string()
                            .parse()
                            .expect("retry-after header"),
                    );
                    return Ok(resp);
                }
                s.metrics.observe_response(endpoint, 503);
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("tile {z}/{x}/{y} not baked yet"),
                ));
            }
            Err(e) => {
                s.metrics.observe_response(endpoint, 500);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cache read failed: {e}"),
                ));
            }
        }
    }

    // Fallback: legacy mock manifest used by Q475 HTTP tests. The
    // mock now declares v1.1 so the projection path through
    // `apply_schema_projection` is exercised end-to-end whenever a
    // client requests `?schema_version=1.0`.
    let manifest = json!({
        "manifest_version": schema_version::LATEST_VERSION,
        "tile_id": format!("{z}-{x}-{y}"),
        "z": z, "x": x, "y": y,
        "osm_snapshot": "2026-05-26T00:00:00Z",
        "style_version": "0.1.0",
        "terrain": { "grid_size": 0, "heights": [] },
        "buildings": [],
        "roads": [],
        "water": [],
        "landmarks": [],
        "note": "mock manifest — Q470 staging shape; real bake lands in Q082"
    });
    let bytes = serde_json::to_vec(&manifest).expect("mock manifest serialises");
    let (bytes, wire_version) = apply_schema_projection(bytes, negotiated);
    let elapsed = started.elapsed().as_secs_f64();
    s.metrics.observe_fetch(Tier::Pinned, &s.universe, elapsed);
    s.metrics.observe_response(endpoint, 200);
    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        bytes,
    )
        .into_response();
    tag_manifest_version(&mut resp, wire_version);
    Ok(resp)
}

/// Q102: apply schema-version projection to a raw manifest byte buffer.
/// Returns the (possibly rewritten) bytes plus the wire version the
/// client will see. If the manifest cannot be parsed as JSON, the bytes
/// pass through unchanged (so legacy mock payloads keep working) and
/// the wire version falls back to `LATEST_VERSION`.
fn apply_schema_projection(
    bytes: Vec<u8>,
    outcome: schema_version::NegotiationOutcome,
) -> (Vec<u8>, &'static str) {
    use schema_version::{NegotiationOutcome, LATEST_VERSION, SUPPORTED_VERSIONS};
    let target: Option<&'static str> = match outcome {
        NegotiationOutcome::Latest => None,
        NegotiationOutcome::Project(v) => Some(v),
        // Defensive parse: unknown version → oldest supported.
        NegotiationOutcome::UnknownFallbackOldest => Some(SUPPORTED_VERSIONS[0]),
        // Retired is handled before this fn is called.
        NegotiationOutcome::Retired => return (bytes, LATEST_VERSION),
    };
    let Some(target) = target else {
        return (bytes, LATEST_VERSION);
    };
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (bytes, LATEST_VERSION);
    };
    schema_version::project_down(&mut v, target);
    let out = serde_json::to_vec(&v).unwrap_or(bytes);
    (out, target)
}

/// Wire the `X-Manifest-Version` response header onto a built response.
fn tag_manifest_version(resp: &mut axum::response::Response, version: &str) {
    if let Ok(v) = version.parse() {
        resp.headers_mut().insert("x-manifest-version", v);
    }
}

/// Shared cache-miss handling: enqueue (when a producer is wired) and
/// 202, otherwise 503. Used by both the layered-cache and the legacy
/// `cache_dir` lookup branches so they stay byte-compatible.
async fn miss_response(
    s: &AppState,
    z: u32,
    x: u32,
    y: u32,
    endpoint: &'static str,
    started: std::time::Instant,
) -> Result<axum::response::Response, (StatusCode, String)> {
    if let Some(prod) = &s.bake_producer {
        let job = BakeJob::new(z, x, y, s.style_version, (*s.osm_snapshot).clone());
        match prod.submit(&job).await {
            Ok(Some(id)) => info!(tile=%job.tile_id, id=%id.as_str(), "cache miss → enqueued"),
            Ok(None) => info!(tile=%job.tile_id, "cache miss but idempotency-marker says done"),
            Err(e) => {
                warn!(tile=%job.tile_id, error=%e, "enqueue failed");
                s.metrics
                    .observe_bake_failure(&s.universe, "enqueue_failed");
                s.metrics.observe_response(endpoint, 503);
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("tile {z}/{x}/{y} enqueue failed"),
                ));
            }
        }
        let elapsed = started.elapsed().as_secs_f64();
        s.metrics.observe_fetch(Tier::Cold, &s.universe, elapsed);
        s.metrics.observe_response(endpoint, 202);
        let body = json!({
            "status": "accepted",
            "tile_id": format!("{z}/{x}/{y}"),
            "retry_after_secs": s.retry_after_secs,
        });
        let mut resp = (StatusCode::ACCEPTED, Json(body)).into_response();
        resp.headers_mut().insert(
            header::RETRY_AFTER,
            s.retry_after_secs
                .to_string()
                .parse()
                .expect("retry-after header"),
        );
        return Ok(resp);
    }
    s.metrics.observe_response(endpoint, 503);
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        format!("tile {z}/{x}/{y} not baked yet"),
    ))
}

// ---------------------------------------------------------------------------
// Q082 admin endpoints — pin / unpin / stats. Auth via the same HMAC
// scheme as `GET /v1/tile/*`. Restricted to ops because pinning is a
// production-safety lever (occupies HOT capacity).
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct PinPath {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

pub async fn admin_pin(
    Path((z, x, y)): Path<(u32, u32, u32)>,
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let path = format!("/v1/admin/cache/pin/{z}/{x}/{y}");
    verify_hmac(&headers, &s.hmac_key, &path)?;
    let cache = s.layered_cache.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "layered cache not configured".into(),
    ))?;
    let key = TileKey::new(z, x, y);
    let created = cache.pin(key).await.map_err(|e| match e {
        cache::CacheError::PinCapacity { .. } => (StatusCode::CONFLICT, e.to_string()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    })?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let body = json!({
        "tile_id": format!("{z}/{x}/{y}"),
        "pinned": true,
        "newly_created": created,
    });
    Ok((status, Json(body)).into_response())
}

pub async fn admin_unpin(
    Path((z, x, y)): Path<(u32, u32, u32)>,
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let path = format!("/v1/admin/cache/pin/{z}/{x}/{y}");
    verify_hmac(&headers, &s.hmac_key, &path)?;
    let cache = s.layered_cache.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "layered cache not configured".into(),
    ))?;
    let key = TileKey::new(z, x, y);
    let removed = cache
        .unpin(key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !removed {
        return Err((StatusCode::NOT_FOUND, format!("{z}/{x}/{y} not pinned")));
    }
    Ok(Json(json!({"tile_id": format!("{z}/{x}/{y}"), "pinned": false})).into_response())
}

pub async fn admin_stats(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let path = "/v1/admin/cache/stats";
    verify_hmac(&headers, &s.hmac_key, path)?;
    let cache = s.layered_cache.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "layered cache not configured".into(),
    ))?;
    let stats = cache.stats().await;
    let pins = cache.list_pins().await;
    Ok(Json(json!({"stats": stats, "pins": pins})).into_response())
}

// ---------------------------------------------------------------------------
// Q084 synchronous-bake endpoints.
// ---------------------------------------------------------------------------

/// `POST /v1/tile/:z/:x/:y/bake` — HMAC-signed. Returns:
/// * `200 OK` + manifest bytes when the cache already has the tile
///   OR the synchronous bake finishes within the soft deadline (8s).
/// * `202 Accepted` + placeholder manifest + `bake_id` when the bake
///   exceeds the soft deadline. Client polls `/job/:bake_id` or
///   subscribes via SSE.
/// * `503 Service Unavailable` when no executor is wired (server
///   misconfiguration).
pub async fn sync_bake_tile(
    Path((z, x, y)): Path<(u32, u32, u32)>,
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let endpoint = "/v1/tile/:z/:x/:y/bake";
    let path = format!("/v1/tile/{z}/{x}/{y}/bake");
    if let Err(e) = verify_hmac(&headers, &s.hmac_key, &path) {
        s.metrics.observe_response(endpoint, e.0.as_u16());
        return Err(e);
    }

    let key = TileKey::new(z, x, y);

    // Cache fast-path — never bake when the manifest is already there.
    if let Some(lc) = &s.layered_cache {
        if let Ok((Some(bytes), tier)) = lc.get(key).await {
            let metric_tier = match tier {
                CacheTier::Pinned => Tier::Pinned,
                CacheTier::Hot => Tier::Hot,
                CacheTier::Cold => Tier::Cold,
                CacheTier::Miss => Tier::Cold,
            };
            s.metrics.observe_fetch(metric_tier, &s.universe, 0.0);
            s.metrics.observe_manifest_size(&s.universe, bytes.len());
            s.metrics.observe_response(endpoint, 200);
            let mut resp = (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                bytes,
            )
                .into_response();
            resp.headers_mut().insert(
                "x-wb-cache-tier",
                tier.as_str().parse().expect("ascii tier"),
            );
            return Ok(resp);
        }
    }

    let Some(executor) = s.bake_executor.clone() else {
        s.metrics.observe_response(endpoint, 503);
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "sync-bake executor not configured".into(),
        ));
    };

    let outcome = run_sync_bake(
        key,
        executor,
        s.job_registry.clone(),
        s.layered_cache.clone(),
        &s.metrics,
        &s.universe,
        s.bake_soft_deadline,
    )
    .await;

    match outcome {
        BakeOutcome::Ready { manifest } => {
            s.metrics.observe_response(endpoint, 200);
            s.metrics.observe_manifest_size(&s.universe, manifest.len());
            // Write-through: if a layered cache is attached, persist.
            if let Some(lc) = &s.layered_cache {
                let _ = lc.put(key, manifest.clone()).await;
            }
            Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                manifest,
            )
                .into_response())
        }
        BakeOutcome::Pending {
            bake_id,
            placeholder,
        } => {
            s.metrics.observe_response(endpoint, 202);
            let body = json!({
                "status": "accepted",
                "tile_id": format!("{z}/{x}/{y}"),
                "bake_id": bake_id,
                "retry_after_secs": s.retry_after_secs,
                "placeholder": placeholder,
            });
            let mut resp = (StatusCode::ACCEPTED, Json(body)).into_response();
            resp.headers_mut().insert(
                header::RETRY_AFTER,
                s.retry_after_secs.to_string().parse().expect("retry-after"),
            );
            Ok(resp)
        }
    }
}

/// `GET /v1/tile/:z/:x/:y/job/:bake_id` — HMAC-signed.
///
/// Without `?stream=sse`: returns JSON `{state, tile_id, bake_id, ...}`.
/// When `state == done`, the response also includes the manifest bytes
/// (base64) so the client can finish in one round-trip without re-fetching
/// the tile endpoint.
///
/// With `?stream=sse`: returns `text/event-stream` and emits an event
/// when the job transitions to a terminal state, then closes.
pub async fn bake_job_status(
    Path((z, x, y, bake_id)): Path<(u32, u32, u32, String)>,
    State(s): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let endpoint = "/v1/tile/:z/:x/:y/job/:bake_id";
    let path = format!("/v1/tile/{z}/{x}/{y}/job/{bake_id}");
    if let Err(e) = verify_hmac(&headers, &s.hmac_key, &path) {
        s.metrics.observe_response(endpoint, e.0.as_u16());
        return Err(e);
    }

    let Some(status) = s.job_registry.get(&bake_id).await else {
        s.metrics.observe_response(endpoint, 404);
        return Err((StatusCode::NOT_FOUND, format!("bake_id {bake_id} unknown")));
    };

    if q.get("stream").map(|v| v.as_str()) == Some("sse") {
        // Server-Sent Events — wait for the next terminal transition (or
        // emit immediately if already terminal) and close.
        let mut rx = s.job_registry.subscribe(&bake_id).await.ok_or((
            StatusCode::GONE,
            format!("bake_id {bake_id} no longer subscribable"),
        ))?;
        let registry = s.job_registry.clone();
        let bake_id_owned = bake_id.clone();
        let stream = async_stream::stream! {
            // Emit current state immediately.
            if let Some(st) = registry.get(&bake_id_owned).await {
                let payload = serde_json::to_string(&st).unwrap_or_default();
                yield Ok::<_, std::convert::Infallible>(format!("event: status\ndata: {payload}\n\n"));
                if !matches!(st.state, JobState::Pending) {
                    return;
                }
            }
            // Wait for a terminal transition.
            while rx.changed().await.is_ok() {
                let state = rx.borrow().clone();
                if let Some(st) = registry.get(&bake_id_owned).await {
                    let payload = serde_json::to_string(&st).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(format!("event: status\ndata: {payload}\n\n"));
                }
                if !matches!(state, JobState::Pending) {
                    return;
                }
            }
        };
        let body = axum::body::Body::from_stream(stream);
        let resp = axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body)
            .expect("sse response");
        s.metrics.observe_response(endpoint, 200);
        return Ok(resp);
    }

    s.metrics.observe_response(endpoint, 200);
    let mut body = serde_json::to_value(&status).unwrap_or(json!({}));
    if let Some(m) = &status.manifest {
        use base64::Engine as _;
        body.as_object_mut().expect("object").insert(
            "manifest_b64".into(),
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(m)),
        );
    }
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// HMAC-SHA256 over `<ts>\n<path>`, hex-encoded in `x-wb-sig`.
pub fn sign_payload(key: &[u8], ts: u64, path: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(format!("{ts}\n{path}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_hmac(
    headers: &HeaderMap,
    key: &[u8],
    path: &str,
) -> Result<(), (StatusCode, String)> {
    let ts = headers
        .get("x-wb-ts")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing x-wb-ts".into()))?;
    let sig = headers
        .get("x-wb-sig")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing x-wb-sig".into()))?;

    let ts_n: u64 = ts
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad ts".into()))?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "clock".into()))?
        .as_secs();
    if now.abs_diff(ts_n) > HMAC_TS_SKEW_SECS {
        warn!(ts, now, "ts skew");
        return Err((StatusCode::UNAUTHORIZED, "ts skew".into()));
    }

    let expected = sign_payload(key, ts_n, path);
    if !ct_eq(expected.as_bytes(), sig.as_bytes()) {
        warn!(path, "bad sig");
        return Err((StatusCode::UNAUTHORIZED, "bad sig".into()));
    }
    Ok(())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Current wall-clock seconds since UNIX epoch. Helper for tests that need
/// to sign with a fresh timestamp.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn sign_payload_is_deterministic() {
        let key = [7u8; 32];
        let a = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        let b = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // hex sha256
    }

    #[test]
    fn sign_payload_changes_with_path() {
        let key = [7u8; 32];
        let a = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        let b = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/2");
        assert_ne!(a, b);
    }

    #[test]
    fn sign_payload_changes_with_ts() {
        let key = [7u8; 32];
        let a = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        let b = sign_payload(&key, 1_700_000_001, "/v1/tile/15/1/1");
        assert_ne!(a, b);
    }
}
