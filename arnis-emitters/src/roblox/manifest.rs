//! Roblox-specific manifest types. Mirrors the JSON Schema in
//! `schema/manifest.v1.0.json` — keep them in sync.

use arnis_core::emitter::TileCoord;
use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: &str = "1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobloxManifest {
    pub manifest_version: String, // "1.0"
    pub style_version: u32,
    pub tile: TileCoord,
    pub stud_scale: f32,        // studs per metre (Q038 = 2.0)
    pub center_wgs84: [f64; 2], // [lat, lon]
    pub region_key: String,

    #[serde(default)]
    pub buildings: Vec<BuildingEntry>,
    #[serde(default)]
    pub roads: Vec<RoadEntry>,
    #[serde(default)]
    pub water: Vec<WaterEntry>,
    #[serde(default)]
    pub landmarks: Vec<LandmarkEntry>,
    #[serde(default)]
    pub assets: Vec<AssetRef>,

    pub terrain: Option<TerrainGrid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingEntry {
    pub osm_id: String,
    /// Outer ring footprint in studs, local tile coordinates.
    pub footprint_studs: Vec<[f32; 2]>,
    pub height_studs: f32,
    pub wall_colour_hex: String,
    pub roof_colour_hex: String,
    pub category: String,
    pub claimable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadEntry {
    pub osm_id: String,
    pub polyline_studs: Vec<[f32; 2]>,
    pub width_studs: f32,
    pub material: String,
    pub class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaterEntry {
    pub osm_id: String,
    pub polygon_studs: Vec<[f32; 2]>,
    pub depth_studs: f32,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LandmarkEntry {
    pub osm_id: String,
    pub position_studs: [f32; 2],
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRef {
    pub key: String,
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerrainGrid {
    pub width: u16,
    pub height: u16,
    pub tile_extent_studs: f32,
    /// Heights in studs, row-major (`width * height` entries).
    pub heights_studs: Vec<f32>,
}
