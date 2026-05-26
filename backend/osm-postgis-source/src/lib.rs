//! Q086 — OSM data source abstraction.
//!
//! The bake-pipeline used to call the public Overpass API directly. Per the
//! Q086 grill decision (PostGIS extract via `osm2pgsql --flex`, never
//! self-host Overpass) the bake hot path now queries our local PostGIS
//! `osm.*` tables. Overpass is kept for dev/debug only.
//!
//! This crate provides a single trait — [`OsmSource`] — with two impls:
//!
//! * [`OverpassSource`] — wraps a cached or live Overpass JSON dump, the
//!   same shape Q465 originally consumed. Used for the dev fallback.
//! * [`PostGisSource`]  — issues one parameterised SQL per bbox against
//!   the `osm.planet_osm_{polygon,line,point}` tables defined by Q085.
//!
//! Both impls return [`OsmElement`] values which are then passed through a
//! shared classifier into the engine-neutral
//! [`arnis_core::emitter::IngestedTile`]. That guarantees the manifest is
//! identical regardless of which source produced the rows.

use std::collections::BTreeMap;

use arnis_core::emitter::{
    IngestedBuilding, IngestedRoad, IngestedTile, IngestedWater, LatLonRing, TileBbox, TileCoord,
};

pub use arnis_emitters::overpass_ingest::{slippy_tile_bbox, slippy_tile_for};

pub mod overpass;
pub mod postgis;

pub use overpass::OverpassSource;
pub use postgis::PostGisSource;

/// Tile bbox in WGS84 degrees. Wraps [`TileBbox`] for ergonomics — the
/// extra newtype keeps `fetch_bbox(bbox)` self-documenting at call sites.
#[derive(Debug, Clone, Copy)]
pub struct Bbox {
    pub south_lat: f64,
    pub west_lon: f64,
    pub north_lat: f64,
    pub east_lon: f64,
}

impl From<TileBbox> for Bbox {
    fn from(b: TileBbox) -> Self {
        Self {
            south_lat: b.south_lat,
            west_lon: b.west_lon,
            north_lat: b.north_lat,
            east_lon: b.east_lon,
        }
    }
}

/// One OSM way or polygon, projected to lat/lon. Shape is the union of what
/// Overpass returns (inline `geometry`) and what our PostGIS view returns
/// (`ST_AsText` decoded into the same lat-lon ring), so a single classifier
/// can process both.
#[derive(Debug, Clone)]
pub struct OsmElement {
    pub osm_id: String,
    /// `way` for line/closed-line; `relation` for multipolygons; `node`
    /// for points.
    pub kind: ElementKind,
    pub tags: BTreeMap<String, String>,
    /// Outer ring (or polyline). For `Node`, length 1.
    pub geometry: LatLonRing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Node,
    Way,
    Relation,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("postgis error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("overpass parse error: {0}")]
    Overpass(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// The contract every OSM backend implements. Keep this narrow: a bbox is
/// the only fan-out point the bake-service needs.
#[async_trait::async_trait]
pub trait OsmSource: Send + Sync {
    /// Return all building / highway / water elements intersecting `bbox`.
    /// Implementations should NOT cap results silently — Q086 calls out
    /// the 50k-buildings warning case explicitly.
    async fn fetch_bbox(&self, bbox: Bbox) -> Result<Vec<OsmElement>, SourceError>;

    /// Identifier surfaced in manifest provenance / log lines.
    fn name(&self) -> &'static str;
}

/// Convert a stream of [`OsmElement`] into the engine-neutral
/// [`IngestedTile`] consumed by every emitter. This is the *single* place
/// classification happens, so OverpassSource and PostGisSource produce
/// byte-identical manifests for the same bbox.
pub fn classify(elements: Vec<OsmElement>, coord: TileCoord) -> IngestedTile {
    let bbox = slippy_tile_bbox(coord);
    let mut buildings = Vec::new();
    let mut roads = Vec::new();
    let mut water = Vec::new();

    for el in elements {
        let ring = el.geometry;
        let prefix = match el.kind {
            ElementKind::Way => "way",
            ElementKind::Relation => "relation",
            ElementKind::Node => "node",
        };
        let osm_id = format!("{prefix}/{}", el.osm_id);

        if el.tags.contains_key("building") {
            if ring.len() < 3 {
                continue;
            }
            let height_m = el
                .tags
                .get("height")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<f32>().ok());
            let levels = el
                .tags
                .get("building:levels")
                .and_then(|v| v.parse::<u16>().ok());
            let kind = el
                .tags
                .get("building")
                .map(|s| s.to_ascii_lowercase())
                .filter(|s| s != "yes");
            buildings.push(IngestedBuilding {
                osm_id,
                footprint: ring,
                height_m,
                levels,
                building_kind: kind,
                ..Default::default()
            });
        } else if let Some(hw) = el.tags.get("highway") {
            if ring.len() < 2 {
                continue;
            }
            let lanes = el.tags.get("lanes").and_then(|v| v.parse::<u8>().ok());
            roads.push(IngestedRoad {
                osm_id,
                polyline: ring,
                highway_class: hw.to_ascii_lowercase(),
                lanes,
            });
        } else if el.tags.get("natural").map(String::as_str) == Some("water")
            || el.tags.contains_key("waterway")
        {
            if ring.len() < 3 {
                continue;
            }
            let kind = el
                .tags
                .get("natural")
                .or_else(|| el.tags.get("waterway"))
                .cloned()
                .unwrap_or_else(|| "water".to_string());
            water.push(IngestedWater {
                osm_id,
                polygon: ring,
                kind,
            });
        }
    }

    // Stable ordering so the two backends' outputs hash identically.
    buildings.sort_by(|a, b| a.osm_id.cmp(&b.osm_id));
    roads.sort_by(|a, b| a.osm_id.cmp(&b.osm_id));
    water.sort_by(|a, b| a.osm_id.cmp(&b.osm_id));

    IngestedTile {
        coord,
        bbox,
        region_key: None,
        buildings,
        roads,
        water,
        heightmap: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(kind: ElementKind, id: &str, ring: LatLonRing, tags: &[(&str, &str)]) -> OsmElement {
        OsmElement {
            osm_id: id.to_string(),
            kind,
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            geometry: ring,
        }
    }

    #[test]
    fn classify_building_road_water() {
        let coord = slippy_tile_for(62.4720, 6.1500, 16);
        let elements = vec![
            el(
                ElementKind::Way,
                "1",
                vec![
                    [62.472, 6.150],
                    [62.473, 6.150],
                    [62.473, 6.151],
                    [62.472, 6.151],
                ],
                &[("building", "house"), ("building:levels", "2")],
            ),
            el(
                ElementKind::Way,
                "2",
                vec![[62.472, 6.150], [62.473, 6.151]],
                &[("highway", "residential"), ("lanes", "2")],
            ),
            el(
                ElementKind::Way,
                "3",
                vec![[62.472, 6.150], [62.473, 6.150], [62.473, 6.151]],
                &[("natural", "water")],
            ),
        ];
        let tile = classify(elements, coord);
        assert_eq!(tile.buildings.len(), 1);
        assert_eq!(tile.buildings[0].levels, Some(2));
        assert_eq!(tile.roads.len(), 1);
        assert_eq!(tile.roads[0].lanes, Some(2));
        assert_eq!(tile.water.len(), 1);
        assert_eq!(tile.water[0].kind, "water");
    }
}
