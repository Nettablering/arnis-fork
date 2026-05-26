//! `wb-invite` — pre-launch alpha invite-code minting + redemption.
//!
//! Format: 12-char Crockford base32 (`0123456789ABCDEFGHJKMNPQRSTVWXYZ`,
//! no I/L/O/U). 60 bits of entropy => 1.15e18 codes; collisions
//! statistically impossible at the program's scale (5050 codes total).
//!
//! Storage: `wb.invite_codes` (see `../migrations/`). The crate is
//! transport-agnostic — the CLI in `src/bin/wb-invite.rs` shells out to
//! sqlx; library callers can hook into bake-server later.
//!
//! Per Q243 / Q247.

use anyhow::{anyhow, Context, Result};
use rand::RngCore;

/// Crockford base32 alphabet (no I/L/O/U; no padding). 5 bits per char.
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Length of an invite code in characters. 12 * 5 = 60 bits of entropy.
pub const CODE_LEN: usize = 12;

/// Generate a single random 12-char Crockford base32 code.
///
/// We don't pull `data_encoding`'s Spec machinery — for Crockford-without-padding
/// over a 12-char output the hand-rolled loop is shorter, has no API stability
/// surface and is trivially reviewable.
pub fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; CODE_LEN];
    rng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| CROCKFORD_ALPHABET[(*b as usize) % 32] as char)
        .collect()
}

/// Generate `n` unique random codes. Uniqueness inside the batch is
/// checked here; uniqueness against existing DB rows is enforced by the
/// PK constraint on `wb.invite_codes.code`.
pub fn generate_batch(n: usize) -> Vec<String> {
    let mut out = std::collections::HashSet::with_capacity(n);
    while out.len() < n {
        out.insert(generate_code());
    }
    out.into_iter().collect()
}

/// Validate that a candidate string looks like a Worldbuilders invite code.
/// Used by the CLI redeem path and the (future) bake-server redemption endpoint.
pub fn validate_shape(code: &str) -> Result<String> {
    let trimmed = code.trim().to_ascii_uppercase();
    if trimmed.len() != CODE_LEN {
        return Err(anyhow!(
            "invite code must be {CODE_LEN} characters; got {}",
            trimmed.len()
        ));
    }
    for ch in trimmed.chars() {
        if !CROCKFORD_ALPHABET.contains(&(ch as u8)) {
            return Err(anyhow!(
                "invite code contains invalid character '{ch}' (Crockford base32 only: 0-9 A-Z minus I/L/O/U)"
            ));
        }
    }
    Ok(trimmed)
}

/// Stage of the alpha programme an invite code grants entry to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Alpha = 1,
    ClosedBeta = 2,
    SoftLaunch = 3,
}

impl Stage {
    pub fn from_i16(v: i16) -> Result<Self> {
        match v {
            1 => Ok(Stage::Alpha),
            2 => Ok(Stage::ClosedBeta),
            3 => Ok(Stage::SoftLaunch),
            other => Err(anyhow!("unknown stage {other}; expected 1, 2 or 3")),
        }
    }
}

/// Re-export the migrations baked into this crate so integration tests and
/// the `wb-invite migrate` command can apply them against a throwaway DB.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../migrations");

/// Insert a fresh batch of codes against an open Postgres pool. Caller owns
/// the pool so this can be reused from the CLI, a bake-server admin handler,
/// or an integration test.
pub async fn persist_batch(
    pool: &sqlx::PgPool,
    codes: &[String],
    batch: &str,
    stage: Stage,
) -> Result<u64> {
    let stage_i16 = stage as i16;
    let mut tx = pool.begin().await.context("begin tx")?;
    let mut inserted = 0u64;
    for code in codes {
        let res = sqlx::query(
            "INSERT INTO wb.invite_codes (code, batch, stage)
             VALUES ($1, $2, $3)
             ON CONFLICT (code) DO NOTHING",
        )
        .bind(code)
        .bind(batch)
        .bind(stage_i16)
        .execute(&mut *tx)
        .await
        .context("insert invite code")?;
        inserted += res.rows_affected();
    }
    tx.commit().await.context("commit tx")?;
    Ok(inserted)
}

/// Redeem a code on behalf of a Roblox user id. Returns the stage granted.
/// Idempotent: redeeming the same (code, user) twice returns Ok with the
/// existing stage. Redeeming a code already redeemed by *another* user
/// returns an error.
pub async fn redeem_code(
    pool: &sqlx::PgPool,
    code: &str,
    roblox_user_id: i64,
) -> Result<Stage> {
    let code = validate_shape(code)?;
    let mut tx = pool.begin().await?;
    let row: Option<(i16, Option<chrono::DateTime<chrono::Utc>>, Option<i64>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT stage, redeemed_at, redeemed_by_user_id, revoked_at
         FROM wb.invite_codes WHERE code = $1 FOR UPDATE",
    )
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await?;

    let (stage, redeemed_at, redeemed_by, revoked_at) =
        row.ok_or_else(|| anyhow!("unknown invite code"))?;

    if revoked_at.is_some() {
        return Err(anyhow!("invite code revoked"));
    }
    if let Some(existing_user) = redeemed_by {
        if existing_user == roblox_user_id {
            return Stage::from_i16(stage);
        }
        return Err(anyhow!(
            "invite code already redeemed by another user (at {:?})",
            redeemed_at
        ));
    }

    sqlx::query(
        "UPDATE wb.invite_codes
         SET redeemed_at = now(), redeemed_by_user_id = $2
         WHERE code = $1",
    )
    .bind(&code)
    .bind(roblox_user_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Stage::from_i16(stage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_is_12_chars_crockford() {
        for _ in 0..1000 {
            let c = generate_code();
            assert_eq!(c.len(), CODE_LEN, "expected {CODE_LEN}-char code, got {c}");
            for ch in c.chars() {
                assert!(
                    CROCKFORD_ALPHABET.contains(&(ch as u8)),
                    "code {c} contains forbidden char {ch}",
                );
            }
        }
    }

    #[test]
    fn batch_is_unique() {
        let batch = generate_batch(500);
        let unique: std::collections::HashSet<_> = batch.iter().collect();
        assert_eq!(unique.len(), 500);
    }

    #[test]
    fn validate_shape_rejects_wrong_length() {
        assert!(validate_shape("ABC").is_err());
        assert!(validate_shape("ABCDEFGHJKMN0").is_err()); // 13 chars
    }

    #[test]
    fn validate_shape_rejects_forbidden_letters() {
        // I, L, O, U are not in Crockford base32.
        assert!(validate_shape("ILOU01234567").is_err());
    }

    #[test]
    fn validate_shape_normalises_case_and_whitespace() {
        let valid = generate_code().to_lowercase();
        let padded = format!("  {valid}  ");
        let normalised = validate_shape(&padded).unwrap();
        assert_eq!(normalised, valid.to_ascii_uppercase());
    }

    #[test]
    fn stage_round_trips() {
        for s in [Stage::Alpha, Stage::ClosedBeta, Stage::SoftLaunch] {
            let i = s as i16;
            assert_eq!(Stage::from_i16(i).unwrap(), s);
        }
        assert!(Stage::from_i16(0).is_err());
        assert!(Stage::from_i16(4).is_err());
    }

    #[test]
    fn distinct_calls_produce_distinct_codes() {
        // Probabilistic — 60 bits of entropy, collisions vanishingly rare.
        let a = generate_code();
        let b = generate_code();
        assert_ne!(a, b);
    }
}
