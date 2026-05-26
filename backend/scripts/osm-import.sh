#!/usr/bin/env bash
# Q086 — import a regional OSM extract into the worldbuilders PostGIS DB.
#
# Default region: Møre og Romsdal (covers Ålesund — our Q465 reference tile).
# Geofabrik publishes a ~30–60 MB .osm.pbf per Norwegian county; small enough
# to import on the CX43 in a couple of minutes, big enough to exercise the
# full bake pipeline end-to-end without Overpass.
#
# Idempotent: a second invocation drops the osm.* feature tables, re-applies
# the migration shape via osm2pgsql --create, then re-runs the Q085 indexes.
# osm2pgsql --slim middle tables are kept in the `osm_slim` schema so daily
# diffs (see osm-daily-update.sh) can keep applying.
#
# Usage:
#   ./backend/scripts/osm-import.sh                                    # default region
#   WB_OSM_PBF=/path/to/file.osm.pbf ./backend/scripts/osm-import.sh   # local fixture
#   WB_OSM_REGION=norway/oslo ./backend/scripts/osm-import.sh          # different Geofabrik region

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LUA_SCRIPT="${REPO_ROOT}/osm-postgis-source/sql/wb-flex.lua"
REINDEX_SQL="${REPO_ROOT}/osm-postgis-source/sql/reindex.sql"
CACHE_DIR="${REPO_ROOT}/cache/osm-pbf"
mkdir -p "${CACHE_DIR}"

# Load DB credentials produced by db-create.sh.
ENV_FILE="${REPO_ROOT}/db/.env"
if [[ ! -s "${ENV_FILE}" ]]; then
    echo "[osm-import] ${ENV_FILE} missing — run backend/scripts/db-create.sh first" >&2
    exit 1
fi
# shellcheck disable=SC1090
set -a; source "${ENV_FILE}"; set +a

if ! command -v osm2pgsql >/dev/null 2>&1; then
    echo "[osm-import] osm2pgsql not installed — see BLOCKED/needs-human.md" >&2
    exit 2
fi

# Resolve PBF path.
if [[ -n "${WB_OSM_PBF:-}" ]]; then
    PBF_PATH="${WB_OSM_PBF}"
    if [[ ! -s "${PBF_PATH}" ]]; then
        echo "[osm-import] WB_OSM_PBF=${PBF_PATH} not found" >&2; exit 1
    fi
else
    # Geofabrik publishes one extract per country for Norway (no county
    # subdivisions). Default behaviour: download `norway-latest.osm.pbf`
    # and use `osmium extract` to slice the Ålesund bbox so the import
    # stays small (a few MB) and matches the Q465 reference tile.
    REGION="${WB_OSM_REGION:-europe/norway}"
    URL="https://download.geofabrik.de/${REGION}-latest.osm.pbf"
    BASE_PBF="${CACHE_DIR}/$(basename "${REGION}")-latest.osm.pbf"
    if [[ ! -s "${BASE_PBF}" ]]; then
        echo "[osm-import] downloading ${URL}"
        curl -fSL --retry 3 -o "${BASE_PBF}.tmp" "${URL}"
        mv "${BASE_PBF}.tmp" "${BASE_PBF}"
    else
        echo "[osm-import] using cached ${BASE_PBF}"
    fi

    # Slice Ålesund bbox unless the caller asks for the full country.
    if [[ "${WB_OSM_FULL_REGION:-0}" = "1" ]]; then
        PBF_PATH="${BASE_PBF}"
    else
        # Ålesund region bbox (Aksla viewpoint + surrounding islands).
        # left bottom right top, lon lat lon lat.
        BBOX="${WB_OSM_BBOX:-5.8,62.30,6.55,62.60}"
        PBF_PATH="${CACHE_DIR}/aksla-extract.osm.pbf"
        if [[ ! -s "${PBF_PATH}" || "${BASE_PBF}" -nt "${PBF_PATH}" ]]; then
            if ! command -v osmium >/dev/null 2>&1; then
                echo "[osm-import] osmium-tool not installed (needed for bbox extract)" >&2
                exit 2
            fi
            echo "[osm-import] osmium extract --bbox=${BBOX}"
            osmium extract --bbox "${BBOX}" \
                --overwrite \
                -o "${PBF_PATH}" \
                "${BASE_PBF}"
        else
            echo "[osm-import] using cached extract ${PBF_PATH}"
        fi
    fi
fi

SIZE=$(wc -c < "${PBF_PATH}")
echo "[osm-import] PBF: ${PBF_PATH} (${SIZE} bytes)"

# osm2pgsql --create wants an empty target. Drop our pre-created osm.* tables
# (the migration shape will be recreated by the Lua define_table calls) and
# clear any previous osm_slim middle.
echo "[osm-import] resetting target tables (DROP TABLE IF EXISTS …)"
psql -v ON_ERROR_STOP=1 "${DATABASE_URL}" <<'SQL'
DROP VIEW IF EXISTS osm.features CASCADE;
DROP TABLE IF EXISTS osm.planet_osm_polygon CASCADE;
DROP TABLE IF EXISTS osm.planet_osm_line    CASCADE;
DROP TABLE IF EXISTS osm.planet_osm_point   CASCADE;
DROP TABLE IF EXISTS osm.planet_osm_rels    CASCADE;
CREATE SCHEMA IF NOT EXISTS osm_slim;
SQL

# osm2pgsql DSN form: parse DATABASE_URL.
# DATABASE_URL=postgres://user:pass@host:port/db
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

# --slim keeps the middle tables so osm-daily-update.sh can --append diffs.
# --middle-schema isolates them under osm_slim so the osm.* schema stays clean.
# -C / cache: 200 MB is plenty for a county-sized extract.
echo "[osm-import] running osm2pgsql --create --output=flex"
osm2pgsql \
    --create \
    --slim \
    --output=flex \
    --style "${LUA_SCRIPT}" \
    --middle-schema osm_slim \
    --cache 200 \
    --number-processes 2 \
    --host "${DB_HOST}" \
    --port "${DB_PORT}" \
    --database "${DB_NAME}" \
    --user "${DB_USER}" \
    "${PBF_PATH}"

unset PGPASSWORD

echo "[osm-import] re-applying Q085 indexes + osm.features view"
psql -v ON_ERROR_STOP=1 "${DATABASE_URL}" -f "${REINDEX_SQL}"

# Quick smoke check.
N_POLY=$(psql -tA "${DATABASE_URL}" -c "SELECT count(*) FROM osm.planet_osm_polygon;")
N_LINE=$(psql -tA "${DATABASE_URL}" -c "SELECT count(*) FROM osm.planet_osm_line;")
N_POINT=$(psql -tA "${DATABASE_URL}" -c "SELECT count(*) FROM osm.planet_osm_point;")
echo "[osm-import] OK: polygon=${N_POLY} line=${N_LINE} point=${N_POINT}"
