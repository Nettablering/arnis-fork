-- V0003: wb.* gameplay state.
--
-- Maps the task spec onto the grill doc decision:
--   task `tiles`     -> wb.tiles_cold     (composite tile_id 'z/x/y', bbox, manifest jsonb)
--   task `manifests` -> wb.tiles_cold.manifest (jsonb column, TOAST-backed)
--   task `overlays`  -> wb.overlays       (per universe+user+tile, mirrors Roblox DataStore)
--   task `players`   -> wb.players        (universe_id + roblox user_id + first/last_seen)
--
-- Partitioning of wb.tiles_cold by RANGE(baked_at) is implemented here as a
-- regular table to keep test fixtures simple; production switches to range
-- partitions via a follow-up migration (tracked in roadmap; see grill doc).
-- This is the conservative path so the migration is idempotent against an
-- empty testcontainers Postgres.

-- Universes (Roblox places that talk to bake-server).
CREATE TABLE IF NOT EXISTS wb.universes (
    universe_id      bigint PRIMARY KEY,
    display_name     text,
    api_key_hash     bytea,
    rate_limit_rpm   int  NOT NULL DEFAULT 500,
    premium          boolean NOT NULL DEFAULT false,
    created_at       timestamptz NOT NULL DEFAULT now(),
    hmac_key_current bytea NOT NULL,
    hmac_key_prev    bytea,
    hmac_rotated_at  timestamptz NOT NULL DEFAULT now()
);

-- Players (per-universe Roblox user records).
CREATE TABLE IF NOT EXISTS wb.players (
    universe_id    bigint NOT NULL REFERENCES wb.universes(universe_id) ON DELETE CASCADE,
    user_id        bigint NOT NULL,                 -- roblox_user_id
    active_plot    text,
    first_seen     timestamptz NOT NULL DEFAULT now(),
    last_seen      timestamptz NOT NULL DEFAULT now(),
    country_hint   text,
    PRIMARY KEY (universe_id, user_id)
);

CREATE INDEX IF NOT EXISTS players_last_seen_idx
    ON wb.players (last_seen DESC);

-- Cold-baked tile manifests. Tile id is the composite 'z/x/y' text PK.
-- Bbox is stored as polygon(EPSG:4326) so we can bbox-match for retention sweeps.
CREATE TABLE IF NOT EXISTS wb.tiles_cold (
    tile_id          text PRIMARY KEY,        -- 'z/x/y'
    z                smallint NOT NULL,
    x                int NOT NULL,
    y                int NOT NULL,
    style_version    int  NOT NULL,
    osm_snapshot     date NOT NULL,
    osm_snapshot_ts  timestamptz NOT NULL DEFAULT now(),
    manifest         jsonb NOT NULL,           -- TOAST-backed
    manifest_hash    bytea NOT NULL,           -- sha256 of canonical manifest bytes
    baked_at         timestamptz NOT NULL DEFAULT now(),
    last_baked       timestamptz NOT NULL DEFAULT now(),
    baked_in_ms      int,
    bbox             geometry(Polygon, 4326),
    size_bytes       int,
    attribution      text NOT NULL DEFAULT 'OpenStreetMap contributors',
    -- Sanity: composite (z,x,y) must agree with tile_id.
    CONSTRAINT tiles_cold_zxy_chk CHECK (tile_id = z::text || '/' || x::text || '/' || y::text)
);

CREATE INDEX IF NOT EXISTS tiles_cold_bbox_gist
    ON wb.tiles_cold USING gist (bbox);
CREATE INDEX IF NOT EXISTS tiles_cold_baked_at_idx
    ON wb.tiles_cold (baked_at DESC);
CREATE INDEX IF NOT EXISTS tiles_cold_z_idx
    ON wb.tiles_cold (z);

-- Hot tile registry (curated megacities, landmarks, capitals, ops-pinned).
CREATE TABLE IF NOT EXISTS wb.hot_tile_registry (
    tile_id       text PRIMARY KEY,
    reason        text NOT NULL CHECK (reason IN
        ('megacity','landmark','capital','roblox-density','ops')),
    score         real NOT NULL,
    pinned_until  timestamptz,
    added_by      text NOT NULL,
    added_at      timestamptz NOT NULL DEFAULT now()
);

-- Overlays mirror the Roblox DataStore per (universe, user, tile).
-- Same row covers claimed buildings, raids incoming, build overlay, defense.
CREATE TABLE IF NOT EXISTS wb.overlays (
    universe_id     bigint NOT NULL,
    user_id         bigint NOT NULL,
    tile_id         text   NOT NULL REFERENCES wb.tiles_cold(tile_id) ON DELETE CASCADE,
    osm_snapshot    date   NOT NULL,
    claimed         jsonb  NOT NULL DEFAULT '{}'::jsonb,
    build_overlay   jsonb  NOT NULL DEFAULT '[]'::jsonb,
    defense         jsonb  NOT NULL DEFAULT '{}'::jsonb,
    raids_incoming  jsonb  NOT NULL DEFAULT '[]'::jsonb,
    version         bigint NOT NULL DEFAULT 1,
    updated_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (universe_id, user_id, tile_id),
    FOREIGN KEY (universe_id, user_id)
        REFERENCES wb.players (universe_id, user_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS overlays_tile_idx
    ON wb.overlays (tile_id);
CREATE INDEX IF NOT EXISTS overlays_updated_at_idx
    ON wb.overlays (updated_at DESC);
CREATE INDEX IF NOT EXISTS overlays_claimed_gin
    ON wb.overlays USING gin (claimed);

-- Bake job queue (status state machine for the worker pool).
CREATE TABLE IF NOT EXISTS wb.bake_jobs (
    bake_id      uuid PRIMARY KEY,             -- ULID stored as uuid for portability
    tile_id      text NOT NULL,
    status       text NOT NULL CHECK (status IN ('queued','running','done','failed')),
    enqueued_at  timestamptz NOT NULL DEFAULT now(),
    started_at   timestamptz,
    done_at      timestamptz,
    error        text,
    retry_count  int NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS bake_jobs_status_idx
    ON wb.bake_jobs (status, enqueued_at);
CREATE INDEX IF NOT EXISTS bake_jobs_tile_idx
    ON wb.bake_jobs (tile_id);
