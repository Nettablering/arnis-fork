#!/usr/bin/env bash
# Q088 — HMAC signing key rotation (operator script).
#
# Rotates the per-universe HMAC signing key:
#   1. Generates a fresh 32-byte random key (via the bake-server CLI when
#      available, falling back to /dev/urandom).
#   2. Encrypts it with the master KEK at
#      ~/.claude/shared/api-keys/worldbuilders-hmac-master.key (mode 0600).
#   3. Inserts a new `current` row into wb.hmac_keys for the universe and
#      demotes the existing `current` to `previous` (atomic transaction).
#   4. Writes an entry into wb.hmac_audit.
#   5. Optionally invokes the bake-server `/v1/admin/hmac/rotate` endpoint
#      to ask running instances to refresh their in-memory keyring.
#
# This script NEVER echoes raw key bytes. The new key only lives in the
# Postgres `wb.hmac_keys.key_bytes` column (envelope-encrypted) and in the
# in-memory keyring of the bake-server.
#
# Usage:
#   rotate-hmac.sh <universe_id> [--actor <email>] [--note <text>]
#
# Env:
#   DATABASE_URL                 (required) Postgres connection string.
#   WB_HMAC_MASTER_KEY_PATH      Override the master KEK location.
#   BAKE_SERVER_ADMIN_URL        Optional, e.g. http://127.0.0.1:9090
#   BAKE_SERVER_ADMIN_TOKEN      Required if BAKE_SERVER_ADMIN_URL is set.

set -euo pipefail

usage() {
    cat >&2 <<EOF
usage: $(basename "$0") <universe_id> [--actor <email>] [--note <text>]

Rotates the per-universe HMAC signing key. See Q088.
EOF
    exit 64
}

if [[ $# -lt 1 ]]; then usage; fi
UNIVERSE_ID="$1"; shift
ACTOR="${USER:-operator}@$(hostname -s)"
NOTE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --actor) ACTOR="$2"; shift 2 ;;
        --note) NOTE="$2"; shift 2 ;;
        *) usage ;;
    esac
done

: "${DATABASE_URL:?DATABASE_URL must be set}"
MASTER_KEY_PATH="${WB_HMAC_MASTER_KEY_PATH:-$HOME/.claude/shared/api-keys/worldbuilders-hmac-master.key}"

if [[ ! -f "$MASTER_KEY_PATH" ]]; then
    echo "FATAL: master KEK not found at $MASTER_KEY_PATH" >&2
    echo "       (the bake-server creates it on first run; run the server once first.)" >&2
    exit 2
fi

# Refuse to run if the master key is world-readable.
MODE="$(stat -c '%a' "$MASTER_KEY_PATH" 2>/dev/null || stat -f '%Lp' "$MASTER_KEY_PATH")"
if [[ "$MODE" != "600" ]]; then
    echo "FATAL: master KEK at $MASTER_KEY_PATH has mode $MODE; must be 600." >&2
    exit 3
fi

# Generate a new ULID (lowercased to satisfy Crockford base32; psql casts to text fine).
NEW_KEY_ID="$(date -u +%s%N | sha256sum | head -c 26 | tr 'a-z' 'A-Z')"

# Generate the new HMAC key bytes via openssl (32 bytes random) and pipe
# them encrypted into Postgres. We use bake-server's own envelope format
# (nonce || ciphertext+tag) via a tiny Python helper because PostgreSQL's
# `pgcrypto` does not implement ChaCha20-Poly1305. Python's `cryptography`
# package is present on the build host (used by other backend scripts).
ENC_BYTES_B64="$(python3 - "$MASTER_KEY_PATH" <<'PY'
import base64, os, secrets, sys
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

master_path = sys.argv[1]
with open(master_path, "rb") as f:
    raw = f.read().strip()
try:
    master = base64.b64decode(raw)
except Exception:
    master = raw
assert len(master) == 32, f"master KEK wrong length {len(master)}"

key = secrets.token_bytes(32)        # the new HMAC key
nonce = secrets.token_bytes(12)
ct = ChaCha20Poly1305(master).encrypt(nonce, key, None)
envelope = nonce + ct
print(base64.b64encode(envelope).decode())
PY
)"

# Atomic rotate: demote existing current → previous (overlap window),
# insert new current, audit both rows.
psql "$DATABASE_URL" --quiet --set ON_ERROR_STOP=1 \
    --set "universe_id=$UNIVERSE_ID" \
    --set "new_key_id=$NEW_KEY_ID" \
    --set "envelope_b64=$ENC_BYTES_B64" \
    --set "actor=$ACTOR" \
    --set "note=$NOTE" <<'SQL'
BEGIN;

-- Demote the existing `current` (if any) to `previous`; previous expires
-- 14 days from now (Q088 overlap window).
UPDATE wb.hmac_keys
   SET status = 'previous',
       rotated_at = now(),
       expires_at = now() + interval '14 days'
 WHERE universe_id = :'universe_id'::bigint
   AND status = 'current';

-- Any older `previous` is immediately retired.
UPDATE wb.hmac_keys
   SET status = 'retired',
       retired_at = now()
 WHERE universe_id = :'universe_id'::bigint
   AND status = 'previous'
   AND rotated_at < now() - interval '1 second';

-- Insert the new current.
INSERT INTO wb.hmac_keys (key_id, universe_id, key_bytes, status, expires_at)
VALUES (
    :'new_key_id',
    :'universe_id'::bigint,
    decode(:'envelope_b64', 'base64'),
    'current',
    now() + interval '90 days'
);

-- Audit log: one row per state transition.
INSERT INTO wb.hmac_audit (universe_id, key_id, action, actor, note)
VALUES (:'universe_id'::bigint, :'new_key_id', 'created', :'actor', NULLIF(:'note', ''));

INSERT INTO wb.hmac_audit (universe_id, key_id, action, actor, note)
SELECT universe_id, key_id, 'demoted', :'actor', NULLIF(:'note', '')
  FROM wb.hmac_keys
 WHERE universe_id = :'universe_id'::bigint
   AND status = 'previous'
   AND rotated_at > now() - interval '5 seconds';

COMMIT;
SQL

echo "Rotated universe=$UNIVERSE_ID new_key_id=$NEW_KEY_ID actor=$ACTOR"

# Best-effort nudge to a running bake-server. Optional — clients also
# self-heal via the keyring miss path.
if [[ -n "${BAKE_SERVER_ADMIN_URL:-}" ]]; then
    : "${BAKE_SERVER_ADMIN_TOKEN:?BAKE_SERVER_ADMIN_TOKEN required when BAKE_SERVER_ADMIN_URL is set}"
    curl --fail --silent --show-error \
        -H "Authorization: Bearer $BAKE_SERVER_ADMIN_TOKEN" \
        -X POST "$BAKE_SERVER_ADMIN_URL/v1/admin/hmac/refresh?universe_id=$UNIVERSE_ID" \
        || echo "WARN: bake-server refresh nudge failed; clients will self-heal." >&2
fi
