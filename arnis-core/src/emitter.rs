//! Emitter trait + ingestion contract.
//!
//! The trait is intentionally narrow: an emitter takes an
//! engine-agnostic [`IngestedTile`] (the upstream of which is Overpass +
//! land-cover + terrain pipelines) and writes engine-specific output to
//! `out_dir`, returning a manifest value that callers can serialise and
//! validate.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum EmitterError {
    #[error("schema validation failed: {0}")]
    SchemaInvalid(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Slippy-map tile coordinate. `z` is the zoom level (Q464 defaults to 15).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TileCoord {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// Geographic bounding box of a tile in WGS84 degrees.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TileBbox {
    pub south_lat: f64,
    pub west_lon: f64,
    pub north_lat: f64,
    pub east_lon: f64,
}

/// A 2D polygon ring in WGS84 degrees, outer-first.
pub type LatLonRing = Vec<[f64; 2]>;

/// An OSM way representing a building. Geometry is `lat_lon` ordered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestedBuilding {
    pub osm_id: String,
    pub footprint: LatLonRing,
    /// Explicit `height=*` in metres, if any.
    pub height_m: Option<f32>,
    /// Explicit `building:levels=*`, if any.
    pub levels: Option<u16>,
    /// Raw `building=*` value, lower-cased, e.g. "house", "apartments".
    pub building_kind: Option<String>,
    /// Wikidata QID resolved from `wikidata=*` tag (Q210). Used by Q211
    /// to look up pageview rarity for this landmark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikidata_qid: Option<String>,
    /// Pre-computed pageview rarity in `[0, 1]` (Q211). Sourced from the
    /// pageview cache; emitters blend it into the final rarity score.
    /// `None` means "no Wikipedia article known" → pageview term is 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pageview_rarity: Option<f32>,
}

/// An OSM way representing a road. Geometry is a polyline of lat/lon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestedRoad {
    pub osm_id: String,
    pub polyline: LatLonRing,
    /// Raw `highway=*` value, lower-cased.
    pub highway_class: String,
    /// Explicit `lanes=*`, if any.
    pub lanes: Option<u8>,
}

/// An OSM polygon representing surface water.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestedWater {
    pub osm_id: String,
    pub polygon: LatLonRing,
    /// `natural`, `waterway`, `landuse` etc. — the tag that classified it.
    pub kind: String,
}

/// A heightmap sample grid, row-major. `width * height == samples.len()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heightmap {
    pub width: u16,
    pub height: u16,
    /// Elevation in metres above MSL.
    pub samples: Vec<f32>,
}

/// Everything an emitter needs to render one tile. Engine-neutral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestedTile {
    pub coord: TileCoord,
    pub bbox: TileBbox,
    /// Region key derived from reverse-geocode + Köppen (Q049). May be
    /// `None`; emitters fall back to `default` palette.
    pub region_key: Option<String>,
    pub buildings: Vec<IngestedBuilding>,
    pub roads: Vec<IngestedRoad>,
    pub water: Vec<IngestedWater>,
    pub heightmap: Option<Heightmap>,
}

impl IngestedTile {
    /// Empty tile with just a coordinate + bbox set. Useful in tests and
    /// for early bring-up of emitters before the ingestion pipeline lands.
    pub fn empty(coord: TileCoord, bbox: TileBbox) -> Self {
        Self {
            coord,
            bbox,
            region_key: None,
            buildings: vec![],
            roads: vec![],
            water: vec![],
            heightmap: None,
        }
    }
}

/// Emitter contract. Implementations live in `arnis-emitters`.
pub trait Emitter {
    type Manifest: Serialize + for<'de> Deserialize<'de>;

    fn name(&self) -> &'static str;
    fn schema_version(&self) -> &'static str;

    fn emit(&self, tile: &IngestedTile, out_dir: &Path) -> Result<Self::Manifest, EmitterError>;

    fn validate(&self, manifest: &Self::Manifest) -> Result<(), EmitterError>;
}
