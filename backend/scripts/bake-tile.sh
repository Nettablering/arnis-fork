#!/usr/bin/env bash
# Q465 + Q086 — emit a schema-valid Roblox tile manifest.
#
# By default this script picks PostGIS when the worldbuilders DB has the
# osm.* tables populated (Q086 happy path) and falls back to the cached
# Overpass JSON otherwise. Override with WB_SOURCE=overpass|postgis.
#
# Either backend is deterministic; the equivalence test in
# `backend/osm-postgis-source/tests/equivalence.rs` proves a single OSM
# snapshot produces byte-identical manifests under both sources.
#
# Idempotent: re-running over the same input produces byte-identical output.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
Z="${WB_Z:-16}"

decide_source() {
    if [[ -n "${WB_SOURCE:-}" ]]; then
        echo "${WB_SOURCE}"; return
    fi
    # Auto-pick: postgis if DB has rows; overpass otherwise.
    if [[ -s "${REPO_ROOT}/db/.env" ]]; then
        # shellcheck disable=SC1090
        local url
        url="$(grep '^DATABASE_URL=' "${REPO_ROOT}/db/.env" | head -1 | cut -d= -f2-)"
        if [[ -n "${url}" ]] && \
           command -v psql >/dev/null 2>&1 && \
           [[ "$(psql -tA "${url}" -c 'SELECT count(*) > 0 FROM osm.planet_osm_polygon LIMIT 1' 2>/dev/null)" = "t" ]]; then
            echo "postgis"; return
        fi
    fi
    echo "overpass"
}

SOURCE="$(decide_source)"
echo "[bake-tile] source=${SOURCE} (override with WB_SOURCE=overpass|postgis)"

if [[ "${SOURCE}" = "overpass" ]]; then
    LAST_ENV="${REPO_ROOT}/cache/overpass/last.env"
    if [[ ! -s "${LAST_ENV}" ]]; then
        echo "[bake-tile] missing ${LAST_ENV} — run fetch-overpass.sh first" >&2
        exit 1
    fi
    # shellcheck disable=SC1090
    source "${LAST_ENV}"

    BAKE_BIN="${REPO_ROOT}/arnis/target/release/examples/bake-tile"
    if [[ ! -x "${BAKE_BIN}" ]]; then
        echo "[bake-tile] building bake-tile example…"
        (cd "${REPO_ROOT}/arnis" && cargo build -p arnis-emitters --example bake-tile --features cli --release)
    fi
else
    # PostGIS path requires DATABASE_URL exported.
    set -a
    # shellcheck disable=SC1090
    source "${REPO_ROOT}/db/.env"
    set +a
    : "${WB_LAT:=62.4720}"
    : "${WB_LON:=6.1500}"

    BAKE_BIN="${REPO_ROOT}/osm-postgis-source/target/release/bake-postgis-tile"
    if [[ ! -x "${BAKE_BIN}" ]]; then
        echo "[bake-tile] building bake-postgis-tile…"
        (cd "${REPO_ROOT}/osm-postgis-source" && cargo build --bin bake-postgis-tile --features cli --release)
    fi
fi

MANIFEST_DIR="${REPO_ROOT}/cache/manifests"
mkdir -p "${MANIFEST_DIR}"

# Compute slippy x/y so we know the cache filename in advance.
read X Y < <(python3 - <<PY
import math
lat, lon, z = ${WB_LAT}, ${WB_LON}, ${Z}
n = 2 ** z
x = int((lon + 180.0) / 360.0 * n)
lat_rad = math.radians(lat)
y = int((1.0 - math.log(math.tan(lat_rad) + 1.0 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
print(x, y)
PY
)

OUT="${MANIFEST_DIR}/${Z}-${X}-${Y}.json"

echo "[bake-tile] z=${Z} x=${X} y=${Y} -> ${OUT}"

if [[ "${SOURCE}" = "overpass" ]]; then
    "${BAKE_BIN}" \
        --input "${WB_OVERPASS_JSON}" \
        --center-lat "${WB_LAT}" \
        --center-lon "${WB_LON}" \
        --z "${Z}" \
        --output "${OUT}"
else
    "${BAKE_BIN}" \
        --source postgis \
        --center-lat "${WB_LAT}" \
        --center-lon "${WB_LON}" \
        --z "${Z}" \
        --output "${OUT}"
fi

echo "[bake-tile] wrote $(wc -c <"${OUT}") bytes"
{
    echo "WB_Z=${Z}"
    echo "WB_X=${X}"
    echo "WB_Y=${Y}"
    echo "WB_MANIFEST=${OUT}"
    echo "WB_SOURCE=${SOURCE}"
} > "${REPO_ROOT}/cache/manifests/last.env"
