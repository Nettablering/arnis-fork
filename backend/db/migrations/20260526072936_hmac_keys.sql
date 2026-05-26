-- V0004: Per-universe HMAC signing keys with rotation history (Q088).
--
-- Decision (q088-hmac-signing-key-rotation.md): per-universe HMAC-SHA256
-- key, rotated every 90 days, with a 14-day dual-key overlap window where
-- both `current` and `previous` keys are accepted. The signed payload
-- includes a `key_id` header so verifiers know which key to verify with.
--
-- This migration introduces the *history* table `wb.hmac_keys` (one row
-- per generated key version) and the `wb.hmac_audit` log. The Q085
-- `wb.universes.hmac_key_{current,prev}` columns continue to hold the
-- raw 32-byte key bytes (encrypted at rest) for the fast-path read; the
-- history table stores the same bytes plus metadata so we can answer
-- "which keys existed when?" for incident response. Both tables are
-- written to inside the same transaction by the rotation routine.

-- Per-universe HMAC key versions.
CREATE TABLE IF NOT EXISTS wb.hmac_keys (
    key_id       text         PRIMARY KEY,            -- ULID
    universe_id  bigint       NOT NULL REFERENCES wb.universes(universe_id) ON DELETE CASCADE,
    key_bytes    bytea        NOT NULL,               -- encrypted with master KEK
    status       text         NOT NULL CHECK (status IN ('current', 'previous', 'retired')),
    created_at   timestamptz  NOT NULL DEFAULT now(),
    rotated_at   timestamptz,                          -- when this key was demoted current → previous
    retired_at   timestamptz,                          -- when this key left the overlap window
    expires_at   timestamptz  NOT NULL,                -- created_at + 90d for current, + 14d after demote for previous
    UNIQUE (universe_id, key_id)
);

CREATE INDEX IF NOT EXISTS hmac_keys_universe_status_idx
    ON wb.hmac_keys (universe_id, status);
CREATE INDEX IF NOT EXISTS hmac_keys_expires_idx
    ON wb.hmac_keys (expires_at);

-- One `current` row per universe is enforced via a partial unique index:
-- the rotation routine MUST flip the old `current` to `previous` in the
-- same transaction it inserts the new `current`.
CREATE UNIQUE INDEX IF NOT EXISTS hmac_keys_one_current_per_universe
    ON wb.hmac_keys (universe_id)
    WHERE status = 'current';

-- Audit log: every rotation, retirement, or manual action.
CREATE TABLE IF NOT EXISTS wb.hmac_audit (
    audit_id     bigserial    PRIMARY KEY,
    universe_id  bigint       NOT NULL,
    key_id       text         NOT NULL,
    action       text         NOT NULL CHECK (action IN
        ('created','promoted','demoted','retired','manual_rotate','manual_revoke')),
    actor        text         NOT NULL,                -- 'cron', operator email, 'bootstrap'
    note         text,
    occurred_at  timestamptz  NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS hmac_audit_universe_idx
    ON wb.hmac_audit (universe_id, occurred_at DESC);
