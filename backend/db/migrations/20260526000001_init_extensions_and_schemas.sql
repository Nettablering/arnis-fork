-- V0001: PostGIS extensions + logical schemas.
--
-- Per docs/grill/q085-postgis-schema.md (accepted decision):
--   single PostGIS instance, TWO logical schemas:
--     osm  = planet OSM data via osm2pgsql (read-mostly, partitioned by continent)
--     wb   = Worldbuilders gameplay state (tiles, overlays, players, universes, bake_jobs)
--
-- Owned by role "worldbuilders" (created by scripts/db-create.sh).
-- NEVER reach into /srv/shared/ Postgres; this is a dedicated DB on 127.0.0.1:5432.

CREATE EXTENSION IF NOT EXISTS postgis;
-- pg_trgm helps tag-text lookups (e.g. building='yes' subselects)
CREATE EXTENSION IF NOT EXISTS pg_trgm;
-- btree_gist needed for some composite GIST indexes
CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE SCHEMA IF NOT EXISTS osm;
CREATE SCHEMA IF NOT EXISTS wb;

COMMENT ON SCHEMA osm IS 'Planet OSM data, populated by osm2pgsql. Read-mostly.';
COMMENT ON SCHEMA wb  IS 'Worldbuilders gameplay state. Read/write by bake-server + Roblox bridge.';
