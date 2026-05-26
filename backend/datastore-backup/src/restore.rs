//! Restore tool. Pulls an archive + manifest from a [`StorageTarget`],
//! verifies sha256, decrypts age, decompresses zstd, and writes the jsonl
//! back to a target directory. Used both by the CLI and by the weekly
//! random-sample integrity test.

use crate::crypto::{decrypt_to_writer, load_identity};
use crate::manifest::Manifest;
use crate::roblox::DataStoreEntry;
use crate::storage::StorageTarget;
use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use std::path::PathBuf;
use std::sync::Arc;

pub struct RestoreConfig {
    pub universe_id: u64,
    pub date: NaiveDate,
    pub identity_key: PathBuf,
    pub target_dir: PathBuf,
    pub source: Arc<dyn StorageTarget>,
}

pub async fn restore_archive(cfg: RestoreConfig) -> Result<Vec<DataStoreEntry>> {
    std::fs::create_dir_all(&cfg.target_dir)?;
    let key_archive = format!(
        "universes/{}/{}/overlays.jsonl.zst.age",
        cfg.universe_id, cfg.date
    );
    let key_manifest = format!(
        "universes/{}/{}/manifest.json.age",
        cfg.universe_id, cfg.date
    );
    let local_archive = cfg.target_dir.join("overlays.jsonl.zst.age");
    let local_manifest = cfg.target_dir.join("manifest.json.age");
    cfg.source.get(&key_archive, &local_archive).await?;
    cfg.source.get(&key_manifest, &local_manifest).await?;

    let identity = load_identity(&cfg.identity_key)?;

    // decrypt manifest first; verify the (still encrypted) archive against it
    let mut manifest_bytes = Vec::new();
    decrypt_to_writer(
        std::fs::File::open(&local_manifest)?,
        &mut manifest_bytes,
        &identity,
    )?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.verify(&cfg.target_dir)?;

    // decrypt archive → zstd-decompress → parse jsonl
    let mut zst_bytes = Vec::new();
    decrypt_to_writer(
        std::fs::File::open(&local_archive)?,
        &mut zst_bytes,
        &identity,
    )?;
    let jsonl = zstd::stream::decode_all(std::io::Cursor::new(&zst_bytes))?;
    let mut out = Vec::new();
    for line in jsonl.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let e: DataStoreEntry = serde_json::from_slice(line)
            .map_err(|e| anyhow!("malformed jsonl line: {e}"))?;
        out.push(e);
    }

    if out.len() as u64 != manifest.entry_count {
        return Err(anyhow!(
            "entry count mismatch: jsonl={}, manifest={}",
            out.len(),
            manifest.entry_count
        ));
    }
    Ok(out)
}
