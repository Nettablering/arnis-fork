//! End-to-end integration tests for Q099.
//!
//! 1. test_e2e_synthetic_backup_and_restore: backup a 10-key synthetic
//!    DataStore, encrypt + store locally, restore, and compare every entry
//!    byte-for-byte.
//! 2. test_restore_fails_cleanly_after_corruption: same flow, but corrupt
//!    the archive on disk before restore — restore MUST fail with a
//!    manifest-verify error and emit NO partial output.

use datastore_backup::{
    backup::{run_backup, BackupConfig},
    restore::{restore_archive, RestoreConfig},
    roblox::{OpenCloudClient, SyntheticClient},
    storage::{LocalStorage, StorageTarget},
};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn make_keypair(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    use age::secrecy::ExposeSecret;
    let id = age::x25519::Identity::generate();
    let pubk = id.to_public().to_string();
    let pp = dir.join("k.pub");
    let kp = dir.join("k.key");
    std::fs::write(&pp, format!("{pubk}\n")).unwrap();
    std::fs::write(
        &kp,
        format!("# public key: {pubk}\n{}\n", id.to_string().expose_secret()),
    )
    .unwrap();
    (pp, kp)
}

#[tokio::test]
async fn test_e2e_synthetic_backup_and_restore() {
    let tmp = TempDir::new().unwrap();
    let (pubk, key) = make_keypair(tmp.path());
    let workdir = tmp.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let storage_root = tmp.path().join("storage");
    let storage: Arc<dyn StorageTarget> = Arc::new(LocalStorage::new(&storage_root));
    let client = SyntheticClient::ten_key_fixture(99);
    let date = chrono::NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();
    let outcome = run_backup(
        &client,
        BackupConfig {
            universe_id: 99,
            date,
            recipient_pub: pubk.clone(),
            targets: vec![storage.clone()],
            work_dir: workdir.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.entry_count, 10);
    assert_eq!(outcome.targets_ok, vec!["local"]);
    assert!(outcome.targets_failed.is_empty());

    // restore
    let restore_dir = tmp.path().join("restore");
    let entries = restore_archive(RestoreConfig {
        universe_id: 99,
        date,
        identity_key: key,
        target_dir: restore_dir,
        source: storage.clone(),
    })
    .await
    .unwrap();

    // compare to expected synthetic fixture
    let expected = client.list_all_entries(99).await.unwrap();
    assert_eq!(entries.len(), expected.len());
    for (a, b) in entries.iter().zip(expected.iter()) {
        assert_eq!(a, b, "decrypted entry must match source");
    }
}

#[tokio::test]
async fn test_restore_fails_cleanly_after_corruption() {
    let tmp = TempDir::new().unwrap();
    let (pubk, key) = make_keypair(tmp.path());
    let workdir = tmp.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let storage_root = tmp.path().join("storage");
    let storage: Arc<dyn StorageTarget> = Arc::new(LocalStorage::new(&storage_root));
    let client = SyntheticClient::ten_key_fixture(7);
    let date = chrono::NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();

    run_backup(
        &client,
        BackupConfig {
            universe_id: 7,
            date,
            recipient_pub: pubk,
            targets: vec![storage.clone()],
            work_dir: workdir,
        },
    )
    .await
    .unwrap();

    // corrupt the archive in the storage root
    let archive = storage_root.join("universes/7/2026-05-26/overlays.jsonl.zst.age");
    let mut bytes = std::fs::read(&archive).unwrap();
    // flip a middle byte (avoid age header)
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&archive, &bytes).unwrap();

    let restore_dir = tmp.path().join("restore");
    let result = restore_archive(RestoreConfig {
        universe_id: 7,
        date,
        identity_key: key,
        target_dir: restore_dir.clone(),
        source: storage,
    })
    .await;
    assert!(result.is_err(), "corrupted archive must not restore");
    // no entries.jsonl should have been emitted
    assert!(!restore_dir.join("entries.jsonl").exists());
}
