//! Q084: Cold-tile on-demand bake SLA.
//!
//! Implements the synchronous-fast-path + placeholder-then-poll protocol
//! described in `docs/grill/q084-cold-tile-on-demand-bake-sla.md`.
//!
//! The flow:
//!
//! ```text
//!   POST /v1/tile/{z}/{x}/{y}/bake           (HMAC-signed)
//!         │
//!         ├── cache hit                       → 200 + manifest
//!         ├── bake finishes within 8 s        → 200 + manifest
//!         └── deadline exceeded               → 202 + placeholder + bake_id
//!
//!   GET  /v1/tile/{z}/{x}/{y}/job/{bake_id}   (HMAC-signed)
//!         ├── ?stream=sse                     → text/event-stream push
//!         └── default                         → 200 JSON status
//! ```
//!
//! The placeholder manifest carries `placeholder: true` and, when a
//! cached parent tile (z-1) is available, embeds the parent's manifest
//! bytes under `parent_manifest` so the client can paint a coarse
//! low-resolution view while the bake completes.
//!
//! ## Boundaries with sister grills
//!
//! - **Q081** (queue): the production [`BakeExecutor`] dispatches via
//!   the bake-queue producer + waits on a oneshot fed by the worker.
//!   This module never writes to Redis directly.
//! - **Q082** (cache): cache lookups + placeholder-parent reads go
//!   through `LayeredCache`. We do not duplicate cache state.
//! - **Q097** (metrics): SLA breaches feed `wb_bake_sla_breach_total`
//!   on the shared `Metrics` registry — no separate exporter.

use crate::cache::{LayeredCache, TileCache, TileKey};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex};
use ulid::Ulid;

/// Soft deadline for the synchronous bake. If the bake completes within
/// this window, the handler returns 200 + manifest. If it does not, the
/// handler returns 202 + placeholder.
pub const SYNC_BAKE_SOFT_DEADLINE: Duration = Duration::from_secs(8);

/// Hard wall-clock ceiling — even if the worker is still going, we
/// release the HTTP connection at this point. Q084 grill, "timeout
/// budget" table.
pub const SYNC_BAKE_HARD_DEADLINE: Duration = Duration::from_secs(12);

/// Outcome of a synchronous-bake attempt. The HTTP layer maps this
/// directly onto a response.
#[derive(Debug, Clone)]
pub enum BakeOutcome {
    /// Bake finished within the soft deadline.
    Ready { manifest: Vec<u8> },
    /// Soft deadline exceeded. The placeholder manifest is returned
    /// immediately; `bake_id` is the handle the client polls / SSE-subscribes to.
    Pending { bake_id: String, placeholder: Value },
}

/// Job lifecycle states surfaced via `GET /v1/tile/.../job/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Pending,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobStatus {
    pub bake_id: String,
    pub tile_id: String,
    pub state: JobState,
    /// When `state == done`, the manifest bytes are inlined (base64-decoded
    /// at the read site — we keep raw bytes here to avoid a round-trip).
    #[serde(skip)]
    pub manifest: Option<Vec<u8>>,
    pub error: Option<String>,
    pub created_at_ms: u128,
}

/// Trait abstracting "perform a bake for this tile". In production this
/// pushes a `BakeJob` onto the queue and awaits the worker's oneshot.
/// In tests it returns canned bytes / delays / failures.
#[async_trait]
pub trait BakeExecutor: Send + Sync {
    /// Execute a bake. Implementations should respect cancellation —
    /// the caller wraps this in `tokio::time::timeout(...)`. The worker
    /// itself keeps running on timeout (idempotent; the manifest will
    /// land in the cache and a subsequent poll will pick it up).
    async fn bake(&self, key: TileKey) -> anyhow::Result<Vec<u8>>;
}

/// In-memory registry of in-flight + recently-completed bakes. Keyed by
/// the ULID `bake_id` we hand to the client.
///
/// Entries are retained for `RETENTION` after completion so polling
/// clients can still observe the terminal state. A background sweeper
/// could prune older entries; for now the volume is tiny (one per
/// SLA-breaching cold miss) and the cap is enforced by a soft FIFO trim.
pub struct JobRegistry {
    inner: Mutex<HashMap<String, JobEntry>>,
    /// Notifies waiters of any registry change. Each job also has its
    /// own per-id watch channel for SSE/poll fan-out; the global signal
    /// keeps the registry test-friendly.
    bell: watch::Sender<u64>,
    cap: usize,
}

struct JobEntry {
    status: JobStatus,
    notifier: watch::Sender<JobState>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl JobRegistry {
    pub fn new(cap: usize) -> Self {
        let (bell, _) = watch::channel(0);
        Self {
            inner: Mutex::new(HashMap::new()),
            bell,
            cap,
        }
    }

    pub async fn create(&self, key: TileKey) -> (String, watch::Receiver<JobState>) {
        let bake_id = Ulid::new().to_string();
        let (tx, rx) = watch::channel(JobState::Pending);
        let entry = JobEntry {
            status: JobStatus {
                bake_id: bake_id.clone(),
                tile_id: key.redis_member(),
                state: JobState::Pending,
                manifest: None,
                error: None,
                created_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
            },
            notifier: tx,
        };
        let mut g = self.inner.lock().await;
        // Soft FIFO trim — drop oldest if over cap.
        if g.len() >= self.cap {
            if let Some(oldest) = g
                .values()
                .min_by_key(|e| e.status.created_at_ms)
                .map(|e| e.status.bake_id.clone())
            {
                g.remove(&oldest);
            }
        }
        g.insert(bake_id.clone(), entry);
        let _ = self.bell.send(self.bell.borrow().wrapping_add(1));
        (bake_id, rx)
    }

    pub async fn finish(&self, bake_id: &str, manifest: Vec<u8>) {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(bake_id) {
            entry.status.state = JobState::Done;
            entry.status.manifest = Some(manifest);
            let _ = entry.notifier.send(JobState::Done);
        }
        let _ = self.bell.send(self.bell.borrow().wrapping_add(1));
    }

    pub async fn fail(&self, bake_id: &str, error: String) {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(bake_id) {
            entry.status.state = JobState::Failed;
            entry.status.error = Some(error);
            let _ = entry.notifier.send(JobState::Failed);
        }
        let _ = self.bell.send(self.bell.borrow().wrapping_add(1));
    }

    pub async fn get(&self, bake_id: &str) -> Option<JobStatus> {
        self.inner
            .lock()
            .await
            .get(bake_id)
            .map(|e| e.status.clone())
    }

    pub async fn subscribe(&self, bake_id: &str) -> Option<watch::Receiver<JobState>> {
        self.inner
            .lock()
            .await
            .get(bake_id)
            .map(|e| e.notifier.subscribe())
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }
}

/// Build the placeholder manifest returned alongside `202 Accepted`
/// when the synchronous bake exceeds [`SYNC_BAKE_SOFT_DEADLINE`].
///
/// If a parent tile (z-1) is in the layered cache, its manifest bytes
/// are embedded under `parent_manifest` so the client can paint a
/// coarse low-resolution view immediately. Q084 grill: "the cached
/// low-resolution version if available (e.g., a z=14 parent tile)".
pub async fn build_placeholder(
    key: TileKey,
    bake_id: &str,
    cache: Option<&Arc<LayeredCache>>,
) -> Value {
    let mut placeholder = serde_json::json!({
        "version": "manifest/2.0",
        "tile_id": key.redis_member(),
        "bake_id": bake_id,
        "status": "placeholder",
        "placeholder": true,
        "buildings_lod": "outlines_only",
    });
    if key.z > 0 {
        let parent_key = TileKey::new(key.z - 1, key.x / 2, key.y / 2);
        if let Some(lc) = cache {
            if let Ok((Some(bytes), _)) = lc.get(parent_key).await {
                if let Ok(parent_json) = serde_json::from_slice::<Value>(&bytes) {
                    placeholder
                        .as_object_mut()
                        .expect("object")
                        .insert("parent_manifest".to_string(), parent_json);
                    placeholder.as_object_mut().expect("object").insert(
                        "parent_tile_id".to_string(),
                        Value::String(parent_key.redis_member()),
                    );
                }
            }
        }
    }
    placeholder
}

/// Run a synchronous-bake attempt with the Q084 deadline budget.
///
/// Returns:
/// - `BakeOutcome::Ready` if the bake finished within
///   [`SYNC_BAKE_SOFT_DEADLINE`].
/// - `BakeOutcome::Pending` if the deadline expired. The bake
///   continues in the background — when the executor returns, the
///   manifest is recorded in the [`JobRegistry`] under `bake_id`.
///
/// Increments `wb_bake_sla_breach_total` on the supplied metrics
/// handle when the soft deadline is exceeded.
pub async fn run_sync_bake(
    key: TileKey,
    executor: Arc<dyn BakeExecutor>,
    registry: Arc<JobRegistry>,
    cache: Option<Arc<LayeredCache>>,
    metrics: &crate::metrics::Metrics,
    universe: &str,
    soft_deadline: Duration,
) -> BakeOutcome {
    let started = Instant::now();
    let (bake_id, _rx) = registry.create(key).await;
    let exec_clone = executor.clone();

    // Spawn the bake on a background task so we can race it against
    // the deadline without losing the work on timeout.
    let task_registry = registry.clone();
    let task_bake_id = bake_id.clone();
    let handle = tokio::spawn(async move {
        match exec_clone.bake(key).await {
            Ok(bytes) => {
                task_registry.finish(&task_bake_id, bytes.clone()).await;
                Ok(bytes)
            }
            Err(e) => {
                task_registry.fail(&task_bake_id, e.to_string()).await;
                Err(e)
            }
        }
    });

    match tokio::time::timeout(soft_deadline, handle).await {
        Ok(Ok(Ok(manifest))) => {
            let elapsed = started.elapsed().as_secs_f64();
            metrics.observe_bake(universe, elapsed, manifest.len());
            BakeOutcome::Ready { manifest }
        }
        Ok(Ok(Err(e))) => {
            // Bake failed within deadline — surface as placeholder with
            // an error breadcrumb. Clients can retry.
            metrics.observe_bake_failure(universe, "bake_error");
            let mut placeholder = build_placeholder(key, &bake_id, cache.as_ref()).await;
            placeholder
                .as_object_mut()
                .expect("object")
                .insert("error".into(), Value::String(e.to_string()));
            BakeOutcome::Pending {
                bake_id,
                placeholder,
            }
        }
        Ok(Err(_join_err)) => {
            metrics.observe_bake_failure(universe, "join_error");
            BakeOutcome::Pending {
                bake_id: bake_id.clone(),
                placeholder: build_placeholder(key, &bake_id, cache.as_ref()).await,
            }
        }
        Err(_timeout) => {
            // Soft-deadline breach — bake keeps running in background.
            metrics.observe_sla_breach(universe);
            BakeOutcome::Pending {
                bake_id: bake_id.clone(),
                placeholder: build_placeholder(key, &bake_id, cache.as_ref()).await,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// In-process [`BakeExecutor`] used by tests. Returns a configurable
/// payload after a configurable delay; optionally fails.
pub struct ScriptedExecutor {
    pub delay: Duration,
    pub result: ScriptedResult,
}

#[derive(Clone)]
pub enum ScriptedResult {
    Ok(Vec<u8>),
    Err(String),
}

impl ScriptedExecutor {
    pub fn ok(delay: Duration, bytes: Vec<u8>) -> Self {
        Self {
            delay,
            result: ScriptedResult::Ok(bytes),
        }
    }
    pub fn fail(delay: Duration, msg: impl Into<String>) -> Self {
        Self {
            delay,
            result: ScriptedResult::Err(msg.into()),
        }
    }
}

#[async_trait]
impl BakeExecutor for ScriptedExecutor {
    async fn bake(&self, _key: TileKey) -> anyhow::Result<Vec<u8>> {
        tokio::time::sleep(self.delay).await;
        match &self.result {
            ScriptedResult::Ok(b) => Ok(b.clone()),
            ScriptedResult::Err(m) => Err(anyhow::anyhow!(m.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;

    fn tk() -> TileKey {
        TileKey::new(15, 17128, 9656)
    }

    #[tokio::test]
    async fn sync_bake_under_deadline_returns_ready() {
        let exec = Arc::new(ScriptedExecutor::ok(
            Duration::from_millis(20),
            b"manifest-bytes".to_vec(),
        ));
        let registry = Arc::new(JobRegistry::default());
        let m = Metrics::new();
        let outcome = run_sync_bake(
            tk(),
            exec,
            registry,
            None,
            &m,
            "demo",
            SYNC_BAKE_SOFT_DEADLINE,
        )
        .await;
        match outcome {
            BakeOutcome::Ready { manifest } => assert_eq!(manifest, b"manifest-bytes"),
            BakeOutcome::Pending { .. } => panic!("expected Ready"),
        }
        let body = String::from_utf8(m.render()).unwrap();
        assert!(body.contains("wb_bake_duration_seconds"));
        assert!(!body.contains("wb_bake_sla_breach_total 1"));
    }

    #[tokio::test]
    async fn sync_bake_over_deadline_returns_pending_and_increments_breach() {
        let exec = Arc::new(ScriptedExecutor::ok(
            Duration::from_millis(300),
            b"slow-manifest".to_vec(),
        ));
        let registry = Arc::new(JobRegistry::default());
        let m = Metrics::new();
        let outcome = run_sync_bake(
            tk(),
            exec,
            registry.clone(),
            None,
            &m,
            "demo",
            Duration::from_millis(50),
        )
        .await;
        let bake_id = match outcome {
            BakeOutcome::Pending {
                bake_id,
                placeholder,
            } => {
                assert_eq!(placeholder["placeholder"], true);
                assert_eq!(placeholder["status"], "placeholder");
                bake_id
            }
            BakeOutcome::Ready { .. } => panic!("expected Pending"),
        };
        let body = String::from_utf8(m.render()).unwrap();
        assert!(
            body.contains("wb_bake_sla_breach_total"),
            "breach counter must be emitted"
        );
        // Worker eventually completes — poll the registry.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let status = registry.get(&bake_id).await.expect("status");
        assert_eq!(status.state, JobState::Done);
        assert_eq!(status.manifest.as_deref(), Some(&b"slow-manifest"[..]));
    }

    #[tokio::test]
    async fn placeholder_embeds_parent_manifest_when_available() {
        use crate::cache::TileCacheWriter;
        let tmp = tempfile::tempdir().unwrap();
        let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
        let parent_bytes = serde_json::to_vec(&serde_json::json!({
            "manifest_version": "1.0",
            "note": "parent",
        }))
        .unwrap();
        cache
            .put(TileKey::new(14, 8564, 4828), parent_bytes)
            .await
            .unwrap();
        let placeholder = build_placeholder(tk(), "01HXYZ", Some(&cache)).await;
        assert_eq!(placeholder["parent_tile_id"], "14/8564/4828");
        assert_eq!(placeholder["parent_manifest"]["note"], "parent");
    }

    #[tokio::test]
    async fn placeholder_without_parent_omits_parent_manifest() {
        let cache = Arc::new(LayeredCache::memory_only(4, 4));
        let placeholder = build_placeholder(tk(), "01HXYZ", Some(&cache)).await;
        assert!(placeholder.get("parent_manifest").is_none());
        assert_eq!(placeholder["placeholder"], true);
    }

    #[tokio::test]
    async fn job_registry_lifecycle_pending_done() {
        let r = JobRegistry::default();
        let (id, mut rx) = r.create(tk()).await;
        assert_eq!(r.get(&id).await.unwrap().state, JobState::Pending);
        r.finish(&id, b"done".to_vec()).await;
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), JobState::Done);
        assert_eq!(r.get(&id).await.unwrap().state, JobState::Done);
    }

    #[tokio::test]
    async fn job_registry_failure_path_records_error() {
        let r = JobRegistry::default();
        let (id, _rx) = r.create(tk()).await;
        r.fail(&id, "bake panic".into()).await;
        let s = r.get(&id).await.unwrap();
        assert_eq!(s.state, JobState::Failed);
        assert_eq!(s.error.as_deref(), Some("bake panic"));
    }

    #[tokio::test]
    async fn job_registry_capacity_evicts_oldest() {
        let r = JobRegistry::new(2);
        let (a, _) = r.create(TileKey::new(15, 0, 0)).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        let (_b, _) = r.create(TileKey::new(15, 0, 1)).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        let (_c, _) = r.create(TileKey::new(15, 0, 2)).await;
        assert_eq!(r.len().await, 2);
        assert!(r.get(&a).await.is_none(), "oldest should be evicted");
    }

    #[tokio::test]
    async fn bake_failure_within_deadline_returns_pending_with_error() {
        let exec = Arc::new(ScriptedExecutor::fail(
            Duration::from_millis(10),
            "OOM in worker",
        ));
        let registry = Arc::new(JobRegistry::default());
        let m = Metrics::new();
        let outcome = run_sync_bake(
            tk(),
            exec,
            registry,
            None,
            &m,
            "demo",
            SYNC_BAKE_SOFT_DEADLINE,
        )
        .await;
        match outcome {
            BakeOutcome::Pending { placeholder, .. } => {
                assert_eq!(placeholder["error"], "OOM in worker");
            }
            _ => panic!("expected Pending with error"),
        }
        let body = String::from_utf8(m.render()).unwrap();
        assert!(body.contains("wb_bake_failures_total"));
    }

    /// Ensure timeout const enforces the SLA from the grill doc.
    #[test]
    fn sla_constants_match_grill() {
        assert_eq!(SYNC_BAKE_SOFT_DEADLINE.as_secs(), 8);
        assert_eq!(SYNC_BAKE_HARD_DEADLINE.as_secs(), 12);
    }
}
