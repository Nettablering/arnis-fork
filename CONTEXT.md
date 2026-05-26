# Worldbuilders — Domain Glossary

Single source of truth for terminology. Updated inline as decisions crystallise during grilling.
No implementation details here — only the language we speak.

---

## Core entities

**Tile** — A square region of the real world, addressed by slippy-map `(z, x, y)`. Geometry inside
a tile is derived from `(location, OSM snapshot, style version)` and is never persisted per player.
Default size ~200–500 m square. Identified by tile id; not the same as a Plot.

**Plot** — A Tile that a specific player has chosen as theirs. The same Tile can be a Plot for
multiple players (each gets their own overlay), but one player has one active Plot at a time.
*(Open: can a Plot be smaller than a Tile? Can a Tile be sub-divided between rival players?)*

**OSM id** — Stable identifier for a real-world feature (`osm:way/12345`, `osm:relation/N`).
The anchor that lets an overlay row survive geometry regeneration.

**Overlay** — The persisted gameplay state for one (player × tile) pair. Keyed by OSM ids.
Holds claimed buildings, levels, accrued earnings, raid state. Geometry is NOT part of overlay.

**Claimed building** — A building (OSM way/relation) that a player owns within their Plot.
Earns idle income at `earnRate`, can be upgraded to `level`, can be raided.

## Verbs on a Tile (progressive drip-unlock — see [[q02-verb-stack]])

**Claim** — Mark an OSM building as yours; starts idle income.
**Upgrade** — Spend currency to multiply a Claimed building's `earnRate` and bump its visual tier.
**Renovate** — Per-building cosmetic overlay (palette, decor). Soft-currency, no gameplay effect.
**Build** — Place new objects (non-OSM) on your Plot; free-form within a grid. Persisted as
build-overlay rows.
**Defend** — Configure passive defences against incoming Raids. Anchored to the Plot, not to
individual Claimed buildings.
**Raid** — Asynchronously attack another player's Plot to steal a % of accrued earnings.
Async means no simultaneous-online requirement.
**Transform** — Re-skin the entire Tile with a Style (cyberpunk, lava, cottagecore, …).
Prestige-gated or premium-currency-gated. Cosmetic across all buildings on the Tile.

**Tile manifest** — The JSON payload served by `GET /v1/tile/{z}/{x}/{y}` containing all
derived geometry for a tile (terrain heightmap, building footprints, roads, water).

**Style version** — Monotonic integer identifying the visual style (palette, height rules).
Part of the cache key; bumping it invalidates baked tiles but NOT overlays.

**OSM snapshot** — Pinned timestamp of the OSM extract used to bake a tile. Persisted overlays
are anchored to a specific snapshot so a re-bake against newer OSM data doesn't orphan claims.

---

## Modes & loops

**Anchor loop** — `tap-to-claim → idle earn → upgrade → collect → repeat`. The one loop that
owns the first 5 seconds and last 5 seconds of every session. Legible without tutorial.

**Build mode** — Player-driven construction on top of (or replacing) baked OSM geometry within
their Plot. Persisted as build-overlay rows; not the same as Claimed buildings.

**Exploration** — Walking the real-world geometry of your own or others' Plots; rewards finds,
fuels collectables, drives discovery of mini-games.

**Mini-game** — Bounded arcade-style sub-experience. Scope TBD (see [[q01-core-loop]]):
attached to a Claimed building, global arcade lobby, or PvP head-to-head.

Anchor loop is the spine; Build / Exploration / Mini-game are sideloops that all feed the
same currency and the same plot.

## Open / unresolved terms

*(Resolved during grilling — moved up to the entity list above.)*

- "Raid" — what exactly happens, who can initiate, what's at stake
- "Rebirth" / "prestige" — does this game have one, and what carries over
- "Meteor shower" — the synchronized server-coordinated event analogue
- "Steal a Brainrot's theft mechanic" — the betrayal/social-tension equivalent
- "5-second core loop" — the literal first action a new mobile player takes
- "Idle / passive income" — how it binds to OSM buildings (per-building? per-tile?)
- "Plot selection" — does a player pick a real-world address, or get assigned one
- "Empty tile" — what gameplay fills a player's hometown when OSM data is sparse

---

## Attribution

OSM data is © OpenStreetMap contributors, ODbL. Every tile manifest carries `attribution`;
the client must display it visibly in-game.

---

## Bake-queue (Q081)

- **bake-queue** — Redis-Streams transport between bake-server (producer on cache miss) and
  arnis-cli worker pool (consumers). Crate: `backend/bake-queue/`.
- **wb:bake.requests.cold** — User-driven cold-tile-miss stream. Must hit the Q084 SLA. The
  bake-server XADDs here on cache miss; workers XREADGROUP-poll cold first.
- **wb:bake.requests.hot** — Preheat-cron stream (low priority). Workers drain only when cold
  is empty.
- **wb:bake.requests.dead** — Dead-letter stream for entries that exceeded retry budget. Owned
  by the future wb-stream-watchdog process.
- **bake-workers** — Canonical consumer group on both lane streams.
- **wb-bake-worker** — Standalone worker binary shipped by `backend/bake-queue/`. Also the
  binary `arnis-cli --consume-stream` re-execs. Exits cleanly on SIGTERM after the in-flight
  bake finishes.
- **Idempotency key** — `bake:done:<tile_id>:v<style_version>:<osm_snapshot>`. SET on success
  with a 24 h TTL; subsequent enqueues and deliveries short-circuit.

---

## Cache eviction (Q082)

- **Layered cache** — Three-tier manifest cache inside `bake-server`. Lookups try PINNED →
  HOT → COLD; misses fall through to the bake-queue producer (Q081).
- **PINNED tile** — Ops-promoted tile that never evicts. Held in an in-process map plus
  mirrored into the Redis sorted-set `wb:cache.hot.tiles` so the Q083 preheat cron can
  rebake them on a schedule.
- **HOT tier** — In-memory LRU (default 10 000 entries). Promotion happens automatically on
  cold-disk hit; eviction is plain LRU.
- **COLD tier** — On-disk `<z>-<x>-<y>.json` manifests (the existing Q465 cache directory)
  with a 30-day soft TTL — stale files are treated as misses so the worker re-bakes them.
- **`wb:cache.hot.tiles`** — Redis sorted-set scored by pin-time-unix; the contract between
  `bake-server` (writes via pin/unpin) and the Q083 preheat cron (reads via
  `ZRANGEBYSCORE -inf +inf`). The cache module never enqueues bakes itself; that boundary
  belongs to Q083.
- **`X-WB-Cache-Tier`** — Response header on `GET /v1/tile/*` that names the serving tier
  (`pinned` / `hot` / `cold`). Surfaces tier behaviour to clients + tests.
- **Pin endpoint** — `POST /v1/admin/cache/pin/{z}/{x}/{y}` promotes a tile to PINNED;
  `DELETE /v1/admin/cache/pin/{z}/{x}/{y}` demotes it back to HOT. Both authenticate with
  the same `x-wb-ts` + `x-wb-sig` HMAC scheme as the public tile endpoint.
- **`GET /v1/admin/cache/stats`** — Counters (hits per tier, evictions, TTL expirations) +
  the current list of pinned tile-ids.

## Q086 — OSM to PostGIS pipeline (additions)

- **osm2pgsql --flex** — Import pipeline producing the canonical
  `osm.planet_osm_*` tables. Configured via `backend/osm-postgis-source/sql/wb-flex.lua`.
  Tables are dropped and re-created on every `osm-import.sh` run; the Q085 GIST/GIN/BRIN
  indexes are re-applied from `sql/reindex.sql`.
- **Daily diff** — Incremental OSM update from Geofabrik's per-region updates stream
  applied via `osm2pgsql-replication`. Driven by
  `backend/scripts/osm-daily-update.sh`; the slim middle tables live in schema
  `osm_slim` so they don't clutter the read-side `osm` schema.
- **No-Overpass-on-hot-path rule** — Bake-service queries PostGIS only via the
  `OsmSource` trait's `PostGisSource` impl (`backend/osm-postgis-source`). The
  `OverpassSource` impl stays in the binary only for dev/debug and the quarterly
  cross-check; never on the bake hot path.
- **`OsmSource` trait** — Single-method async trait
  (`fetch_bbox(bbox) -> Vec<OsmElement>`) with two impls (`OverpassSource`,
  `PostGisSource`). Both feed the same `classify()` function so manifest output is
  byte-identical regardless of backend (proved by
  `backend/osm-postgis-source/tests/equivalence.rs`).
