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
    /// Q211 + Q013 + Q030: composite landmark rarity in `[0, 1]`. Driven
    /// by Wikipedia pageviews (via `IngestedBuilding::pageview_rarity`)
    /// plus per-factor blend. Omitted from JSON when None so plain
    /// buildings (no Wikidata link) keep the existing manifest shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity_score: Option<f32>,
    /// Q013 rarity tier label derived from `rarity_score` — kept in the
    /// manifest so the Roblox client doesn't have to duplicate the
    /// thresholds. Omitted when `rarity_score` is None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rarity_tier: Option<String>,
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
    /// Wikidata fact-pack (Q210). `None` when no `wikidata=Q*` tag was
    /// present or fetch failed; serialised away in that case so existing
    /// v1.0 baselines without enrichment remain byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment: Option<Enrichment>,
}

/// Wikidata-sourced fact-pack attached to a landmark (Q210). Additive
/// to the manifest contract: all fields optional, omitted when absent,
/// so this is a v1.0 minor add per Q102.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Enrichment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikidata_qid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub founding_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub founder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heritage_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official_website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub located_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_wgs84: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commemorates: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fictional_appearances: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub named_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_stories: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from_chain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
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
