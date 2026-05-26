//! Overpass JSON → [`IngestedTile`] for the Roblox emitter (Q465).
//!
//! Parses the response shape produced by `[out:json]; ... out body geom;` —
//! `way` elements with inline `geometry` arrays, plus their `tags`. This is
//! the minimal slice needed to land the first end-to-end vertical (Q465).
//! Heavier OSM features (multipolygons, nested relations, land cover, etc.)
//! are handled by the existing `arnis-cli` pipeline and will be migrated
//! into `arnis-core` over subsequent tickets.
//!
//! The output of this module is an engine-neutral [`IngestedTile`] which
//! the [`crate::roblox::RobloxEmitter`] consumes unchanged.

use arnis_core::emitter::{
    IngestedBuilding, IngestedRoad, IngestedTile, IngestedWater, LatLonRing, TileBbox, TileCoord,
};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum OverpassError {
    #[error("invalid Overpass JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid Overpass response: {0}")]
    Shape(String),
}

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    #[serde(default)]
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Deserialize)]
struct OverpassElement {
    #[serde(rename = "type")]
    el_type: String,
    #[serde(default)]
    id: u64,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    geometry: Option<Vec<LatLon>>,
}

#[derive(Debug, Deserialize)]
struct LatLon {
    lat: f64,
    lon: f64,
}

/// Compute a slippy-map tile coord (z, x, y) from a WGS84 (lat, lon).
pub fn slippy_tile_for(lat_deg: f64, lon_deg: f64, z: u8) -> TileCoord {
    let n = 2f64.powi(z as i32);
    let x = ((lon_deg + 180.0) / 360.0 * n).floor() as u32;
    let lat_rad = lat_deg.to_radians();
    let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
        .floor() as u32;
    TileCoord { z, x, y }
}

/// Return the WGS84 bbox of a slippy tile.
pub fn slippy_tile_bbox(coord: TileCoord) -> TileBbox {
    let n = 2f64.powi(coord.z as i32);
    let lon_w = coord.x as f64 / n * 360.0 - 180.0;
    let lon_e = (coord.x as f64 + 1.0) / n * 360.0 - 180.0;
    let lat_n_rad = (std::f64::consts::PI * (1.0 - 2.0 * coord.y as f64 / n))
        .sinh()
        .atan();
    let lat_s_rad = (std::f64::consts::PI * (1.0 - 2.0 * (coord.y as f64 + 1.0) / n))
        .sinh()
        .atan();
    TileBbox {
        south_lat: lat_s_rad.to_degrees(),
        west_lon: lon_w,
        north_lat: lat_n_rad.to_degrees(),
        east_lon: lon_e,
    }
}

/// Parse an Overpass JSON dump into an [`IngestedTile`] for the given
/// slippy-map tile coordinate.
///
/// The tile bbox is reconstructed from `coord`, NOT from the Overpass
/// query bbox; this keeps the manifest authoritative even when the
/// fetcher queried a tighter or looser bbox (e.g. radius-based).
pub fn ingest_overpass(raw: &[u8], coord: TileCoord) -> Result<IngestedTile, OverpassError> {
    let resp: OverpassResponse = serde_json::from_slice(raw)?;
    let bbox = slippy_tile_bbox(coord);

    let mut buildings = Vec::new();
    let mut roads = Vec::new();
    let mut water = Vec::new();

    for el in resp.elements {
        if el.el_type != "way" {
            // Q465 vertical slice: ignore relations + nodes for now.
            // Multipolygon relations are tracked for a future ticket.
            continue;
        }
        let geom = match el.geometry {
            Some(g) if g.len() >= 2 => g,
            _ => continue,
        };
        let ring: LatLonRing = geom.iter().map(|p| [p.lat, p.lon]).collect();
        let osm_id = format!("way/{}", el.id);

        if el.tags.contains_key("building") {
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
            if ring.len() < 3 {
                continue;
            }
            buildings.push(IngestedBuilding {
                osm_id,
                footprint: ring,
                height_m,
                levels,
                building_kind: kind,
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

    Ok(IngestedTile {
        coord,
        bbox,
        region_key: None,
        buildings,
        roads,
        water,
        heightmap: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slippy_round_trip_aksla() {
        // Aksla (Ålesund) at zoom 16: known tile (33864, 18347).
        let coord = slippy_tile_for(62.4720, 6.1500, 16);
        assert_eq!(coord.z, 16);
        let bbox = slippy_tile_bbox(coord);
        assert!(bbox.south_lat <= 62.4720 && 62.4720 <= bbox.north_lat);
        assert!(bbox.west_lon <= 6.1500 && 6.1500 <= bbox.east_lon);
    }

    #[test]
    fn parses_minimal_way() {
        let raw = br#"{"elements":[
            {"type":"way","id":1,"tags":{"building":"house"},
             "geometry":[{"lat":62.472,"lon":6.150},{"lat":62.473,"lon":6.150},
                         {"lat":62.473,"lon":6.151},{"lat":62.472,"lon":6.151}]}
        ]}"#;
        let coord = slippy_tile_for(62.4720, 6.1500, 16);
        let tile = ingest_overpass(raw, coord).unwrap();
        assert_eq!(tile.buildings.len(), 1);
        assert_eq!(tile.buildings[0].osm_id, "way/1");
        assert_eq!(tile.buildings[0].building_kind.as_deref(), Some("house"));
    }

    #[test]
    fn classifies_highway_and_water() {
        let raw = br#"{"elements":[
            {"type":"way","id":2,"tags":{"highway":"residential","lanes":"2"},
             "geometry":[{"lat":62.472,"lon":6.150},{"lat":62.473,"lon":6.151}]},
            {"type":"way","id":3,"tags":{"natural":"water"},
             "geometry":[{"lat":62.472,"lon":6.150},{"lat":62.473,"lon":6.150},{"lat":62.473,"lon":6.151}]}
        ]}"#;
        let coord = slippy_tile_for(62.4720, 6.1500, 16);
        let tile = ingest_overpass(raw, coord).unwrap();
        assert_eq!(tile.roads.len(), 1);
        assert_eq!(tile.roads[0].highway_class, "residential");
        assert_eq!(tile.roads[0].lanes, Some(2));
        assert_eq!(tile.water.len(), 1);
        assert_eq!(tile.water[0].kind, "water");
    }
}
