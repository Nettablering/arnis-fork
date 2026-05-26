-- wb-flex.lua — osm2pgsql flex output that mirrors Q085 schema.
--
-- Routes ways/relations/nodes into osm.planet_osm_{polygon,line,point,rels}
-- with the exact columns Q085 migrations create. After import the
-- backend/scripts/osm-import.sh script re-applies the GIST/GIN/BRIN indexes
-- defined in 20260526000002_osm_features.sql so the bake hot-path queries
-- stay fast.
--
-- See docs/grill/q086-overpass-self-host-vs-postgis-extract.md.

local tables = {}

tables.polygon = osm2pgsql.define_table({
    name = 'planet_osm_polygon',
    schema = 'osm',
    ids = { type = 'area', id_column = 'osm_id' },
    columns = {
        { column = 'tags',     type = 'jsonb', not_null = true },
        { column = 'way',      type = 'geometry', projection = 4326, not_null = true },
        { column = 'z_order',  type = 'int' },
        { column = 'way_area', type = 'real' },
    },
})

tables.line = osm2pgsql.define_table({
    name = 'planet_osm_line',
    schema = 'osm',
    ids = { type = 'way', id_column = 'osm_id' },
    columns = {
        { column = 'tags',    type = 'jsonb', not_null = true },
        { column = 'way',     type = 'geometry', projection = 4326, not_null = true },
        { column = 'z_order', type = 'int' },
    },
})

tables.point = osm2pgsql.define_table({
    name = 'planet_osm_point',
    schema = 'osm',
    ids = { type = 'node', id_column = 'osm_id' },
    columns = {
        { column = 'tags', type = 'jsonb', not_null = true },
        { column = 'way',  type = 'point', projection = 4326, not_null = true },
    },
})

tables.rels = osm2pgsql.define_table({
    name = 'planet_osm_rels',
    schema = 'osm',
    ids = { type = 'relation', id_column = 'id' },
    columns = {
        { column = 'tags', type = 'jsonb', not_null = true },
    },
})

-- Keep only the tag keys the bake-service actually reads. Dropping the long
-- tail of source/note/attribution/etc keeps the toast table small.
local KEEP_KEYS = {
    ['building'] = true, ['building:levels'] = true, ['building:height'] = true,
    ['height'] = true, ['name'] = true,
    ['amenity'] = true, ['shop'] = true, ['tourism'] = true,
    ['natural'] = true, ['landuse'] = true, ['leisure'] = true,
    ['highway'] = true, ['lanes'] = true, ['maxspeed'] = true,
    ['waterway'] = true, ['wikidata'] = true, ['wikipedia'] = true,
    ['barrier'] = true, ['surface'] = true, ['service'] = true,
}

local function filter_tags(raw)
    local out = {}
    local any = false
    for k, v in pairs(raw) do
        if KEEP_KEYS[k] or k:sub(1, 5) == 'addr:' then
            out[k] = v
            any = true
        end
    end
    return out, any
end

local function is_polygon(tags)
    if tags.building or tags.landuse or tags.leisure or tags.amenity
       or tags['building:part'] or tags.tourism then
        return true
    end
    if tags.natural and (tags.natural == 'water' or tags.natural == 'wood'
                         or tags.natural == 'wetland' or tags.natural == 'beach'
                         or tags.natural == 'scrub' or tags.natural == 'grassland') then
        return true
    end
    return false
end

function osm2pgsql.process_node(object)
    local tags, any = filter_tags(object.tags)
    if not any then return end
    tables.point:insert({
        tags = tags,
        way  = object:as_point(),
    })
end

function osm2pgsql.process_way(object)
    local tags, any = filter_tags(object.tags)
    if not any then return end

    if object.is_closed and is_polygon(object.tags) then
        local geom = object:as_polygon()
        tables.polygon:insert({
            tags     = tags,
            way      = geom,
            z_order  = 0,
            way_area = geom:area(),
        })
    else
        if object.tags.highway or object.tags.waterway or object.tags.barrier then
            tables.line:insert({
                tags    = tags,
                way     = object:as_linestring(),
                z_order = 0,
            })
        end
    end
end

function osm2pgsql.process_relation(object)
    local tags, any = filter_tags(object.tags)
    if not any then return end
    -- Multipolygon → polygon table
    if object.tags.type == 'multipolygon' or object.tags.boundary or object.tags.building then
        local ok, geom = pcall(function() return object:as_multipolygon() end)
        if ok and geom and not geom:is_null() then
            tables.polygon:insert({
                tags     = tags,
                way      = geom,
                z_order  = 0,
                way_area = geom:area(),
            })
        end
    end
    tables.rels:insert({ tags = tags })
end
