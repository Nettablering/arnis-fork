//! Reader for the Q082 pinned-tile sorted set `wb:cache.hot.tiles`.
//!
//! Abstracted behind a trait so tests can inject an in-process fake
//! without a running redis-server. Production wires in
//! [`RedisPinnedReader`] which issues `ZRANGEBYSCORE -inf +inf`.

use async_trait::async_trait;

/// Read-only view of the pinned sorted-set members. The score (pin time)
/// is not interesting to the preheat cron; we just want the membership
/// list to dedupe against the landmarks manifest.
#[async_trait]
pub trait PinnedReader: Send + Sync {
    /// Returns the tile-ids currently in the pinned sorted set, in any
    /// order. Each id is `"{z}/{x}/{y}"`.
    async fn read_pinned(&self) -> anyhow::Result<Vec<String>>;
}

/// Real Redis-backed reader.
pub struct RedisPinnedReader {
    conn: ::redis::aio::ConnectionManager,
    key: String,
}

impl RedisPinnedReader {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = ::redis::Client::open(url)?;
        let conn = ::redis::aio::ConnectionManager::new(client).await?;
        Ok(Self {
            conn,
            key: bake_server_hot_tiles_key().to_string(),
        })
    }

    /// Override the sorted-set key (used by tests that want to point at
    /// an isolated key on a shared redis).
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
}

#[async_trait]
impl PinnedReader for RedisPinnedReader {
    async fn read_pinned(&self) -> anyhow::Result<Vec<String>> {
        use ::redis::AsyncCommands;
        let mut conn = self.conn.clone();
        // ZRANGEBYSCORE <key> -inf +inf — matches the Q082 contract.
        let members: Vec<String> = conn.zrangebyscore(&self.key, "-inf", "+inf").await?;
        Ok(members)
    }
}

/// Canonical pinned-tile sorted-set key.
///
/// Duplicates `bake_server::cache::REDIS_HOT_TILES_KEY` deliberately:
/// `bake-preheat` is a leaf crate that must not depend on bake-server's
/// HTTP stack just to read a constant. The wire contract is the same
/// string; both sides keep it under their own constant + the value is
/// regression-tested by the integration test.
pub fn bake_server_hot_tiles_key() -> &'static str {
    "wb:cache.hot.tiles"
}

/// In-process fake for tests.
#[derive(Debug, Clone, Default)]
pub struct InMemoryPinnedReader {
    pub tiles: Vec<String>,
}

impl InMemoryPinnedReader {
    pub fn new(tiles: Vec<String>) -> Self {
        Self { tiles }
    }
}

#[async_trait]
impl PinnedReader for InMemoryPinnedReader {
    async fn read_pinned(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.tiles.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_reader_round_trips() {
        let r = InMemoryPinnedReader::new(vec!["15/1/2".into(), "15/3/4".into()]);
        let got = r.read_pinned().await.unwrap();
        assert_eq!(got, vec!["15/1/2".to_string(), "15/3/4".to_string()]);
    }

    #[test]
    fn canonical_key_matches_q082() {
        // Keep the wire-format string locked to Q082's contract.
        assert_eq!(bake_server_hot_tiles_key(), "wb:cache.hot.tiles");
    }
}
