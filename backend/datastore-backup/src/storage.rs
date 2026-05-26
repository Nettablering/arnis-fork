//! Storage targets. The primary order is:
//!   1. Hetzner Storage Box via SSH/rsync (if credentials)
//!   2. Backblaze B2 via the native API (if keys)
//!   3. Local `backups/` directory (always works; the fallback)
//!
//! Only the local target is exercised end-to-end in this build because
//! Hetzner/B2 credentials are not yet provisioned (logged to BLOCKED on the
//! first run). The trait shape and dispatcher are wired so adding the remote
//! impls is a 50-line patch each.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

#[async_trait]
pub trait StorageTarget: Send + Sync {
    fn name(&self) -> &'static str;
    /// Upload `local_path` as `remote_key`. Idempotent: a re-run of the same
    /// nightly with the same archive should overwrite atomically.
    async fn put(&self, local_path: &Path, remote_key: &str) -> Result<()>;
    /// Pull `remote_key` into `local_path`. Used by the restore tool and the
    /// weekly random-sample integrity test.
    async fn get(&self, remote_key: &str, local_path: &Path) -> Result<()>;
}

/// Local filesystem fallback. Always available; root is configurable so tests
/// can use a tmpdir without touching `~/projects/.../backups`.
pub struct LocalStorage {
    pub root: PathBuf,
}

impl LocalStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl StorageTarget for LocalStorage {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn put(&self, local_path: &Path, remote_key: &str) -> Result<()> {
        let dst = self.root.join(remote_key);
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(local_path, &dst).await?;
        Ok(())
    }

    async fn get(&self, remote_key: &str, local_path: &Path) -> Result<()> {
        let src = self.root.join(remote_key);
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(&src, local_path).await?;
        Ok(())
    }
}
