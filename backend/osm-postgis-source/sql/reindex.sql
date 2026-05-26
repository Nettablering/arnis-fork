-- Re-apply the Q085 indexes after an osm2pgsql import recreated the tables.
-- Run idempotently from backend/scripts/osm-import.sh.

CREATE INDEX IF NOT EXISTS planet_osm_polygon_way_gist
    ON osm.planet_osm_polygon USING gist (way);
CREATE INDEX IF NOT EXISTS planet_osm_polygon_way_brin
    ON osm.planet_osm_polygon USING brin (way);
CREATE INDEX IF NOT EXISTS planet_osm_polygon_tags_gin
    ON osm.planet_osm_polygon USING gin (tags);
CREATE INDEX IF NOT EXISTS planet_osm_polygon_building_partial
    ON osm.planet_osm_polygon USING gist (way)
    WHERE tags ? 'building';

CREATE INDEX IF NOT EXISTS planet_osm_line_way_gist
    ON osm.planet_osm_line USING gist (way);
CREATE INDEX IF NOT EXISTS planet_osm_line_way_brin
    ON osm.planet_osm_line USING brin (way);
CREATE INDEX IF NOT EXISTS planet_osm_line_tags_gin
    ON osm.planet_osm_line USING gin (tags);

CREATE INDEX IF NOT EXISTS planet_osm_point_way_gist
    ON osm.planet_osm_point USING gist (way);
CREATE INDEX IF NOT EXISTS planet_osm_point_tags_gin
    ON osm.planet_osm_point USING gin (tags);

CREATE INDEX IF NOT EXISTS planet_osm_rels_tags_gin
    ON osm.planet_osm_rels USING gin (tags);

CREATE OR REPLACE VIEW osm.features AS
    SELECT osm_id, 'polygon'::text AS osm_type, tags, way::geometry AS geom
        FROM osm.planet_osm_polygon
    UNION ALL
    SELECT osm_id, 'line'::text,    tags, way::geometry FROM osm.planet_osm_line
    UNION ALL
    SELECT osm_id, 'point'::text,   tags, way::geometry FROM osm.planet_osm_point;

ANALYZE osm.planet_osm_polygon;
ANALYZE osm.planet_osm_line;
ANALYZE osm.planet_osm_point;
