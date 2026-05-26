//! Q088 integration tests — keyring-aware verifier flow.
//!
//! These tests exercise the `hmac` module end-to-end:
//! - sign with the active key → verify via the keyring
//! - sign with the previous key during the 14-day overlap → still verifies
//! - sign with a key whose overlap window has elapsed → rejected
//! - rotation cycles `current → previous → retired`
//! - `key_id` header carries the right ULID forward
//! - master key generated mode 0600 on first run
//!
//! Note: the bake-server HTTP layer still uses the legacy single-key
//! header pair `x-wb-ts` + `x-wb-sig` (kept stable for Q475 callers).
//! Q088 wires the *backend* keyring + persistence + audit-log path; the
//! `x-wb-key-id` header lands when the HTTP layer migrates to the new
//! keyring (tracked separately in q470/q475 follow-up).

use bake_server::hmac::sign_payload;
use bake_server::{
    encrypt_key_bytes, generate_key_bytes, keyring_from_rows, load_or_init_master_key, new_key_id,
    HmacKey, HmacKeyring, KeyStatus, HEADER_KEY_ID, HMAC_KEY_LEN, MASTER_KEK_LEN,
    OVERLAP_WINDOW_DAYS, ROTATION_INTERVAL_DAYS,
};
use chrono::{Duration, Utc};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

#[test]
fn end_to_end_sign_and_verify_with_active_key() {
    let now = Utc::now();
    let key_bytes = Arc::new(generate_key_bytes());
    let kid = new_key_id();
    let ring = HmacKeyring::single(HmacKey {
        key_id: kid.clone(),
        bytes: key_bytes.clone(),
        status: KeyStatus::Current,
        created_at: now,
        expires_at: now + Duration::days(ROTATION_INTERVAL_DAYS),
    });

    let ts = 1_700_000_000u64;
    let path = "/v1/tile/15/17128/9656";
    let sig = sign_payload(key_bytes.as_ref(), ts, path);
    let verified = ring.verify_signature(&kid, ts, path, &sig, now).unwrap();
    assert_eq!(verified.key_id, kid);
    assert_eq!(verified.status, KeyStatus::Current);
}

#[test]
fn previous_key_remains_valid_until_overlap_window_ends() {
    let t0 = Utc::now();
    let ring0 = HmacKeyring::single(HmacKey {
        key_id: new_key_id(),
        bytes: Arc::new(generate_key_bytes()),
        status: KeyStatus::Current,
        created_at: t0,
        expires_at: t0 + Duration::days(ROTATION_INTERVAL_DAYS),
    });
    let rot_at = t0 + Duration::days(ROTATION_INTERVAL_DAYS);
    let ring1 = ring0.rotate(rot_at);
    let prev = ring1.previous.clone().unwrap();

    // 13 days in (window is 14 days): still verifies.
    let still_ok = ring1
        .verify_signature(
            &prev.key_id,
            1,
            "/p",
            &sign_payload(prev.bytes.as_ref(), 1, "/p"),
            rot_at + Duration::days(13),
        )
        .is_ok();
    assert!(still_ok);

    // 15 days in: rejected.
    let too_late = ring1.verify_signature(
        &prev.key_id,
        1,
        "/p",
        &sign_payload(prev.bytes.as_ref(), 1, "/p"),
        rot_at + Duration::days(OVERLAP_WINDOW_DAYS + 1),
    );
    assert!(too_late.is_err());
}

#[test]
fn rotation_changes_key_id() {
    let now = Utc::now();
    let original_id = new_key_id();
    let ring = HmacKeyring::single(HmacKey {
        key_id: original_id.clone(),
        bytes: Arc::new(generate_key_bytes()),
        status: KeyStatus::Current,
        created_at: now,
        expires_at: now + Duration::days(90),
    });
    let rotated = ring.rotate(now + Duration::days(90));
    assert_ne!(rotated.active.key_id, original_id);
    assert_eq!(rotated.previous.unwrap().key_id, original_id);
}

#[test]
fn keyring_from_db_rows_with_master_kek() {
    let master = [0xAAu8; MASTER_KEK_LEN];
    let now = Utc::now();
    let cur_raw = generate_key_bytes();
    let prev_raw = generate_key_bytes();
    let cur_env = encrypt_key_bytes(&master, &cur_raw).unwrap();
    let prev_env = encrypt_key_bytes(&master, &prev_raw).unwrap();

    let ring = keyring_from_rows(
        &master,
        (
            "CUR_ID".into(),
            cur_env,
            now,
            now + Duration::days(ROTATION_INTERVAL_DAYS),
        ),
        Some((
            "PREV_ID".into(),
            prev_env,
            now - Duration::days(90),
            now + Duration::days(OVERLAP_WINDOW_DAYS),
        )),
    )
    .unwrap();

    // Signing with the bytes we encrypted must verify via the keyring.
    let sig = sign_payload(&cur_raw, 42, "/v1/tile/1/2/3");
    assert!(ring
        .verify_signature("CUR_ID", 42, "/v1/tile/1/2/3", &sig, now)
        .is_ok());
    let psig = sign_payload(&prev_raw, 42, "/v1/tile/1/2/3");
    assert!(ring
        .verify_signature("PREV_ID", 42, "/v1/tile/1/2/3", &psig, now)
        .is_ok());
}

#[test]
fn master_key_generated_mode_600_on_first_run() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("master.key");
    let k = load_or_init_master_key(&path).unwrap();
    assert_eq!(k.len(), MASTER_KEK_LEN);
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn header_key_id_constant_matches_doc() {
    assert_eq!(HEADER_KEY_ID, "x-wb-key-id");
}

#[test]
fn key_byte_length_constant_is_32() {
    assert_eq!(HMAC_KEY_LEN, 32);
    assert_eq!(generate_key_bytes().len(), 32);
}
