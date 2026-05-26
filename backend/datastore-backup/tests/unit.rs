//! Unit tests for datastore-backup (Q099). Twelve focused tests covering
//! crypto roundtrip, manifest verification, restore-on-corruption,
//! key-rotation, storage put/get, synthetic-client shape, and edge cases.

use chrono::Utc;
use datastore_backup::{
    crypto::{decrypt_to_writer, encrypt_to_writer, load_identity, load_recipient},
    manifest::Manifest,
    roblox::{OpenCloudClient, SyntheticClient},
    storage::{LocalStorage, StorageTarget},
};
use std::io::Cursor;
use std::path::Path;
use tempfile::TempDir;

fn make_keypair(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let pub_path = dir.join("k.pub");
    let key_path = dir.join("k.key");
    // generate with the `age` crate so the test is hermetic (no system binary)
    use age::secrecy::ExposeSecret;
    let id = age::x25519::Identity::generate();
    let pub_str = id.to_public().to_string();
    let sec_str = id.to_string();
    std::fs::write(&pub_path, format!("# public key: {pub_str}\n{pub_str}\n")).unwrap();
    std::fs::write(
        &key_path,
        format!("# public key: {pub_str}\n{}\n", sec_str.expose_secret()),
    )
    .unwrap();
    (pub_path, key_path)
}

#[test]
fn test_01_age_roundtrip_small() {
    let dir = TempDir::new().unwrap();
    let (pub_p, key_p) = make_keypair(dir.path());
    let recipients = load_recipient(&pub_p).unwrap();
    let id = load_identity(&key_p).unwrap();
    let plaintext = b"hello worldbuilders";
    let mut ct = Vec::new();
    encrypt_to_writer(Cursor::new(plaintext), &mut ct, recipients).unwrap();
    let mut pt = Vec::new();
    decrypt_to_writer(Cursor::new(&ct), &mut pt, &id).unwrap();
    assert_eq!(pt, plaintext);
}

#[test]
fn test_02_age_roundtrip_large_random() {
    let dir = TempDir::new().unwrap();
    let (pub_p, key_p) = make_keypair(dir.path());
    let recipients = load_recipient(&pub_p).unwrap();
    let id = load_identity(&key_p).unwrap();
    let mut data = Vec::with_capacity(64 * 1024);
    for i in 0..(64 * 1024) {
        data.push((i * 31) as u8);
    }
    let mut ct = Vec::new();
    encrypt_to_writer(Cursor::new(&data), &mut ct, recipients).unwrap();
    let mut pt = Vec::new();
    decrypt_to_writer(Cursor::new(&ct), &mut pt, &id).unwrap();
    assert_eq!(pt, data);
    assert_ne!(ct, data, "ciphertext must differ from plaintext");
}

#[test]
fn test_03_recipient_file_parses_with_header() {
    let dir = TempDir::new().unwrap();
    let id = age::x25519::Identity::generate();
    let pubk = id.to_public().to_string();
    let p = dir.path().join("k.pub");
    std::fs::write(&p, format!("# created: now\n# public key: {pubk}\n{pubk}\n")).unwrap();
    let recs = load_recipient(&p).unwrap();
    assert_eq!(recs.len(), 1);
}

#[test]
fn test_04_manifest_verify_ok() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("a.bin");
    std::fs::write(&f, b"abcdefg").unwrap();
    let mut m = Manifest::new(1, "age1xxxx".into());
    m.add_file(&f, "a.bin").unwrap();
    m.verify(dir.path()).unwrap();
}

#[test]
fn test_05_manifest_detects_corruption() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("a.bin");
    std::fs::write(&f, b"original").unwrap();
    let mut m = Manifest::new(1, "age1xxxx".into());
    m.add_file(&f, "a.bin").unwrap();
    std::fs::write(&f, b"tampered").unwrap();
    assert!(m.verify(dir.path()).is_err());
}

#[test]
fn test_06_manifest_detects_size_mismatch() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("a.bin");
    std::fs::write(&f, b"abcdefg").unwrap();
    let mut m = Manifest::new(1, "age1".into());
    m.add_file(&f, "a.bin").unwrap();
    std::fs::write(&f, b"abc").unwrap();
    let err = m.verify(dir.path()).unwrap_err().to_string();
    assert!(err.contains("size mismatch"), "{err}");
}

#[test]
fn test_07_manifest_detects_missing_file() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("a.bin");
    std::fs::write(&f, b"abc").unwrap();
    let mut m = Manifest::new(1, "age1".into());
    m.add_file(&f, "a.bin").unwrap();
    std::fs::remove_file(&f).unwrap();
    assert!(m.verify(dir.path()).is_err());
}

#[tokio::test]
async fn test_08_local_storage_roundtrip() {
    let dir = TempDir::new().unwrap();
    let s = LocalStorage::new(dir.path());
    let src = dir.path().join("src.bin");
    std::fs::write(&src, b"payload").unwrap();
    s.put(&src, "universes/1/2026-05-26/x.bin").await.unwrap();
    let dst = dir.path().join("dst.bin");
    s.get("universes/1/2026-05-26/x.bin", &dst).await.unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), b"payload");
}

#[tokio::test]
async fn test_09_synthetic_client_returns_ten_keys() {
    let c = SyntheticClient::ten_key_fixture(42);
    let v = c.list_all_entries(42).await.unwrap();
    assert_eq!(v.len(), 10);
    assert!(v.iter().all(|e| e.datastore == "OverlayStore"));
}

#[test]
fn test_10_key_rotation_old_key_cannot_decrypt_new_archive() {
    let dir = TempDir::new().unwrap();
    let (pub_old, key_old) = make_keypair(dir.path());
    // rotate: new keypair in a separate dir
    let dir2 = TempDir::new().unwrap();
    let (pub_new, _key_new) = make_keypair(dir2.path());
    let recipients_new = load_recipient(&pub_new).unwrap();
    let id_old = load_identity(&key_old).unwrap();
    let mut ct = Vec::new();
    encrypt_to_writer(Cursor::new(b"secret"), &mut ct, recipients_new).unwrap();
    let mut pt = Vec::new();
    let err = decrypt_to_writer(Cursor::new(&ct), &mut pt, &id_old);
    assert!(err.is_err(), "old key must not decrypt archive encrypted to new key");
    // (and sanity: the OLD key still works on data encrypted to it)
    let mut ct2 = Vec::new();
    encrypt_to_writer(
        Cursor::new(b"older"),
        &mut ct2,
        load_recipient(&pub_old).unwrap(),
    )
    .unwrap();
    let mut pt2 = Vec::new();
    decrypt_to_writer(Cursor::new(&ct2), &mut pt2, &id_old).unwrap();
    assert_eq!(pt2, b"older");
}

#[test]
fn test_11_manifest_serde_roundtrip() {
    let mut m = Manifest::new(42, "age1abc".into());
    m.created_at = Utc::now();
    m.entry_count = 10;
    let s = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&s).unwrap();
    assert_eq!(back, m);
}

#[test]
fn test_12_load_recipient_empty_file_errors() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("empty.pub");
    std::fs::write(&p, "# only comments\n\n").unwrap();
    assert!(load_recipient(&p).is_err());
}
