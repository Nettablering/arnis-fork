-- V0002: osm.* feature tables.
--
-- These mirror the osm2pgsql default flex output shape, but we own them ourselves
-- so test environments (testcontainers) don't need a full planet import.
-- In production osm2pgsql will write into the SAME table names; we just preempt
-- creation so FKs and indexes exist before the first import.
--
-- The dominant query (see grill doc) is:
--   SELECT osm_id, tags, way
--   FROM osm.planet_osm_polygon
--   WHERE way && ST_MakeEnvelope($1,$2,$3,$4,4326)
--     AND tags ? 'building'
-- so GIST(way) is the critical index. BRIN(way) is a cheap supplement for cold
-- large-bbox scans.

-- Polygons: buildings, water bodies, parks, admin polys.
CREATE TABLE IF NOT EXISTS osm.planet_osm_polygon (
    osm_id      bigint PRIMARY KEY,
    tags        jsonb  NOT NULL DEFAULT '{}'::jsonb,
    way         geometry(Geometry, 4326) NOT NULL,
    z_order     int,
    way_area    real
);

CREATE INDEX IF NOT EXISTS planet_osm_polygon_way_gist
    ON osm.planet_osm_polygon USING gist (way);
CREATE INDEX IF NOT EXISTS planet_osm_polygon_way_brin
    ON osm.planet_osm_polygon USING brin (way);
CREATE INDEX IF NOT EXISTS planet_osm_polygon_tags_gin
    ON osm.planet_osm_polygon USING gin (tags);
-- Partial index for the dominant "buildings only" subquery.
CREATE INDEX IF NOT EXISTS planet_osm_polygon_building_partial
    ON osm.planet_osm_polygon USING gist (way)
    WHERE tags ? 'building';

-- Lines: roads, rivers, paths.
CREATE TABLE IF NOT EXISTS osm.planet_osm_line (
    osm_id      bigint PRIMARY KEY,
    tags        jsonb  NOT NULL DEFAULT '{}'::jsonb,
    way         geometry(Geometry, 4326) NOT NULL,
    z_order     int
);

CREATE INDEX IF NOT EXISTS planet_osm_line_way_gist
    ON osm.planet_osm_line USING gist (way);
CREATE INDEX IF NOT EXISTS planet_osm_line_way_brin
    ON osm.planet_osm_line USING brin (way);
CREATE INDEX IF NOT EXISTS planet_osm_line_tags_gin
    ON osm.planet_osm_line USING gin (tags);

-- Points: POIs, addresses.
CREATE TABLE IF NOT EXISTS osm.planet_osm_point (
    osm_id      bigint PRIMARY KEY,
    tags        jsonb  NOT NULL DEFAULT '{}'::jsonb,
    way         geometry(Point, 4326) NOT NULL
);

CREATE INDEX IF NOT EXISTS planet_osm_point_way_gist
    ON osm.planet_osm_point USING gist (way);
CREATE INDEX IF NOT EXISTS planet_osm_point_tags_gin
    ON osm.planet_osm_point USING gin (tags);

-- Relations metadata (rare query target; cheap to keep).
CREATE TABLE IF NOT EXISTS osm.planet_osm_rels (
    id          bigint PRIMARY KEY,
    way_off     smallint,
    rel_off     smallint,
    parts       bigint[],
    members     text[],
    tags        jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS planet_osm_rels_tags_gin
    ON osm.planet_osm_rels USING gin (tags);

-- Unified read-side view used by Q086 (overpass-self-host-vs-postgis-extract).
-- A bake-worker can issue one query against osm.features and union polygons+lines+points.
CREATE OR REPLACE VIEW osm.features AS
    SELECT osm_id, 'polygon'::text AS osm_type, tags, way::geometry AS geom
        FROM osm.planet_osm_polygon
    UNION ALL
    SELECT osm_id, 'line'::text,    tags, way::geometry FROM osm.planet_osm_line
    UNION ALL
    SELECT osm_id, 'point'::text,   tags, way::geometry FROM osm.planet_osm_point;

COMMENT ON VIEW osm.features IS 'Unified polygon+line+point view for tile-bbox extracts.';
