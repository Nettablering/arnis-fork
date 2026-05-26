//! datastore-backup — nightly Roblox DataStore overlay export, age-encrypted,
//! mirrored to Hetzner Storage Box / Backblaze B2 with a local fallback.
//!
//! Architecture (see docs/grill/q099-backup-overlay-snapshots.md):
//!   open-cloud client → jsonl stream → zstd → age → storage targets
//!                                                  + sha256 manifest
//!
//! The crate is split into small modules so each can be unit-tested in
//! isolation, and the [`OpenCloudClient`] trait lets tests inject a synthetic
//! 10-key DataStore without ever touching apis.roblox.com.

pub mod backup;
pub mod crypto;
pub mod manifest;
pub mod restore;
pub mod roblox;
pub mod storage;

pub use backup::{run_backup, BackupConfig, BackupOutcome};
pub use crypto::{decrypt_to_writer, encrypt_to_writer, load_identity, load_recipient};
pub use manifest::{Manifest, ManifestEntry};
pub use restore::{restore_archive, RestoreConfig};
pub use roblox::{DataStoreEntry, OpenCloudClient, OpenCloudHttp, SyntheticClient};
pub use storage::{LocalStorage, StorageTarget};
