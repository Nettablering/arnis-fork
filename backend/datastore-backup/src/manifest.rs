//! Per-backup SHA-256 manifest. The manifest itself is age-encrypted alongside
//! the archive; on restore we recompute hashes and refuse to emit data if a
//! single byte changed.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    pub created_at: DateTime<Utc>,
    pub universe_id: u64,
    pub entry_count: u64,
    pub age_recipient_fingerprint: String,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub fn new(universe_id: u64, recipient_fpr: String) -> Self {
        Self {
            schema_version: 1,
            created_at: Utc::now(),
            universe_id,
            entry_count: 0,
            age_recipient_fingerprint: recipient_fpr,
            entries: Vec::new(),
        }
    }

    pub fn add_file(&mut self, path: &Path, filename: &str) -> Result<()> {
        let bytes = std::fs::read(path)?;
        let mut h = Sha256::new();
        h.update(&bytes);
        self.entries.push(ManifestEntry {
            filename: filename.to_string(),
            sha256: hex::encode(h.finalize()),
            size_bytes: bytes.len() as u64,
        });
        Ok(())
    }

    /// Verify that every file in `dir` matches its recorded hash.
    /// A missing file or a hash mismatch returns Err — restore aborts hard.
    pub fn verify(&self, dir: &Path) -> Result<()> {
        for e in &self.entries {
            let p = dir.join(&e.filename);
            let bytes = std::fs::read(&p)
                .map_err(|err| anyhow!("manifest verify: missing {}: {err}", e.filename))?;
            if bytes.len() as u64 != e.size_bytes {
                return Err(anyhow!(
                    "manifest verify: size mismatch for {} (got {}, expected {})",
                    e.filename,
                    bytes.len(),
                    e.size_bytes
                ));
            }
            let mut h = Sha256::new();
            h.update(&bytes);
            let got = hex::encode(h.finalize());
            if got != e.sha256 {
                return Err(anyhow!(
                    "manifest verify: sha256 mismatch for {}",
                    e.filename
                ));
            }
        }
        Ok(())
    }
}
