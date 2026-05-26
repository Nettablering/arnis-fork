#!/usr/bin/env bash
# Q086 — apply daily OSM diffs to the worldbuilders PostGIS DB.
#
# Uses osm2pgsql-replication (ships with osm2pgsql 1.5+) to track the diff
# stream for the region originally imported via osm-import.sh. The slim
# middle tables in schema `osm_slim` carry the replication state.
#
# First invocation initialises the replication state from the import file's
# timestamp. Subsequent invocations apply daily diffs idempotently.
#
# Usage (typical cron entry):
#   2 0 * * *  /home/deploy/projects/worldbuilders/backend/scripts/osm-daily-update.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LUA_SCRIPT="${REPO_ROOT}/osm-postgis-source/sql/wb-flex.lua"
REINDEX_SQL="${REPO_ROOT}/osm-postgis-source/sql/reindex.sql"

ENV_FILE="${REPO_ROOT}/db/.env"
if [[ ! -s "${ENV_FILE}" ]]; then
    echo "[osm-daily-update] ${ENV_FILE} missing — run backend/scripts/db-create.sh first" >&2
    exit 1
fi
# shellcheck disable=SC1090
set -a; source "${ENV_FILE}"; set +a

if ! command -v osm2pgsql-replication >/dev/null 2>&1; then
    echo "[osm-daily-update] osm2pgsql-replication not found — see BLOCKED/needs-human.md" >&2
    exit 2
fi

# Default replication source: Geofabrik's per-region updates stream. The URL
# is derived from WB_OSM_REGION (matches osm-import.sh default).
REGION="${WB_OSM_REGION:-europe/norway/more-og-romsdal}"
REPL_URL="${WB_OSM_REPL_URL:-https://download.geofabrik.de/${REGION}-updates/}"

DBURL_NO_SCHEME="${DATABASE_URL#postgres://}"
USERPASS="${DBURL_NO_SCHEME%@*}"
HOSTPORTDB="${DBURL_NO_SCHEME##*@}"
DB_USER="${USERPASS%%:*}"
DB_PASS="${USERPASS#*:}"
HOSTPORT="${HOSTPORTDB%/*}"
DB_NAME="${HOSTPORTDB##*/}"
DB_HOST="${HOSTPORT%:*}"
DB_PORT="${HOSTPORT#*:}"

export PGPASSWORD="${DB_PASS}"

PG_FLAGS=(
    --host "${DB_HOST}"
    --port "${DB_PORT}"
    --database "${DB_NAME}"
    --user "${DB_USER}"
)

# Init step is idempotent: re-running on an already-initialised slim middle
# is a no-op (status check first, then init only if needed).
if ! osm2pgsql-replication status --middle-schema osm_slim "${PG_FLAGS[@]}" >/dev/null 2>&1; then
    echo "[osm-daily-update] replication state not initialised — running init from ${REPL_URL}"
    osm2pgsql-replication init \
        --server "${REPL_URL}" \
        --middle-schema osm_slim \
        "${PG_FLAGS[@]}"
fi

echo "[osm-daily-update] applying diffs (max 5 retries w/ exponential backoff)"
ATTEMPT=0
while true; do
    if osm2pgsql-replication update \
            --middle-schema osm_slim \
            -- \
            --output=flex \
            --style "${LUA_SCRIPT}" \
            "${PG_FLAGS[@]}"; then
        break
    fi
    ATTEMPT=$((ATTEMPT + 1))
    if [[ "${ATTEMPT}" -ge 5 ]]; then
        echo "[osm-daily-update] giving up after 5 attempts — see Q086 grill doc" >&2
        unset PGPASSWORD
        exit 3
    fi
    SLEEP=$((2 ** ATTEMPT))
    echo "[osm-daily-update] retry ${ATTEMPT}/5 in ${SLEEP}s"
    sleep "${SLEEP}"
done

unset PGPASSWORD

# Diff application may have created new rows but won't refresh ANALYZE stats.
psql -v ON_ERROR_STOP=1 "${DATABASE_URL}" -f "${REINDEX_SQL}"

echo "[osm-daily-update] OK"
