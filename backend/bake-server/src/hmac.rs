//! Q088 — HMAC signing key rotation.
//!
//! Per-universe HMAC-SHA256 keys, rotated every 90 days, with a 14-day
//! dual-key overlap window during which both `current` and `previous`
//! keys verify successfully. Signed payloads include an `x-wb-key-id`
//! header so the verifier knows which key version to consult.
//!
//! See [`docs/grill/q088-hmac-signing-key-rotation.md`].
//!
//! This module deliberately decouples the *crypto* (sign/verify), the
//! *keyring* (active + previous), the *encryption-at-rest* (master KEK
//! wraps each 32-byte HMAC key with ChaCha20-Poly1305), and the
//! *persistence* (env loader + Postgres loader). The bake-server uses
//! the keyring through a single [`HmacKeyring::verify_signature`] entry
//! point; rotation runs out-of-band via the operator script
//! `backend/scripts/rotate-hmac.sh` or the systemd timer.

use ::hmac::{Hmac, Mac};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng as AeadOsRng},
    ChaCha20Poly1305, Key as ChaChaKey, Nonce,
};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sha2::Sha256;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use subtle::ConstantTimeEq;

/// Raw HMAC key is always exactly 32 bytes (256-bit).
pub const HMAC_KEY_LEN: usize = 32;

/// Master KEK is 32 bytes (ChaCha20-Poly1305 key length).
pub const MASTER_KEK_LEN: usize = 32;

/// ChaCha20-Poly1305 nonce is 12 bytes.
pub const NONCE_LEN: usize = 12;

/// Rotation cadence — the primary key is demoted to `previous` after this.
pub const ROTATION_INTERVAL_DAYS: i64 = 90;

/// Dual-key overlap window — `previous` is accepted for this long after demotion.
pub const OVERLAP_WINDOW_DAYS: i64 = 14;

/// Header that carries the key version identifier (ULID).
pub const HEADER_KEY_ID: &str = "x-wb-key-id";

/// One HMAC key version with metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HmacKey {
    pub key_id: String,
    pub bytes: Arc<[u8; HMAC_KEY_LEN]>,
    pub status: KeyStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyStatus {
    Current,
    Previous,
    Retired,
}

impl KeyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyStatus::Current => "current",
            KeyStatus::Previous => "previous",
            KeyStatus::Retired => "retired",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "current" => Ok(KeyStatus::Current),
            "previous" => Ok(KeyStatus::Previous),
            "retired" => Ok(KeyStatus::Retired),
            other => Err(anyhow!("unknown key status: {other}")),
        }
    }
}

/// A keyring carries at most one `current` and (optionally) one
/// `previous` key. Both verify successfully while `previous` is within
/// its overlap window.
#[derive(Clone, Debug)]
pub struct HmacKeyring {
    pub active: HmacKey,
    pub previous: Option<HmacKey>,
}

impl HmacKeyring {
    pub fn single(active: HmacKey) -> Self {
        Self {
            active,
            previous: None,
        }
    }

    /// Resolve a key by `key_id`, returning `None` if it's neither
    /// active nor a still-valid previous key.
    pub fn key_by_id(&self, key_id: &str, now: DateTime<Utc>) -> Option<&HmacKey> {
        if self.active.key_id == key_id {
            return Some(&self.active);
        }
        if let Some(p) = &self.previous {
            if p.key_id == key_id && now <= p.expires_at && p.status != KeyStatus::Retired {
                return Some(p);
            }
        }
        None
    }

    /// Verify a signature given the header-derived `key_id`. Returns
    /// the matching key on success.
    pub fn verify_signature(
        &self,
        key_id: &str,
        ts: u64,
        path: &str,
        sig_hex: &str,
        now: DateTime<Utc>,
    ) -> Result<&HmacKey> {
        let key = self
            .key_by_id(key_id, now)
            .ok_or_else(|| anyhow!("unknown or retired key_id"))?;
        let expected = sign_payload(key.bytes.as_ref(), ts, path);
        if !ct_eq(expected.as_bytes(), sig_hex.as_bytes()) {
            bail!("bad signature");
        }
        Ok(key)
    }

    /// Rotate the keyring locally (no I/O). The current key is demoted
    /// to previous (status flipped, `expires_at` set to now + overlap
    /// window); a freshly generated key takes its place as current.
    ///
    /// Any pre-existing `previous` is dropped — its overlap window has
    /// already elapsed once we rotate again.
    pub fn rotate(&self, now: DateTime<Utc>) -> Self {
        let mut demoted = self.active.clone();
        demoted.status = KeyStatus::Previous;
        demoted.expires_at = now + Duration::days(OVERLAP_WINDOW_DAYS);

        let new_active = HmacKey {
            key_id: new_key_id(),
            bytes: Arc::new(generate_key_bytes()),
            status: KeyStatus::Current,
            created_at: now,
            expires_at: now + Duration::days(ROTATION_INTERVAL_DAYS),
        };
        Self {
            active: new_active,
            previous: Some(demoted),
        }
    }
}

// ─── Crypto: HMAC sign/verify ────────────────────────────────────────

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 over `<ts>\n<path>`, hex-encoded.
pub fn sign_payload(key: &[u8], ts: u64, path: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("hmac key");
    mac.update(format!("{ts}\n{path}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

// ─── Key generation ──────────────────────────────────────────────────

pub fn generate_key_bytes() -> [u8; HMAC_KEY_LEN] {
    let mut buf = [0u8; HMAC_KEY_LEN];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

pub fn new_key_id() -> String {
    ulid::Ulid::new().to_string()
}

// ─── Encryption at rest: ChaCha20-Poly1305 ───────────────────────────

/// Default location for the master KEK. Mode 0600, NEVER echoed.
pub fn default_master_key_path() -> PathBuf {
    PathBuf::from(
        std::env::var("WB_HMAC_MASTER_KEY_PATH").unwrap_or_else(|_| {
            "/home/deploy/.claude/shared/api-keys/worldbuilders-hmac-master.key".into()
        }),
    )
}

/// Load the master KEK from disk; create it (mode 0600, crypto-secure
/// random bytes) on first run. Caller must keep the bytes off the
/// log/trace path.
pub fn load_or_init_master_key(path: &Path) -> Result<[u8; MASTER_KEK_LEN]> {
    if path.exists() {
        let raw = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let decoded = if raw.len() == MASTER_KEK_LEN {
            raw
        } else {
            // Accept either raw 32-byte or base64-encoded (newline-terminated).
            let trimmed = String::from_utf8_lossy(&raw).trim().to_string();
            B64.decode(trimmed.as_bytes())
                .with_context(|| "master key file is not 32 raw bytes or base64")?
        };
        if decoded.len() != MASTER_KEK_LEN {
            bail!(
                "master KEK at {} has wrong length {} (need {})",
                path.display(),
                decoded.len(),
                MASTER_KEK_LEN
            );
        }
        let mut out = [0u8; MASTER_KEK_LEN];
        out.copy_from_slice(&decoded);
        return Ok(out);
    }

    // First run: generate, write mode 0600. We base64-encode so the
    // file is text-safe (rsync/backup tools, age-encrypt layer below).
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir {}", parent.display()))?;
    }
    let mut buf = [0u8; MASTER_KEK_LEN];
    rand::thread_rng().fill_bytes(&mut buf);
    let encoded = B64.encode(buf);
    fs::write(path, format!("{encoded}\n")).with_context(|| format!("write {}", path.display()))?;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(buf)
}

/// Envelope = nonce(12) || ciphertext(32 + 16 tag) = 60 bytes.
pub fn encrypt_key_bytes(master: &[u8; MASTER_KEK_LEN], key: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(ChaChaKey::from_slice(master));
    let mut nonce = [0u8; NONCE_LEN];
    AeadOsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), key)
        .map_err(|e| anyhow!("chacha20poly1305 encrypt: {e}"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt_key_bytes(master: &[u8; MASTER_KEK_LEN], envelope: &[u8]) -> Result<Vec<u8>> {
    if envelope.len() < NONCE_LEN + 16 {
        bail!("envelope too short ({} bytes)", envelope.len());
    }
    let (nonce_b, ct) = envelope.split_at(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new(ChaChaKey::from_slice(master));
    cipher.decrypt(Nonce::from_slice(nonce_b), ct).map_err(|e| {
        anyhow!("chacha20poly1305 decrypt (bad master key or corrupted envelope): {e}")
    })
}

// ─── Loading: env (single-key) or persisted (envelope-wrapped) ───────

/// Load a single-key ring from the legacy `BAKE_HMAC_KEY` hex env var.
/// Used by tests and by the bootstrap path before any rotation has run.
pub fn load_from_env_legacy(now: DateTime<Utc>) -> Result<HmacKeyring> {
    let hex_key = std::env::var("BAKE_HMAC_KEY").context("BAKE_HMAC_KEY env var missing")?;
    let raw = hex::decode(hex_key.trim()).context("BAKE_HMAC_KEY not valid hex")?;
    if raw.len() < HMAC_KEY_LEN {
        bail!(
            "BAKE_HMAC_KEY must decode to >={HMAC_KEY_LEN} bytes, got {}",
            raw.len()
        );
    }
    let mut bytes = [0u8; HMAC_KEY_LEN];
    bytes.copy_from_slice(&raw[..HMAC_KEY_LEN]);
    Ok(HmacKeyring::single(HmacKey {
        key_id: "legacy".to_string(),
        bytes: Arc::new(bytes),
        status: KeyStatus::Current,
        created_at: now,
        expires_at: now + Duration::days(ROTATION_INTERVAL_DAYS),
    }))
}

/// Decrypt a row's `key_bytes` envelope back to a 32-byte key. Pure
/// helper so callers can construct an `HmacKey` from any persistence
/// layer (Postgres, sqlite, JSON test fixture).
pub fn unwrap_key_row(
    master: &[u8; MASTER_KEK_LEN],
    key_id: String,
    envelope: &[u8],
    status: &str,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<HmacKey> {
    let raw = decrypt_key_bytes(master, envelope)?;
    if raw.len() != HMAC_KEY_LEN {
        bail!("decrypted key has wrong length {}", raw.len());
    }
    let mut bytes = [0u8; HMAC_KEY_LEN];
    bytes.copy_from_slice(&raw);
    Ok(HmacKey {
        key_id,
        bytes: Arc::new(bytes),
        status: KeyStatus::parse(status)?,
        created_at,
        expires_at,
    })
}

/// A row fetched from `wb.hmac_keys`: `(key_id, encrypted_envelope,
/// created_at, expires_at)`.
pub type HmacKeyRow = (String, Vec<u8>, DateTime<Utc>, DateTime<Utc>);

/// Build a keyring from raw rows fetched out of `wb.hmac_keys` (current +
/// optional previous, both still within their `expires_at`).
///
/// The caller has already filtered for `universe_id` and `status`.
pub fn keyring_from_rows(
    master: &[u8; MASTER_KEK_LEN],
    current_row: HmacKeyRow,
    previous_row: Option<HmacKeyRow>,
) -> Result<HmacKeyring> {
    let (cid, ce, cca, cea) = current_row;
    let active = unwrap_key_row(master, cid, &ce, "current", cca, cea)?;
    let previous = match previous_row {
        Some((pid, pe, pca, pea)) => Some(unwrap_key_row(master, pid, &pe, "previous", pca, pea)?),
        None => None,
    };
    Ok(HmacKeyring { active, previous })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-26T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn sign_payload_is_deterministic_and_64_hex_chars() {
        let key = [7u8; HMAC_KEY_LEN];
        let a = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        let b = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn sign_payload_differs_per_path_and_per_ts() {
        let key = [7u8; HMAC_KEY_LEN];
        let base = sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/1");
        assert_ne!(base, sign_payload(&key, 1_700_000_000, "/v1/tile/15/1/2"));
        assert_ne!(base, sign_payload(&key, 1_700_000_001, "/v1/tile/15/1/1"));
    }

    #[test]
    fn new_key_id_is_ulid_shaped() {
        let id = new_key_id();
        assert_eq!(id.len(), 26);
        // ULID is Crockford base32 — only [0-9A-Z], no I/L/O/U.
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn generate_key_bytes_is_random() {
        let a = generate_key_bytes();
        let b = generate_key_bytes();
        assert_ne!(a, b, "thread_rng must not repeat 32 bytes in two calls");
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let master = [42u8; MASTER_KEK_LEN];
        let key = generate_key_bytes();
        let env = encrypt_key_bytes(&master, &key).unwrap();
        assert_eq!(env.len(), NONCE_LEN + HMAC_KEY_LEN + 16);
        let dec = decrypt_key_bytes(&master, &env).unwrap();
        assert_eq!(dec, key.to_vec());
    }

    #[test]
    fn decrypt_with_wrong_master_fails() {
        let master = [42u8; MASTER_KEK_LEN];
        let wrong = [43u8; MASTER_KEK_LEN];
        let env = encrypt_key_bytes(&master, &[1u8; HMAC_KEY_LEN]).unwrap();
        assert!(decrypt_key_bytes(&wrong, &env).is_err());
    }

    #[test]
    fn keyring_verify_with_active_key() {
        let now = fixed_now();
        let key = HmacKey {
            key_id: new_key_id(),
            bytes: Arc::new(generate_key_bytes()),
            status: KeyStatus::Current,
            created_at: now,
            expires_at: now + Duration::days(90),
        };
        let kid = key.key_id.clone();
        let key_bytes = *key.bytes;
        let ring = HmacKeyring::single(key);

        let ts = 1_700_000_000u64;
        let path = "/v1/tile/15/1/1";
        let sig = sign_payload(&key_bytes, ts, path);
        assert!(ring.verify_signature(&kid, ts, path, &sig, now).is_ok());
    }

    #[test]
    fn keyring_verify_with_previous_key_during_overlap() {
        let now = fixed_now();
        let ring_after_rotate = {
            let initial = HmacKey {
                key_id: new_key_id(),
                bytes: Arc::new(generate_key_bytes()),
                status: KeyStatus::Current,
                created_at: now,
                expires_at: now + Duration::days(90),
            };
            HmacKeyring::single(initial).rotate(now + Duration::days(90))
        };
        let prev = ring_after_rotate.previous.as_ref().unwrap();

        // Signing with the (now-demoted) previous key still verifies
        // because we're inside the 14-day overlap window.
        let ts = 1_700_000_000u64;
        let path = "/v1/tile/15/1/1";
        let sig = sign_payload(prev.bytes.as_ref(), ts, path);
        let check_at = now + Duration::days(95); // 5 days into overlap window
        assert!(ring_after_rotate
            .verify_signature(&prev.key_id, ts, path, &sig, check_at)
            .is_ok());
    }

    #[test]
    fn keyring_rejects_previous_key_after_overlap_expires() {
        let now = fixed_now();
        let ring = {
            let initial = HmacKey {
                key_id: new_key_id(),
                bytes: Arc::new(generate_key_bytes()),
                status: KeyStatus::Current,
                created_at: now,
                expires_at: now + Duration::days(90),
            };
            HmacKeyring::single(initial).rotate(now + Duration::days(90))
        };
        let prev = ring.previous.as_ref().unwrap();

        let ts = 1_700_000_000u64;
        let path = "/v1/tile/15/1/1";
        let sig = sign_payload(prev.bytes.as_ref(), ts, path);
        // 20 days after rotation > 14-day overlap window.
        let too_late = now + Duration::days(90 + 20);
        let err = ring
            .verify_signature(&prev.key_id, ts, path, &sig, too_late)
            .unwrap_err();
        assert!(err.to_string().contains("unknown or retired"));
    }

    #[test]
    fn keyring_rejects_unknown_key_id() {
        let now = fixed_now();
        let ring = HmacKeyring::single(HmacKey {
            key_id: new_key_id(),
            bytes: Arc::new(generate_key_bytes()),
            status: KeyStatus::Current,
            created_at: now,
            expires_at: now + Duration::days(90),
        });
        let err = ring
            .verify_signature("does-not-exist", 0, "/p", "00", now)
            .unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }

    #[test]
    fn keyring_rejects_tampered_signature() {
        let now = fixed_now();
        let key = HmacKey {
            key_id: new_key_id(),
            bytes: Arc::new(generate_key_bytes()),
            status: KeyStatus::Current,
            created_at: now,
            expires_at: now + Duration::days(90),
        };
        let kid = key.key_id.clone();
        let key_bytes = *key.bytes;
        let ring = HmacKeyring::single(key);

        let ts = 1_700_000_000u64;
        let path = "/v1/tile/15/1/1";
        let mut sig = sign_payload(&key_bytes, ts, path);
        // Flip a hex digit.
        sig.replace_range(0..1, if &sig[0..1] == "0" { "1" } else { "0" });
        let err = ring
            .verify_signature(&kid, ts, path, &sig, now)
            .unwrap_err();
        assert!(err.to_string().contains("bad signature"));
    }

    #[test]
    fn rotate_produces_fresh_active_and_demotes_previous() {
        let now = fixed_now();
        let initial = HmacKey {
            key_id: "ORIGINAL_ID".to_string(),
            bytes: Arc::new([9u8; HMAC_KEY_LEN]),
            status: KeyStatus::Current,
            created_at: now,
            expires_at: now + Duration::days(90),
        };
        let ring = HmacKeyring::single(initial);
        let rotated = ring.rotate(now + Duration::days(90));

        assert_ne!(rotated.active.key_id, "ORIGINAL_ID");
        assert_eq!(rotated.active.status, KeyStatus::Current);
        let prev = rotated.previous.as_ref().unwrap();
        assert_eq!(prev.key_id, "ORIGINAL_ID");
        assert_eq!(prev.status, KeyStatus::Previous);
        assert_eq!(
            prev.expires_at,
            now + Duration::days(90) + Duration::days(OVERLAP_WINDOW_DAYS)
        );
    }

    #[test]
    fn rotate_drops_stale_previous_when_rotating_again() {
        let now = fixed_now();
        let initial = HmacKey {
            key_id: new_key_id(),
            bytes: Arc::new(generate_key_bytes()),
            status: KeyStatus::Current,
            created_at: now,
            expires_at: now + Duration::days(90),
        };
        let r1 = HmacKeyring::single(initial).rotate(now + Duration::days(90));
        let first_prev_id = r1.previous.as_ref().unwrap().key_id.clone();
        let r2 = r1.rotate(now + Duration::days(180));

        // After a second rotation the "previous" is the *first* rotation's
        // active key, not the original (which is now multi-step retired).
        assert_ne!(r2.previous.as_ref().unwrap().key_id, first_prev_id);
    }

    #[test]
    fn unwrap_key_row_roundtrip() {
        let master = [11u8; MASTER_KEK_LEN];
        let raw = generate_key_bytes();
        let env = encrypt_key_bytes(&master, &raw).unwrap();
        let now = fixed_now();
        let key = unwrap_key_row(
            &master,
            "01H...".into(),
            &env,
            "current",
            now,
            now + Duration::days(90),
        )
        .unwrap();
        assert_eq!(key.bytes.as_ref(), &raw);
        assert_eq!(key.status, KeyStatus::Current);
    }

    #[test]
    fn keyring_from_rows_builds_two_key_keyring() {
        let master = [13u8; MASTER_KEK_LEN];
        let now = fixed_now();
        let cur_bytes = generate_key_bytes();
        let prev_bytes = generate_key_bytes();
        let cur_env = encrypt_key_bytes(&master, &cur_bytes).unwrap();
        let prev_env = encrypt_key_bytes(&master, &prev_bytes).unwrap();

        let ring = keyring_from_rows(
            &master,
            ("cur".into(), cur_env, now, now + Duration::days(90)),
            Some((
                "prev".into(),
                prev_env,
                now - Duration::days(90),
                now + Duration::days(14),
            )),
        )
        .unwrap();

        assert_eq!(ring.active.key_id, "cur");
        assert_eq!(ring.previous.as_ref().unwrap().key_id, "prev");
        // Both verify their own signatures.
        let ts = 1u64;
        let p = "/t";
        let csig = sign_payload(ring.active.bytes.as_ref(), ts, p);
        assert!(ring.verify_signature("cur", ts, p, &csig, now).is_ok());
        let psig = sign_payload(ring.previous.as_ref().unwrap().bytes.as_ref(), ts, p);
        assert!(ring.verify_signature("prev", ts, p, &psig, now).is_ok());
    }

    #[test]
    fn load_or_init_master_key_creates_mode_600() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("master.key");
        let k1 = load_or_init_master_key(&path).unwrap();
        assert_eq!(k1.len(), MASTER_KEK_LEN);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "master key must be mode 0600");
        // Loading again returns the same bytes (idempotent).
        let k2 = load_or_init_master_key(&path).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn key_status_str_roundtrip() {
        for s in &[KeyStatus::Current, KeyStatus::Previous, KeyStatus::Retired] {
            assert_eq!(KeyStatus::parse(s.as_str()).unwrap(), *s);
        }
        assert!(KeyStatus::parse("bogus").is_err());
    }
}
