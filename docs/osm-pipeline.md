# OSM → PostGIS pipeline (Q086)

Operational doc for the OSM ingest pipeline that feeds the bake-service.
Decision rationale lives in
[`docs/grill/q086-overpass-self-host-vs-postgis-extract.md`](grill/q086-overpass-self-host-vs-postgis-extract.md);
this file is the day-to-day operator guide.

## Components

| Component | Path | Role |
|-----------|------|------|
| `osm-import.sh` | `backend/scripts/osm-import.sh` | One-shot regional import (default: Ålesund bbox out of `norway-latest.osm.pbf`). |
| `osm-daily-update.sh` | `backend/scripts/osm-daily-update.sh` | Cron-driven `osm2pgsql-replication` daily diff. |
| `wb-flex.lua` | `backend/osm-postgis-source/sql/wb-flex.lua` | osm2pgsql flex style that mirrors the Q085 `osm.planet_osm_*` schema. |
| `reindex.sql` | `backend/osm-postgis-source/sql/reindex.sql` | Re-apply Q085 GIST/GIN/BRIN + `osm.features` view after import (osm2pgsql recreates tables and loses these). |
| `osm-postgis-source` crate | `backend/osm-postgis-source/` | Rust `OsmSource` trait + `OverpassSource` + `PostGisSource` impls. |
| `bake-postgis-tile` binary | `backend/osm-postgis-source/src/bin/bake_postgis_tile.rs` | CLI that drives either backend (`--source overpass|postgis`). |
| `bake-tile.sh` | `backend/scripts/bake-tile.sh` | Wrapper. Defaults to PostGIS if `osm.planet_osm_polygon` is populated; otherwise falls back to the cached Overpass JSON. |

## Initial import

Prereqs:
* `osm2pgsql` ≥ 1.5 (`apt install osm2pgsql`)
* `osmium-tool` (`apt install osmium-tool`)
* `backend/db/.env` produced by `backend/scripts/db-create.sh` (Q085)

Run:
```bash
bash backend/scripts/osm-import.sh
```

Defaults: download `europe/norway-latest.osm.pbf` (~1.3 GB) into
`backend/cache/osm-pbf/`, slice the Ålesund bbox
(`5.8,62.30,6.55,62.60`) via `osmium extract`, then `osm2pgsql --create
--slim --output=flex`. Final step re-applies the Q085 indexes.

Tunables (env vars):

| Var | Default | Notes |
|-----|---------|-------|
| `WB_OSM_PBF` | _unset_ | Use a local PBF instead of downloading. |
| `WB_OSM_REGION` | `europe/norway` | Geofabrik path prefix. |
| `WB_OSM_BBOX` | `5.8,62.30,6.55,62.60` | osmium extract bbox (`lon,lat,lon,lat`). |
| `WB_OSM_FULL_REGION` | `0` | Set `1` to skip bbox slicing and import the whole country. |

Smoke counts after a fresh Ålesund-bbox import (2026-05-26):
```
polygon=53866  line=22367  point=44465
```

## Daily updates

The slim middle tables (created under schema `osm_slim` by `--slim
--middle-schema`) carry osm2pgsql-replication state. After the initial
import, schedule:

```cron
2 0 * * *  /home/deploy/projects/worldbuilders/backend/scripts/osm-daily-update.sh
```

The script:
1. Auto-initialises replication state from
   `https://download.geofabrik.de/${WB_OSM_REGION}-updates/` if not
   already initialised.
2. Applies daily diffs idempotently (osm2pgsql-replication tracks the
   last applied sequence).
3. Retries up to 5× with exponential backoff on transient failures (see
   the Q086 grill doc edge-case table).
4. Re-runs `reindex.sql` to ANALYZE + ensure indexes survive any DDL
   changes osm2pgsql may have made.

## OsmSource abstraction

The bake-pipeline talks to OSM through a single trait:

```rust
#[async_trait]
pub trait OsmSource {
    async fn fetch_bbox(&self, bbox: Bbox) -> Result<Vec<OsmElement>, SourceError>;
    fn name(&self) -> &'static str;
}
```

Two impls:

* **`OverpassSource::from_path(json)`** — reads a cached Overpass JSON
  dump. Used by the dev/debug fallback, the cross-check, and the
  integration test fixture.
* **`PostGisSource::new(pool)`** — issues one parameterised SQL per
  bbox, UNION across polygon/line/point. Used by the bake hot path.

Both impls feed the same `classify()` function, which produces the
engine-neutral `IngestedTile`. The integration test
`backend/osm-postgis-source/tests/equivalence.rs` asserts that — given
the same three OSM rows — both backends produce a byte-equal manifest.

## arnis-cli / bake-tile integration

`bake-tile.sh` chooses a backend:

```bash
# Auto-pick: postgis when osm.planet_osm_polygon has rows, else overpass.
bash backend/scripts/bake-tile.sh

# Force:
WB_SOURCE=postgis bash backend/scripts/bake-tile.sh
WB_SOURCE=overpass bash backend/scripts/bake-tile.sh
```

The CLI surface for the Rust binary mirrors the example from the grill
doc:

```bash
bake-postgis-tile --source postgis \
    --center-lat 62.4720 --center-lon 6.1500 --z 16 \
    --output cache/manifests/16-33887-18095.json
```

`--database-url` may be passed explicitly; otherwise the binary reads
`$DATABASE_URL` (loaded from `backend/db/.env`).

## Quarterly Overpass cross-check (Q086 §"Why we still keep a link")

Sample 100 random tiles, run them through both `--source=postgis` and
`--source=overpass` (after refetching live Overpass for each bbox), diff
the resulting manifests. Drift ≤ 2 % is expected (Overpass uses
minutely diffs, PostGIS uses daily). > 2 % drift is a signal that the
Lua flex script needs an audit — see the grill doc edge cases for the
exact cases to look for (deprecated tag schemes, partition
misrouting, etc.).

## Failure modes (recap from Q086 grill)

* **Initial import OOMs.** Set `--cache 100` (already the default in
  `osm-import.sh`) and raise `maintenance_work_mem` for the import.
* **Daily diff repeatedly fails.** The script retries 5×; on permanent
  failure the bake-service keeps serving stale OSM rather than
  breaking. Alert + investigate.
* **osm2pgsql column-layout upgrade.** Pin the apt version explicitly
  in any future container image; a major upgrade is a planned
  re-import.
* **Limit-hit (50k features for a dense bbox).** Bake-service should
  inspect the row count; if `== 50000` it must subdivide the bbox and
  re-query. Logged as a warning.
