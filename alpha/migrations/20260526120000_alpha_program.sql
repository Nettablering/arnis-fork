-- V0010: Pre-launch alpha-tester program (Q243).
--
-- Adds two tables to the existing `wb.*` schema (Q085):
--   wb.applicants    — every application submitted via /alpha
--   wb.invite_codes  — minted 12-char base32 invite codes + redemption state
--
-- These join the existing wb-db schema:
--   * wb.applicants.invite_code (nullable) -> wb.invite_codes.code
--   * wb.invite_codes.redeemed_by_user_id -> wb.players(user_id) (logical, not FK,
--     because players are universe-scoped; we only know roblox_user_id here)
--
-- The schema is idempotent; safe to re-apply against an empty testcontainers PG.

CREATE TABLE IF NOT EXISTS wb.applicants (
    applicant_id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name              text NOT NULL,
    email                     citext,
    country                   text NOT NULL,                 -- ISO 3166-1 alpha-2
    age_band                  text NOT NULL,                 -- enum mirror of form schema
    parental_consent          boolean NOT NULL DEFAULT false,
    primary_device            text NOT NULL,
    roblox_username           text NOT NULL,
    weekly_hours_available    text NOT NULL,
    willing_to_record_video   boolean NOT NULL DEFAULT false,
    favourite_roblox_games    jsonb,
    anything_else             text,
    referral_source           text,
    consent_pre_launch_emails boolean NOT NULL,
    consent_nda_acknowledged  boolean NOT NULL DEFAULT false,
    application_payload       jsonb,                         -- raw submission for audit
    received_at               timestamptz NOT NULL DEFAULT now(),
    -- Triage state, driven by operator + churn-monitor:
    status                    text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','invited','redeemed','rejected','withdrawn')),
    invite_code               text,                          -- FK target below
    invited_at                timestamptz,
    rejected_at               timestamptz,
    rejection_reason          text
);

-- citext + pgcrypto guard (idempotent against the wb schema's existing extensions).
CREATE EXTENSION IF NOT EXISTS citext;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE INDEX IF NOT EXISTS applicants_status_idx
    ON wb.applicants (status, received_at DESC);
CREATE INDEX IF NOT EXISTS applicants_country_idx
    ON wb.applicants (country);
CREATE UNIQUE INDEX IF NOT EXISTS applicants_email_unique_idx
    ON wb.applicants (email) WHERE email IS NOT NULL;

CREATE TABLE IF NOT EXISTS wb.invite_codes (
    code                  text PRIMARY KEY,            -- 12-char base32 (Crockford)
    batch                 text NOT NULL,               -- e.g. 'alpha-wave-1'
    stage                 smallint NOT NULL            -- 1 = alpha, 2 = closed beta, 3 = soft launch
        CHECK (stage IN (1,2,3)),
    created_at            timestamptz NOT NULL DEFAULT now(),
    expires_at            timestamptz,                 -- nullable = no expiry
    redeemed_at           timestamptz,
    redeemed_by_user_id   bigint,                      -- roblox user id; logical link only
    redeemed_by_applicant uuid REFERENCES wb.applicants(applicant_id),
    revoked_at            timestamptz,
    revoke_reason         text,
    notes                 text
);

CREATE INDEX IF NOT EXISTS invite_codes_batch_idx
    ON wb.invite_codes (batch, stage);
CREATE INDEX IF NOT EXISTS invite_codes_redeemed_idx
    ON wb.invite_codes (redeemed_at) WHERE redeemed_at IS NOT NULL;

-- Add FK from applicants.invite_code now that invite_codes exists.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'applicants_invite_code_fk'
    ) THEN
        ALTER TABLE wb.applicants
            ADD CONSTRAINT applicants_invite_code_fk
            FOREIGN KEY (invite_code) REFERENCES wb.invite_codes(code)
            ON DELETE SET NULL;
    END IF;
END $$;

COMMENT ON TABLE wb.applicants    IS 'Pre-launch alpha applicants (Q243). One row per /alpha submission.';
COMMENT ON TABLE wb.invite_codes  IS 'Minted invite codes (Q243 + Q247). 12-char base32; staged across alpha / closed beta / soft launch.';
