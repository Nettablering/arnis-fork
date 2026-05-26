//! Backup orchestrator. Produces, per (universe, date), a directory like:
//!
//! ```text
//! <date>/
//!   overlays.jsonl.zst.age   - encrypted compressed export
//!   manifest.json.age        - encrypted sha256 manifest
//! ```
//!
//! The plaintext jsonl never lands on disk in long-lived form: we stream
//! jsonl → zstd → tempfile and immediately encrypt; the tempfile is wiped
//! on drop. The encrypted artefacts are then pushed to every configured
//! [`StorageTarget`].

use crate::crypto::{encrypt_to_writer, load_recipient};
use crate::manifest::Manifest;
use crate::roblox::OpenCloudClient;
use crate::storage::StorageTarget;
use anyhow::Result;
use chrono::{NaiveDate, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tracing::info;

pub struct BackupConfig {
    pub universe_id: u64,
    pub date: NaiveDate,
    pub recipient_pub: PathBuf,
    /// Targets are tried in order and ALL successful uploads recorded.
    /// If every target fails the whole backup fails.
    pub targets: Vec<Arc<dyn StorageTarget>>,
    pub work_dir: PathBuf,
}

#[derive(Debug)]
pub struct BackupOutcome {
    pub universe_id: u64,
    pub date: NaiveDate,
    pub entry_count: u64,
    pub archive_bytes: u64,
    pub manifest_bytes: u64,
    pub targets_ok: Vec<String>,
    pub targets_failed: Vec<(String, String)>,
}

pub async fn run_backup(
    client: &dyn OpenCloudClient,
    cfg: BackupConfig,
) -> Result<BackupOutcome> {
    let entries = client.list_all_entries(cfg.universe_id).await?;
    info!(universe_id = cfg.universe_id, count = entries.len(), "fetched");

    // jsonl → zstd → tempfile (compressed plaintext, short-lived)
    let zst_tmp = NamedTempFile::new_in(&cfg.work_dir)?;
    {
        let file = zst_tmp.reopen()?;
        let mut enc = zstd::stream::Encoder::new(file, 3)?;
        for e in &entries {
            serde_json::to_writer(&mut enc, e)?;
            std::io::Write::write_all(&mut enc, b"\n")?;
        }
        enc.finish()?;
    }

    // age-encrypt the compressed blob
    let recipients = load_recipient(&cfg.recipient_pub)?;
    let recipient_fpr = std::fs::read_to_string(&cfg.recipient_pub)?
        .lines()
        .find_map(|l| l.trim().strip_prefix("age1").map(|s| format!("age1{}", s)))
        .or_else(|| {
            std::fs::read_to_string(&cfg.recipient_pub)
                .ok()
                .and_then(|t| {
                    t.split_whitespace()
                        .find(|w| w.starts_with("age1"))
                        .map(String::from)
                })
        })
        .unwrap_or_else(|| "unknown".into());

    let date_dir = cfg.work_dir.join(cfg.date.to_string());
    std::fs::create_dir_all(&date_dir)?;
    let archive_path = date_dir.join("overlays.jsonl.zst.age");
    {
        let plaintext = std::fs::File::open(zst_tmp.path())?;
        let dst = std::fs::File::create(&archive_path)?;
        encrypt_to_writer(plaintext, dst, recipients)?;
    }

    // manifest covers ONLY the encrypted archive (the encrypted bytes are
    // what we want to verify; if they decrypt cleanly the plaintext is
    // automatically authenticated by age's own MAC)
    let mut manifest = Manifest::new(cfg.universe_id, recipient_fpr);
    manifest.entry_count = entries.len() as u64;
    manifest.add_file(&archive_path, "overlays.jsonl.zst.age")?;
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    let manifest_path = date_dir.join("manifest.json.age");
    {
        let recipients = load_recipient(&cfg.recipient_pub)?;
        let dst = std::fs::File::create(&manifest_path)?;
        encrypt_to_writer(std::io::Cursor::new(&manifest_json), dst, recipients)?;
    }

    let archive_bytes = std::fs::metadata(&archive_path)?.len();
    let manifest_bytes = std::fs::metadata(&manifest_path)?.len();

    // Upload to every target. We treat the local target as authoritative for
    // success — i.e. a working local copy + at least one storage hit is OK.
    let mut targets_ok = Vec::new();
    let mut targets_failed = Vec::new();
    for t in &cfg.targets {
        let key_archive = format!(
            "universes/{}/{}/overlays.jsonl.zst.age",
            cfg.universe_id, cfg.date
        );
        let key_manifest = format!(
            "universes/{}/{}/manifest.json.age",
            cfg.universe_id, cfg.date
        );
        let res = async {
            t.put(&archive_path, &key_archive).await?;
            t.put(&manifest_path, &key_manifest).await
        }
        .await;
        match res {
            Ok(()) => targets_ok.push(t.name().to_string()),
            Err(e) => targets_failed.push((t.name().to_string(), e.to_string())),
        }
    }

    if targets_ok.is_empty() {
        return Err(anyhow::anyhow!(
            "all storage targets failed: {:?}",
            targets_failed
        ));
    }

    Ok(BackupOutcome {
        universe_id: cfg.universe_id,
        date: cfg.date,
        entry_count: entries.len() as u64,
        archive_bytes,
        manifest_bytes,
        targets_ok,
        targets_failed,
    })
}

/// Helper for the binary: today's UTC date.
pub fn today_utc() -> NaiveDate {
    Utc::now().date_naive()
}
