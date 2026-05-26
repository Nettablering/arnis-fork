//! Layered tile-manifest cache (Q082).
//!
//! Three tiers with two backends:
//!
//! ```text
//!  PINNED   in-memory  never evict   (ops-promoted; mirrored to Redis
//!                                      sorted-set `wb:cache.hot.tiles`
//!                                      so worker preheat — Q083 — can
//!                                      walk the same registry)
//!  HOT      in-memory  LRU 10K       (warm working set; promoted on
//!                                      repeat hit from cold)
//!  COLD     disk       30-day TTL    (existing on-disk manifest cache;
//!                                      backwards compatible with the
//!                                      Q465 `cache_dir` layout)
//! ```
//!
//! The previous `AppState::with_cache_dir(dir)` keeps working: it now
//! constructs a [`LayeredCache`] whose HOT tier is empty (size-1 LRU)
//! and whose COLD tier is the same on-disk directory, with no Redis
//! hot-key registry attached. The HTTP cache-lookup path becomes
//! tier-aware while old test fixtures continue to round-trip through
//! the new code unchanged.
//!
//! ## Contract with Q083 (hot-tile preheating)
//!
//! When a Redis backend is wired in, [`LayeredCache`] mirrors every pin
//! into the sorted set
//!
//! ```text
//!   wb:cache.hot.tiles   ZADD <unix-ts> <tile-id>
//! ```
//!
//! and removes the member on unpin (`ZREM`). The preheat cron in Q083
//! reads this set (`ZRANGEBYSCORE -inf +inf`) and enqueues each member
//! into `wb:bake.requests.hot` so the manifest stays warm. This module
//! never writes to the bake stream itself — that boundary belongs to
//! Q083.
//!
//! ## Cache-key shape
//!
//! Internal lookup uses the slippy `(z, x, y)` triplet so the existing
//! `<z>-<x>-<y>.json` on-disk layout from Q465 stays compatible. The
//! `tile:<tier>:{z}:{x}:{y}:v{style}:{snap}` Redis-key shape from the
//! Q082 grill spec is a separate concern (manifest storage in Redis is
//! tracked in the follow-up; this module covers the in-process tier +
//! disk + pin registry that the bake-server actually reaches for on
//! every request today).

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Default cap for the HOT in-memory LRU. Matches the per-tile budget
/// outlined in the Q082 grill (10k tiles ≈ working set hot path).
pub const DEFAULT_HOT_CAPACITY: usize = 10_000;

/// Default cap for PINNED tiles. Matches `wb_hot_tiles` cap of 2000.
pub const DEFAULT_PIN_CAPACITY: usize = 2_000;

/// Default cold-tier TTL (30 days) — manifests older than this are
/// treated as a miss even if the file exists. Re-bake-on-miss is cheap
/// (Q084) so the staleness sweep is conservative.
pub const DEFAULT_COLD_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Redis sorted-set key tracking PINNED tile-ids. Q083 reads this.
pub const REDIS_HOT_TILES_KEY: &str = "wb:cache.hot.tiles";

/// Which tier a lookup hit was served from. Named [`CacheTier`] (not
/// just `Tier`) so we don't collide with the [`crate::metrics::Tier`]
/// enum used for Prometheus labelling — that one has three variants
/// (no `Miss`) and is the wire-format for the `X-WB-Cache-Tier` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    Pinned,
    Hot,
    Cold,
    Miss,
}

impl CacheTier {
    pub fn as_str(self) -> &'static str {
        match self {
            CacheTier::Pinned => "pinned",
            CacheTier::Hot => "hot",
            CacheTier::Cold => "cold",
            CacheTier::Miss => "miss",
        }
    }
}

/// Slippy-map tile id `(z, x, y)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub fn new(z: u32, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    /// Canonical string form used in the Redis hot-tiles sorted set:
    /// `"{z}/{x}/{y}"` — same shape as `BakeJob.tile_id`.
    pub fn redis_member(&self) -> String {
        format!("{}/{}/{}", self.z, self.x, self.y)
    }

    /// On-disk filename used by the Q465 cold-tier layout.
    pub fn disk_filename(&self) -> String {
        format!("{}-{}-{}.json", self.z, self.x, self.y)
    }
}

/// Read-side trait — `bake-server`'s tile handler depends on this only.
#[async_trait]
pub trait TileCache: Send + Sync {
    /// Look the tile up across all tiers. Returns the manifest bytes
    /// plus the tier it was served from. On miss returns
    /// `Ok((None, CacheTier::Miss))` (i.e. a miss is not an error — the
    /// handler turns it into 202/503 itself).
    async fn get(&self, key: TileKey) -> CacheResult<(Option<Vec<u8>>, CacheTier)>;

    /// Snapshot of counters used by `GET /v1/admin/cache/stats`.
    async fn stats(&self) -> CacheStats;

    /// List currently pinned tiles. Used by stats + ops debugging.
    async fn list_pins(&self) -> Vec<String>;

    /// Pin a tile: promotes to PINNED tier and mirrors into the Redis
    /// hot-key registry (when a backend is attached). Returns whether
    /// the pin was newly created (`true`) or already existed (`false`).
    async fn pin(&self, key: TileKey) -> CacheResult<bool>;

    /// Unpin a tile: demotes from PINNED, leaving its manifest (if any)
    /// in HOT so it can age out under LRU pressure. Removes the entry
    /// from the Redis hot-key registry. Returns whether the pin was
    /// removed (`true`) or didn't exist (`false`).
    async fn unpin(&self, key: TileKey) -> CacheResult<bool>;
}

/// Write-side trait used by the bake worker / pin reconciler to insert
/// freshly baked manifests into the appropriate tier. Lives separate
/// from [`TileCache`] so HTTP handlers can take `Arc<dyn TileCache>`
/// without holding write capability.
#[async_trait]
pub trait TileCacheWriter: Send + Sync {
    /// Insert / refresh a manifest. The cache picks the tier:
    /// pinned-registered tiles always land in PINNED; everything else
    /// goes to HOT. The cold-tier disk copy is written unconditionally
    /// (durable backup; survives restart).
    async fn put(&self, key: TileKey, bytes: Vec<u8>) -> CacheResult<CacheTier>;
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("redis: {0}")]
    Redis(String),
    #[error("pin capacity exceeded ({cap}); refuse to add {tile}")]
    PinCapacity { cap: usize, tile: String },
}

pub type CacheResult<T> = std::result::Result<T, CacheError>;

/// Hot-tile registry — abstracted so unit tests don't need redis-server.
/// In production this is backed by Redis `ZADD/ZREM/ZRANGE` against
/// [`REDIS_HOT_TILES_KEY`].
#[async_trait]
pub trait HotRegistry: Send + Sync {
    async fn add(&self, tile_id: &str, score: f64) -> CacheResult<()>;
    async fn remove(&self, tile_id: &str) -> CacheResult<()>;
    async fn list(&self) -> CacheResult<Vec<String>>;
}

/// In-process fake used by tests + by `with_cache_dir` (no Redis).
#[derive(Default)]
pub struct InMemoryHotRegistry {
    inner: Mutex<HashMap<String, f64>>,
}

#[async_trait]
impl HotRegistry for InMemoryHotRegistry {
    async fn add(&self, tile_id: &str, score: f64) -> CacheResult<()> {
        self.inner.lock().await.insert(tile_id.to_string(), score);
        Ok(())
    }
    async fn remove(&self, tile_id: &str) -> CacheResult<()> {
        self.inner.lock().await.remove(tile_id);
        Ok(())
    }
    async fn list(&self) -> CacheResult<Vec<String>> {
        let g = self.inner.lock().await;
        let mut v: Vec<(String, f64)> = g.iter().map(|(k, v)| (k.clone(), *v)).collect();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(v.into_iter().map(|(k, _)| k).collect())
    }
}

/// Redis-backed [`HotRegistry`] writing to the sorted set
/// [`REDIS_HOT_TILES_KEY`]. Used in production. Constructed lazily —
/// callers handle connection failure (we never want a Redis outage to
/// take down the read path; pins still work locally, they just don't
/// propagate to Q083's preheat cron until Redis recovers).
pub struct RedisHotRegistry {
    conn: ::redis::aio::ConnectionManager,
}

impl RedisHotRegistry {
    pub async fn connect(url: &str) -> CacheResult<Self> {
        let client = ::redis::Client::open(url).map_err(|e| CacheError::Redis(e.to_string()))?;
        let conn = ::redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| CacheError::Redis(e.to_string()))?;
        Ok(Self { conn })
    }
}

#[async_trait]
impl HotRegistry for RedisHotRegistry {
    async fn add(&self, tile_id: &str, score: f64) -> CacheResult<()> {
        use ::redis::AsyncCommands;
        let mut c = self.conn.clone();
        let _: i64 = c
            .zadd(REDIS_HOT_TILES_KEY, tile_id, score)
            .await
            .map_err(|e| CacheError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn remove(&self, tile_id: &str) -> CacheResult<()> {
        use ::redis::AsyncCommands;
        let mut c = self.conn.clone();
        let _: i64 = c
            .zrem(REDIS_HOT_TILES_KEY, tile_id)
            .await
            .map_err(|e| CacheError::Redis(e.to_string()))?;
        Ok(())
    }

    async fn list(&self) -> CacheResult<Vec<String>> {
        use ::redis::AsyncCommands;
        let mut c = self.conn.clone();
        let v: Vec<String> = c
            .zrange(REDIS_HOT_TILES_KEY, 0, -1)
            .await
            .map_err(|e| CacheError::Redis(e.to_string()))?;
        Ok(v)
    }
}

/// Cache counters returned by `GET /v1/admin/cache/stats`.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CacheStats {
    pub pinned_entries: usize,
    pub hot_entries: usize,
    pub hot_capacity: usize,
    pub pin_capacity: usize,
    pub hits_pinned: u64,
    pub hits_hot: u64,
    pub hits_cold: u64,
    pub misses: u64,
    pub promotions_cold_to_hot: u64,
    pub evictions_hot: u64,
    pub cold_ttl_expirations: u64,
}

/// Layered (pinned + hot in-memory LRU + cold disk) cache.
pub struct LayeredCache {
    pinned: Mutex<HashMap<TileKey, Vec<u8>>>,
    pin_capacity: usize,
    hot: Mutex<LruMap<TileKey, Vec<u8>>>,
    cold_dir: Option<PathBuf>,
    cold_ttl: Duration,
    counters: Mutex<CacheStats>,
    registry: Arc<dyn HotRegistry>,
}

impl LayeredCache {
    /// Build a cache backed by `cold_dir` (the legacy Q465 directory) +
    /// an in-process pin registry. The default hot LRU capacity is
    /// 10000 and the default cold TTL is 30 days.
    pub fn with_cold_dir(cold_dir: PathBuf) -> Self {
        Self::new(
            Some(cold_dir),
            DEFAULT_HOT_CAPACITY,
            DEFAULT_PIN_CAPACITY,
            DEFAULT_COLD_TTL,
            Arc::new(InMemoryHotRegistry::default()),
        )
    }

    /// Memory-only cache (no cold tier). Useful for unit tests.
    pub fn memory_only(hot_capacity: usize, pin_capacity: usize) -> Self {
        Self::new(
            None,
            hot_capacity,
            pin_capacity,
            DEFAULT_COLD_TTL,
            Arc::new(InMemoryHotRegistry::default()),
        )
    }

    pub fn new(
        cold_dir: Option<PathBuf>,
        hot_capacity: usize,
        pin_capacity: usize,
        cold_ttl: Duration,
        registry: Arc<dyn HotRegistry>,
    ) -> Self {
        let capped = hot_capacity.max(1);
        Self {
            pinned: Mutex::new(HashMap::new()),
            pin_capacity,
            hot: Mutex::new(LruMap::new(capped)),
            cold_dir,
            cold_ttl,
            counters: Mutex::new(CacheStats {
                hot_capacity: capped,
                pin_capacity,
                ..CacheStats::default()
            }),
            registry,
        }
    }

    /// Attach a custom hot-tile registry (e.g. a real Redis backend).
    pub fn with_registry(mut self, r: Arc<dyn HotRegistry>) -> Self {
        self.registry = r;
        self
    }

    /// Whether `key` is currently pinned. Public so tests + the pin
    /// reconciler (future Q083 cron) can introspect.
    pub async fn is_pinned(&self, key: TileKey) -> bool {
        self.pinned.lock().await.contains_key(&key)
    }

    fn cold_path(&self, key: TileKey) -> Option<PathBuf> {
        self.cold_dir.as_ref().map(|d| d.join(key.disk_filename()))
    }

    async fn read_cold(&self, key: TileKey) -> CacheResult<Option<Vec<u8>>> {
        let Some(path) = self.cold_path(key) else {
            return Ok(None);
        };
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CacheError::Io(e)),
        };
        // Cold-tier TTL: a stale manifest is treated as a miss so the
        // handler triggers a re-bake. We do NOT delete the file — a
        // subsequent successful bake will overwrite it atomically.
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = SystemTime::now().duration_since(modified) {
                if age > self.cold_ttl {
                    let mut c = self.counters.lock().await;
                    c.cold_ttl_expirations += 1;
                    debug!(tile=%key.redis_member(), age_secs=age.as_secs(), "cold tile TTL expired");
                    return Ok(None);
                }
            }
        }
        match tokio::fs::read(&path).await {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CacheError::Io(e)),
        }
    }

    async fn write_cold(&self, key: TileKey, bytes: &[u8]) -> CacheResult<()> {
        let Some(path) = self.cold_path(key) else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Atomic write: temp + rename so a partial bytes spill never
        // leaves a torn manifest visible to lookups.
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
}

#[async_trait]
impl TileCache for LayeredCache {
    async fn get(&self, key: TileKey) -> CacheResult<(Option<Vec<u8>>, CacheTier)> {
        // 1. PINNED — direct map hit, never evicts.
        if let Some(bytes) = self.pinned.lock().await.get(&key).cloned() {
            let mut c = self.counters.lock().await;
            c.hits_pinned += 1;
            return Ok((Some(bytes), CacheTier::Pinned));
        }
        // 2. HOT — LRU touch + return.
        if let Some(bytes) = self.hot.lock().await.get(&key).cloned() {
            let mut c = self.counters.lock().await;
            c.hits_hot += 1;
            return Ok((Some(bytes), CacheTier::Hot));
        }
        // 3. COLD — disk, with 30-day TTL.
        if let Some(bytes) = self.read_cold(key).await? {
            // Promote to HOT so repeat reads stay in memory.
            let evicted = {
                let mut h = self.hot.lock().await;
                h.put(key, bytes.clone())
            };
            let mut c = self.counters.lock().await;
            c.hits_cold += 1;
            c.promotions_cold_to_hot += 1;
            if evicted.is_some() {
                c.evictions_hot += 1;
            }
            return Ok((Some(bytes), CacheTier::Cold));
        }
        let mut c = self.counters.lock().await;
        c.misses += 1;
        Ok((None, CacheTier::Miss))
    }

    async fn stats(&self) -> CacheStats {
        let mut s = self.counters.lock().await.clone();
        s.pinned_entries = self.pinned.lock().await.len();
        s.hot_entries = self.hot.lock().await.len();
        s
    }

    async fn list_pins(&self) -> Vec<String> {
        let g = self.pinned.lock().await;
        let mut v: Vec<String> = g.keys().map(TileKey::redis_member).collect();
        v.sort();
        v
    }

    async fn pin(&self, key: TileKey) -> CacheResult<bool> {
        let mut p = self.pinned.lock().await;
        if p.contains_key(&key) {
            return Ok(false);
        }
        if p.len() >= self.pin_capacity {
            return Err(CacheError::PinCapacity {
                cap: self.pin_capacity,
                tile: key.redis_member(),
            });
        }
        // If the manifest is already in HOT or cold-on-disk, lift it
        // into the pinned map so a subsequent eviction can't take it.
        let lifted = {
            let mut h = self.hot.lock().await;
            h.pop(&key)
        };
        let bytes = match lifted {
            Some(b) => Some(b),
            None => self.read_cold(key).await.ok().flatten(),
        };
        p.insert(key, bytes.unwrap_or_default());
        drop(p);
        // Mirror into the Redis registry. We score by insertion-time so
        // Q083 can preheat oldest pins first (rough fairness).
        let score = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as f64)
            .unwrap_or(0.0);
        if let Err(e) = self.registry.add(&key.redis_member(), score).await {
            warn!(tile=%key.redis_member(), error=%e, "hot-registry add failed; pin persists locally");
        }
        Ok(true)
    }

    async fn unpin(&self, key: TileKey) -> CacheResult<bool> {
        let removed = self.pinned.lock().await.remove(&key);
        if let Some(bytes) = removed {
            // Demote into HOT so repeat reads still hit memory.
            if !bytes.is_empty() {
                let mut h = self.hot.lock().await;
                if h.put(key, bytes).is_some() {
                    self.counters.lock().await.evictions_hot += 1;
                }
            }
            if let Err(e) = self.registry.remove(&key.redis_member()).await {
                warn!(tile=%key.redis_member(), error=%e, "hot-registry remove failed");
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[async_trait]
impl TileCacheWriter for LayeredCache {
    async fn put(&self, key: TileKey, bytes: Vec<u8>) -> CacheResult<CacheTier> {
        // Pinned tiles refresh in place — never demoted by a write.
        {
            use std::collections::hash_map::Entry;
            let mut p = self.pinned.lock().await;
            if let Entry::Occupied(mut e) = p.entry(key) {
                e.insert(bytes.clone());
                drop(p);
                self.write_cold(key, &bytes).await?;
                return Ok(CacheTier::Pinned);
            }
        }
        // Everyone else lands in HOT + COLD.
        let evicted = {
            let mut h = self.hot.lock().await;
            h.put(key, bytes.clone())
        };
        if evicted.is_some() {
            self.counters.lock().await.evictions_hot += 1;
        }
        self.write_cold(key, &bytes).await?;
        Ok(CacheTier::Hot)
    }
}

// ---------------------------------------------------------------------------
// Minimal hand-rolled LRU. Keeps the dep graph small (no `lru` crate);
// the cache hot path is dominated by network/disk, not by the LRU's
// constant factors, so a HashMap + VecDeque-of-keys is plenty.
// ---------------------------------------------------------------------------

struct LruMap<K, V> {
    cap: usize,
    map: HashMap<K, V>,
    order: std::collections::VecDeque<K>,
}

impl<K: std::hash::Hash + Eq + Clone, V> LruMap<K, V> {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            map: HashMap::with_capacity(cap),
            order: std::collections::VecDeque::with_capacity(cap),
        }
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn get(&mut self, k: &K) -> Option<&V> {
        if self.map.contains_key(k) {
            // Move-to-front: O(n) on the VecDeque but n is bounded by
            // cap (10k) so the worst case is still trivially fast vs
            // the disk-read path it gates.
            if let Some(pos) = self.order.iter().position(|x| x == k) {
                if pos != 0 {
                    let key = self.order.remove(pos).expect("pos in range");
                    self.order.push_front(key);
                }
            }
            self.map.get(k)
        } else {
            None
        }
    }

    fn put(&mut self, k: K, v: V) -> Option<V> {
        if self.map.contains_key(&k) {
            if let Some(pos) = self.order.iter().position(|x| x == &k) {
                self.order.remove(pos);
            }
            self.order.push_front(k.clone());
            return self.map.insert(k, v);
        }
        let evicted = if self.map.len() >= self.cap {
            if let Some(old) = self.order.pop_back() {
                self.map.remove(&old)
            } else {
                None
            }
        } else {
            None
        };
        self.order.push_front(k.clone());
        self.map.insert(k, v);
        evicted
    }

    fn pop(&mut self, k: &K) -> Option<V> {
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            self.order.remove(pos);
        }
        self.map.remove(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tk(z: u32, x: u32, y: u32) -> TileKey {
        TileKey::new(z, x, y)
    }

    #[tokio::test]
    async fn miss_returns_none_and_increments_counter() {
        let c = LayeredCache::memory_only(4, 4);
        let (v, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert!(v.is_none());
        assert_eq!(tier, CacheTier::Miss);
        assert_eq!(c.stats().await.misses, 1);
    }

    #[tokio::test]
    async fn put_hot_then_get_hot() {
        let c = LayeredCache::memory_only(4, 4);
        c.put(tk(15, 1, 1), b"manifest".to_vec()).await.unwrap();
        let (v, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(v.as_deref(), Some(&b"manifest"[..]));
        assert_eq!(tier, CacheTier::Hot);
        assert_eq!(c.stats().await.hits_hot, 1);
    }

    #[tokio::test]
    async fn pin_promotes_and_get_returns_pinned_tier() {
        let c = LayeredCache::memory_only(4, 4);
        c.put(tk(15, 1, 1), b"m".to_vec()).await.unwrap();
        assert!(c.pin(tk(15, 1, 1)).await.unwrap());
        let (_, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(tier, CacheTier::Pinned);
        assert_eq!(c.stats().await.hits_pinned, 1);
    }

    #[tokio::test]
    async fn unpin_demotes_to_hot() {
        let c = LayeredCache::memory_only(4, 4);
        c.put(tk(15, 1, 1), b"m".to_vec()).await.unwrap();
        c.pin(tk(15, 1, 1)).await.unwrap();
        assert!(c.unpin(tk(15, 1, 1)).await.unwrap());
        let (_, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(tier, CacheTier::Hot);
    }

    #[tokio::test]
    async fn pinned_tile_survives_hot_pressure() {
        // hot capacity = 2, pin one tile, flood with new puts — pinned
        // entry must still be there.
        let c = LayeredCache::memory_only(2, 4);
        c.put(tk(15, 1, 1), b"pinned".to_vec()).await.unwrap();
        c.pin(tk(15, 1, 1)).await.unwrap();
        for i in 0..10 {
            c.put(tk(15, 2, i), vec![0u8; 8]).await.unwrap();
        }
        let (v, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(v.as_deref(), Some(&b"pinned"[..]));
        assert_eq!(tier, CacheTier::Pinned);
    }

    #[tokio::test]
    async fn hot_lru_evicts_oldest_first() {
        let c = LayeredCache::memory_only(2, 4);
        c.put(tk(15, 0, 0), b"a".to_vec()).await.unwrap();
        c.put(tk(15, 0, 1), b"b".to_vec()).await.unwrap();
        // Touch 0,0 so it becomes MRU.
        c.get(tk(15, 0, 0)).await.unwrap();
        c.put(tk(15, 0, 2), b"c".to_vec()).await.unwrap();
        // Now 0,1 should be evicted.
        let (v, _) = c.get(tk(15, 0, 1)).await.unwrap();
        assert!(v.is_none(), "LRU should have evicted 0,1");
        // And 0,0 should still be there.
        let (v, _) = c.get(tk(15, 0, 0)).await.unwrap();
        assert_eq!(v.as_deref(), Some(&b"a"[..]));
    }

    #[tokio::test]
    async fn cold_disk_hit_promotes_to_hot() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("15-1-1.json"), b"on-disk").unwrap();
        let c = LayeredCache::with_cold_dir(tmp.path().to_path_buf());
        let (v, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(v.as_deref(), Some(&b"on-disk"[..]));
        assert_eq!(tier, CacheTier::Cold);
        // Second read should hit HOT now.
        let (v, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(v.as_deref(), Some(&b"on-disk"[..]));
        assert_eq!(tier, CacheTier::Hot);
        let s = c.stats().await;
        assert_eq!(s.promotions_cold_to_hot, 1);
    }

    #[tokio::test]
    async fn cold_ttl_expired_returns_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("15-1-1.json");
        std::fs::write(&p, b"stale").unwrap();
        // Backdate mtime by 31 days.
        let past = std::time::SystemTime::now() - Duration::from_secs(31 * 24 * 60 * 60);
        filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(past)).unwrap();
        let c = LayeredCache::new(
            Some(tmp.path().to_path_buf()),
            4,
            4,
            DEFAULT_COLD_TTL,
            Arc::new(InMemoryHotRegistry::default()),
        );
        let (v, tier) = c.get(tk(15, 1, 1)).await.unwrap();
        assert!(v.is_none(), "expired cold tile must be a miss");
        assert_eq!(tier, CacheTier::Miss);
        assert_eq!(c.stats().await.cold_ttl_expirations, 1);
    }

    #[tokio::test]
    async fn pin_capacity_rejects_overflow() {
        let c = LayeredCache::memory_only(4, 2);
        c.pin(tk(15, 0, 0)).await.unwrap();
        c.pin(tk(15, 0, 1)).await.unwrap();
        let r = c.pin(tk(15, 0, 2)).await;
        assert!(matches!(r, Err(CacheError::PinCapacity { .. })));
    }

    #[tokio::test]
    async fn pin_mirrors_into_hot_registry() {
        let reg = Arc::new(InMemoryHotRegistry::default());
        let c = LayeredCache::new(None, 4, 4, DEFAULT_COLD_TTL, reg.clone());
        c.pin(tk(15, 17128, 9656)).await.unwrap();
        let list = reg.list().await.unwrap();
        assert_eq!(list, vec!["15/17128/9656".to_string()]);
        c.unpin(tk(15, 17128, 9656)).await.unwrap();
        assert!(reg.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn double_pin_is_idempotent() {
        let c = LayeredCache::memory_only(4, 4);
        assert!(c.pin(tk(15, 1, 1)).await.unwrap());
        assert!(!c.pin(tk(15, 1, 1)).await.unwrap()); // already pinned
    }

    #[tokio::test]
    async fn unpin_nonexistent_returns_false() {
        let c = LayeredCache::memory_only(4, 4);
        assert!(!c.unpin(tk(15, 1, 1)).await.unwrap());
    }

    #[tokio::test]
    async fn put_to_pinned_refreshes_in_place() {
        let c = LayeredCache::memory_only(4, 4);
        c.put(tk(15, 1, 1), b"v1".to_vec()).await.unwrap();
        c.pin(tk(15, 1, 1)).await.unwrap();
        let tier = c.put(tk(15, 1, 1), b"v2".to_vec()).await.unwrap();
        assert_eq!(tier, CacheTier::Pinned);
        let (v, _) = c.get(tk(15, 1, 1)).await.unwrap();
        assert_eq!(v.as_deref(), Some(&b"v2"[..]));
    }

    #[tokio::test]
    async fn stats_reflect_entry_counts() {
        let c = LayeredCache::memory_only(4, 4);
        c.put(tk(15, 0, 0), vec![1]).await.unwrap();
        c.put(tk(15, 0, 1), vec![1]).await.unwrap();
        c.pin(tk(15, 0, 0)).await.unwrap();
        let s = c.stats().await;
        assert_eq!(s.pinned_entries, 1);
        assert_eq!(s.hot_entries, 1); // 0,0 was lifted out of HOT into PINNED
    }

    #[tokio::test]
    async fn list_pins_returns_sorted_tile_ids() {
        let c = LayeredCache::memory_only(4, 4);
        c.pin(tk(15, 2, 2)).await.unwrap();
        c.pin(tk(15, 1, 1)).await.unwrap();
        let pins = c.list_pins().await;
        assert_eq!(pins, vec!["15/1/1", "15/2/2"]);
    }
}
