//! Roblox JSON-manifest emitter (Q464).
//!
//! Takes an engine-neutral [`IngestedTile`] from `arnis-core` and produces
//! a `RobloxManifest` plus an on-disk `manifest.json` file. The manifest
//! is validated against a compile-time-embedded JSON Schema; rendering on
//! the Roblox side reconstructs geometry from this manifest using
//! `EditableMesh` per the design doc.
//!
//! The pipeline is intentionally pure: no Overpass calls, no network,
//! no global state. All randomness is replaced by deterministic FNV-1a
//! hashes of `osm_id` so re-baking a tile twice yields byte-identical
//! manifests (this matters for Q464 verification + downstream caching).

pub mod heuristics;
pub mod manifest;
pub mod palette;
pub mod rarity;
pub mod schema;

use std::fs;
use std::path::Path;

use arnis_core::emitter::{
    Emitter, EmitterError, IngestedBuilding, IngestedRoad, IngestedTile, IngestedWater,
};
use arnis_core::projection::{LtpOrigin, DEFAULT_STUDS_PER_METRE};

use manifest::{
    AssetRef, BuildingEntry, LandmarkEntry, RoadEntry, RobloxManifest, TerrainGrid, WaterEntry,
    MANIFEST_VERSION,
};
use schema::MANIFEST_SCHEMA;

/// Default terrain heightmap resolution per Q036 (base grid).
pub const DEFAULT_TERRAIN_WIDTH: u16 = 128;
pub const DEFAULT_TERRAIN_HEIGHT: u16 = 128;

/// 200 m tile size per Q041 / Q037.
pub const DEFAULT_TILE_EXTENT_M: f64 = 200.0;

#[derive(Debug, Clone, Copy)]
pub struct RobloxEmitter {
    pub studs_per_metre: f64,
    pub style_version: u32,
    pub terrain_width: u16,
    pub terrain_height: u16,
    pub tile_extent_m: f64,
}

impl Default for RobloxEmitter {
    fn default() -> Self {
        Self {
            studs_per_metre: DEFAULT_STUDS_PER_METRE,
            style_version: 1,
            terrain_width: DEFAULT_TERRAIN_WIDTH,
            terrain_height: DEFAULT_TERRAIN_HEIGHT,
            tile_extent_m: DEFAULT_TILE_EXTENT_M,
        }
    }
}

impl Emitter for RobloxEmitter {
    type Manifest = RobloxManifest;

    fn name(&self) -> &'static str {
        "roblox"
    }
    fn schema_version(&self) -> &'static str {
        MANIFEST_VERSION
    }

    fn emit(&self, tile: &IngestedTile, out_dir: &Path) -> Result<RobloxManifest, EmitterError> {
        let manifest = self.build_manifest(tile);

        if !out_dir.as_os_str().is_empty() {
            fs::create_dir_all(out_dir)?;
            let path = out_dir.join("manifest.json");
            let json = serde_json::to_string_pretty(&manifest)?;
            fs::write(&path, json)?;
        }

        Ok(manifest)
    }

    fn validate(&self, manifest: &RobloxManifest) -> Result<(), EmitterError> {
        let json = serde_json::to_value(manifest)?;
        let schema = &*MANIFEST_SCHEMA;
        let result = schema.validate(&json);
        if let Err(errs) = result {
            let msg = errs.map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
            return Err(EmitterError::SchemaInvalid(msg));
        }
        Ok(())
    }
}

impl RobloxEmitter {
    /// Pure build — useful in tests so we don't have to touch the filesystem.
    pub fn build_manifest(&self, tile: &IngestedTile) -> RobloxManifest {
        let origin = LtpOrigin::new(tile.bbox.south_lat, tile.bbox.west_lon);
        let region_key = tile.region_key.clone().unwrap_or_else(|| "default".into());

        let center_lat = 0.5 * (tile.bbox.south_lat + tile.bbox.north_lat);
        let center_lon = 0.5 * (tile.bbox.west_lon + tile.bbox.east_lon);

        let buildings = tile
            .buildings
            .iter()
            .filter_map(|b| self.build_building(&origin, &region_key, b))
            .collect();

        let roads = tile
            .roads
            .iter()
            .filter_map(|r| self.build_road(&origin, r))
            .collect();

        let water = tile
            .water
            .iter()
            .filter_map(|w| self.build_water(&origin, w))
            .collect();

        let terrain = Some(self.build_terrain(tile));

        RobloxManifest {
            manifest_version: MANIFEST_VERSION.to_string(),
            style_version: self.style_version,
            tile: tile.coord,
            stud_scale: self.studs_per_metre as f32,
            center_wgs84: [center_lat, center_lon],
            region_key,
            buildings,
            roads,
            water,
            landmarks: Vec::<LandmarkEntry>::new(),
            assets: Vec::<AssetRef>::new(),
            terrain,
        }
    }

    fn project_ring(&self, origin: &LtpOrigin, ring: &[[f64; 2]]) -> Vec<[f32; 2]> {
        ring.iter()
            .map(|p| {
                let (x, y) = origin.project_studs(p[0], p[1], self.studs_per_metre);
                [x as f32, y as f32]
            })
            .collect()
    }

    fn build_building(
        &self,
        origin: &LtpOrigin,
        region_key: &str,
        b: &IngestedBuilding,
    ) -> Option<BuildingEntry> {
        if b.footprint.len() < 3 {
            return None;
        }
        let area_m2 = heuristics::polygon_area_m2(&b.footprint);
        let (height_m, category) = heuristics::building_height_m(
            b.height_m,
            b.levels,
            b.building_kind.as_deref(),
            area_m2,
        );

        let pal = palette::palette_for(region_key);
        // Salt wall and roof so they don't always agree.
        let wall = palette::pick(pal.wall, &b.osm_id, 0xA1);
        let roof = palette::pick(pal.roof, &b.osm_id, 0xB2);

        // Q211 blend — pageview rarity is the dominant signal; height
        // contributes secondarily. Other factors (heritage, age, etc.)
        // land as their tickets ship. `blend` returns None when every
        // factor is zero so we don't bloat snapshots for ordinary
        // buildings.
        let rarity_inputs = rarity::RarityInputs {
            pageview_rarity: b.pageview_rarity,
            height_m: Some(height_m),
            ..Default::default()
        };
        let rarity_score = rarity::blend(&rarity_inputs);
        let rarity_tier = rarity_score.map(|s| rarity::tier_label(s).to_string());

        Some(BuildingEntry {
            osm_id: b.osm_id.clone(),
            footprint_studs: self.project_ring(origin, &b.footprint),
            height_studs: (height_m as f64 * self.studs_per_metre) as f32,
            wall_colour_hex: wall.to_string(),
            roof_colour_hex: roof.to_string(),
            category: category.to_string(),
            claimable: matches!(
                category,
                "residential" | "apartments" | "apartments_large" | "generic"
            ),
            rarity_score,
            rarity_tier,
        })
    }

    fn build_road(&self, origin: &LtpOrigin, r: &IngestedRoad) -> Option<RoadEntry> {
        if r.polyline.len() < 2 {
            return None;
        }
        let (width_m, material) = heuristics::road_width_m(&r.highway_class, r.lanes);
        Some(RoadEntry {
            osm_id: r.osm_id.clone(),
            polyline_studs: self.project_ring(origin, &r.polyline),
            width_studs: (width_m as f64 * self.studs_per_metre) as f32,
            material: material.to_string(),
            class: r.highway_class.clone(),
        })
    }

    fn build_water(&self, origin: &LtpOrigin, w: &IngestedWater) -> Option<WaterEntry> {
        if w.polygon.len() < 3 {
            return None;
        }
        let depth_m = heuristics::water_depth_m(&w.kind);
        Some(WaterEntry {
            osm_id: w.osm_id.clone(),
            polygon_studs: self.project_ring(origin, &w.polygon),
            depth_studs: (depth_m as f64 * self.studs_per_metre) as f32,
            kind: w.kind.clone(),
        })
    }

    fn build_terrain(&self, tile: &IngestedTile) -> TerrainGrid {
        let extent_studs = (self.tile_extent_m * self.studs_per_metre) as f32;
        let expected = (self.terrain_width as usize) * (self.terrain_height as usize);

        // Prefer provided heightmap; otherwise emit a flat zero grid so
        // downstream consumers never have to special-case "missing terrain".
        let heights_studs = if let Some(hm) = &tile.heightmap {
            if hm.samples.len() == expected
                && hm.width == self.terrain_width
                && hm.height == self.terrain_height
            {
                hm.samples
                    .iter()
                    .map(|m| (*m as f64 * self.studs_per_metre) as f32)
                    .collect()
            } else {
                // Resample by nearest-neighbour into our target grid.
                resample_nearest(
                    &hm.samples,
                    hm.width as usize,
                    hm.height as usize,
                    self.terrain_width as usize,
                    self.terrain_height as usize,
                )
                .into_iter()
                .map(|m| (m as f64 * self.studs_per_metre) as f32)
                .collect()
            }
        } else {
            vec![0.0_f32; expected]
        };

        TerrainGrid {
            width: self.terrain_width,
            height: self.terrain_height,
            tile_extent_studs: extent_studs,
            heights_studs,
        }
    }
}

fn resample_nearest(
    src: &[f32],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<f32> {
    if src.is_empty() || src_w == 0 || src_h == 0 {
        return vec![0.0; dst_w * dst_h];
    }
    let mut out = Vec::with_capacity(dst_w * dst_h);
    for y in 0..dst_h {
        let sy = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let sx = (x * src_w) / dst_w;
            out.push(src[sy * src_w + sx]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arnis_core::emitter::{Heightmap, IngestedBuilding, TileBbox, TileCoord};

    fn bbox() -> TileBbox {
        // ~200 m square near Oslo. 1° lat ≈ 111_320 m → 0.001797° ≈ 200 m
        TileBbox {
            south_lat: 59.9100,
            west_lon: 10.7500,
            north_lat: 59.9100 + 0.001797,
            east_lon: 10.7500 + 0.003580, // ≈200m east at lat 59.91
        }
    }

    fn tile() -> IngestedTile {
        IngestedTile::empty(
            TileCoord {
                z: 15,
                x: 17000,
                y: 9500,
            },
            bbox(),
        )
    }

    #[test]
    fn empty_manifest_validates_against_schema() {
        let e = RobloxEmitter::default();
        let m = e.build_manifest(&tile());
        e.validate(&m).expect("empty manifest must validate");
        assert_eq!(m.manifest_version, "1.0");
        assert_eq!(m.stud_scale, 2.0);
        assert_eq!(m.region_key, "default");
        assert!(m.terrain.is_some());
        let t = m.terrain.as_ref().unwrap();
        assert_eq!(t.width, 128);
        assert_eq!(t.height, 128);
        assert_eq!(t.heights_studs.len(), 128 * 128);
    }

    #[test]
    fn invalid_stud_scale_is_rejected_by_schema() {
        let e = RobloxEmitter {
            studs_per_metre: 999.0,
            ..RobloxEmitter::default()
        };
        let m = e.build_manifest(&tile());
        let err = e.validate(&m).expect_err("stud scale 999 must fail");
        assert!(matches!(err, EmitterError::SchemaInvalid(_)));
    }

    #[test]
    fn building_extrusion_uses_q031_defaults() {
        // 6 m house default for `house` per Q031, × 2 studs/m = 12 studs.
        let mut t = tile();
        t.buildings.push(IngestedBuilding {
            osm_id: "way/1".into(),
            footprint: vec![
                [59.9101, 10.7501],
                [59.9101, 10.7502],
                [59.9102, 10.7502],
                [59.9102, 10.7501],
            ],
            height_m: None,
            levels: None,
            building_kind: Some("house".into()),
            ..Default::default()
        });
        let e = RobloxEmitter::default();
        let m = e.build_manifest(&t);
        e.validate(&m).expect("manifest must validate");
        assert_eq!(m.buildings.len(), 1);
        let b = &m.buildings[0];
        assert!(
            (b.height_studs - 12.0).abs() < 1e-3,
            "got {}",
            b.height_studs
        );
        assert_eq!(b.footprint_studs.len(), 4);
        assert_eq!(b.category, "residential");
        assert!(b.claimable);
        assert!(b.wall_colour_hex.starts_with('#'));
        assert_eq!(b.wall_colour_hex.len(), 7);
    }

    #[test]
    fn projection_anchors_to_sw_corner() {
        let t = tile();
        let e = RobloxEmitter::default();
        let origin = LtpOrigin::new(t.bbox.south_lat, t.bbox.west_lon);
        let pts = e.project_ring(
            &origin,
            &[
                [t.bbox.south_lat, t.bbox.west_lon],
                [t.bbox.north_lat, t.bbox.east_lon],
            ],
        );
        assert!(
            pts[0][0].abs() < 1e-3 && pts[0][1].abs() < 1e-3,
            "SW must project to (0,0): {:?}",
            pts[0]
        );
        // NE corner should be ~400 studs east and ~400 studs north (200 m × 2 studs/m).
        assert!(pts[1][0] > 350.0 && pts[1][0] < 450.0, "x={}", pts[1][0]);
        assert!(pts[1][1] > 350.0 && pts[1][1] < 450.0, "y={}", pts[1][1]);
    }

    #[test]
    fn manifest_serialises_roundtrip() {
        let e = RobloxEmitter::default();
        let m = e.build_manifest(&tile());
        let json = serde_json::to_string(&m).unwrap();
        let back: RobloxManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.manifest_version, m.manifest_version);
        assert_eq!(back.tile, m.tile);
        assert_eq!(back.region_key, m.region_key);
        assert_eq!(back.terrain.unwrap().heights_studs.len(), 128 * 128);
    }

    #[test]
    fn deterministic_palette_pick() {
        let e = RobloxEmitter::default();
        let mut t = tile();
        t.region_key = Some("NO_rural_subarctic".into());
        for i in 0..5 {
            t.buildings.push(IngestedBuilding {
                osm_id: format!("way/{}", i),
                footprint: vec![[59.9101, 10.7501], [59.9101, 10.7502], [59.9102, 10.7502]],
                height_m: None,
                levels: None,
                building_kind: Some("house".into()),
                ..Default::default()
            });
        }
        let m1 = e.build_manifest(&t);
        let m2 = e.build_manifest(&t);
        assert_eq!(
            m1.buildings
                .iter()
                .map(|b| b.wall_colour_hex.clone())
                .collect::<Vec<_>>(),
            m2.buildings
                .iter()
                .map(|b| b.wall_colour_hex.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn heightmap_resamples_to_target_grid() {
        let mut t = tile();
        t.heightmap = Some(Heightmap {
            width: 4,
            height: 4,
            samples: (0..16).map(|i| i as f32).collect(),
        });
        let e = RobloxEmitter::default();
        let m = e.build_manifest(&t);
        let grid = m.terrain.unwrap();
        assert_eq!(grid.heights_studs.len(), 128 * 128);
        // Values should be in [0, 15] * studs_per_metre.
        let max = grid.heights_studs.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max <= 15.0 * 2.0 + 1e-3);
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use arnis_core::emitter::{IngestedBuilding, TileBbox, TileCoord};
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn projection_within_tile_stays_in_bounds(
            d_lat in 0.0f64..0.001797,
            d_lon in 0.0f64..0.003580,
        ) {
            let bbox = TileBbox {
                south_lat: 59.91, west_lon: 10.75,
                north_lat: 59.91 + 0.001797, east_lon: 10.75 + 0.003580,
            };
            let origin = LtpOrigin::new(bbox.south_lat, bbox.west_lon);
            let (x, y) = origin.project_studs(
                bbox.south_lat + d_lat,
                bbox.west_lon + d_lon,
                DEFAULT_STUDS_PER_METRE,
            );
            // 200 m tile × 2 studs/m = 400 studs edge, allow 1 stud slack.
            prop_assert!((-0.5..=401.0).contains(&x), "x out of bounds: {x}");
            prop_assert!((-0.5..=401.0).contains(&y), "y out of bounds: {y}");
        }

        #[test]
        fn building_height_monotonic_in_levels(
            levels in 1u16..50,
        ) {
            let (h, _) = heuristics::building_height_m(None, Some(levels), Some("apartments"), 500.0);
            prop_assert!(h >= 3.0);
            prop_assert!(h <= (levels as f32) * 4.0 + 0.001);
        }

        #[test]
        fn polygon_area_non_negative_for_random_rings(
            n in 3usize..12,
            seed in 0u64..1000,
        ) {
            // Deterministic pseudo-random vertices around a small bbox.
            let mut ring: Vec<[f64; 2]> = (0..n).map(|i| {
                let t = (i as f64) / (n as f64) * std::f64::consts::TAU
                    + (seed as f64) * 0.0001;
                [0.001 * t.cos(), 0.001 * t.sin()]
            }).collect();
            // Translate the ring well off origin to defeat any first-vertex bias.
            for p in ring.iter_mut() { p[0] += 50.0; p[1] += 10.0; }
            let area = heuristics::polygon_area_m2(&ring);
            prop_assert!(area >= 0.0, "area must be non-negative, got {}", area);
            // A non-degenerate ring with n>=3 distinct vertices around a
            // circle of radius ~0.001° must have strictly positive area.
            prop_assert!(area > 0.0);
        }

        #[test]
        fn palette_pick_is_deterministic_per_osm_id(
            id in "way/[0-9]{1,8}",
            salt in any::<u64>(),
        ) {
            let pal = palette::palette_for("NO_rural_subarctic");
            let a = palette::pick(pal.wall, &id, salt);
            let b = palette::pick(pal.wall, &id, salt);
            prop_assert_eq!(a, b);
        }

        #[test]
        fn building_height_monotonic_in_levels_strict(
            l1 in 1u16..30, bump in 1u16..20,
        ) {
            let l2 = l1.saturating_add(bump).min(50);
            let (h1, _) = heuristics::building_height_m(None, Some(l1), Some("apartments"), 500.0);
            let (h2, _) = heuristics::building_height_m(None, Some(l2), Some("apartments"), 500.0);
            prop_assert!(h2 >= h1, "h({l2})={h2} must be >= h({l1})={h1}");
        }

        #[test]
        fn schema_validation_holds_for_random_buildings(
            n in 0usize..8,
        ) {
            let mut t = IngestedTile::empty(
                TileCoord { z: 15, x: 1, y: 1 },
                TileBbox {
                    south_lat: 0.0, west_lon: 0.0,
                    north_lat: 0.001797, east_lon: 0.001797,
                },
            );
            for i in 0..n {
                t.buildings.push(IngestedBuilding {
                    osm_id: format!("w/{i}"),
                    footprint: vec![
                        [0.0001, 0.0001],
                        [0.0001, 0.0002],
                        [0.0002, 0.0002],
                        [0.0002, 0.0001],
                    ],
                    height_m: None,
                    levels: None,
                    building_kind: Some("house".into()),
                    ..Default::default()
                });
            }
            let e = RobloxEmitter::default();
            let m = e.build_manifest(&t);
            e.validate(&m).expect("must validate");
        }
    }
}
