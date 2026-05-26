//! Insta JSON snapshot tests for the Roblox manifest emitter (Q475).
//!
//! Captures the exact serialised JSON shape of `RobloxManifest` for a
//! couple of canonical fixtures. Baselines committed under
//! `tests/snapshots/`; mismatches fail CI and require `cargo insta review`
//! to accept. The terrain grid (128*128 floats) is omitted from the
//! manifest before snapshotting to keep diffs reviewable — its shape is
//! still covered by the inline unit tests in `src/roblox/mod.rs`.

use arnis_core::emitter::{
    IngestedBuilding, IngestedRoad, IngestedTile, IngestedWater, TileBbox, TileCoord,
};
use arnis_emitters::roblox::manifest::RobloxManifest;
use arnis_emitters::RobloxEmitter;

fn fixture_bbox() -> TileBbox {
    TileBbox {
        south_lat: 59.9100,
        west_lon: 10.7500,
        north_lat: 59.9100 + 0.001797,
        east_lon: 10.7500 + 0.003580,
    }
}

fn fixture_empty_tile() -> IngestedTile {
    IngestedTile::empty(
        TileCoord {
            z: 15,
            x: 17000,
            y: 9500,
        },
        fixture_bbox(),
    )
}

fn fixture_three_building_tile() -> IngestedTile {
    let mut t = fixture_empty_tile();
    t.region_key = Some("NO_rural_subarctic".into());

    t.buildings = vec![
        IngestedBuilding {
            osm_id: "way/100".into(),
            footprint: vec![
                [59.9101, 10.7501],
                [59.9101, 10.7503],
                [59.9103, 10.7503],
                [59.9103, 10.7501],
            ],
            height_m: None,
            levels: None,
            building_kind: Some("house".into()),
            ..Default::default()
        },
        IngestedBuilding {
            osm_id: "way/200".into(),
            footprint: vec![
                [59.9105, 10.7505],
                [59.9105, 10.7510],
                [59.9110, 10.7510],
                [59.9110, 10.7505],
            ],
            height_m: None,
            levels: Some(4),
            building_kind: Some("apartments".into()),
            ..Default::default()
        },
        IngestedBuilding {
            osm_id: "way/300".into(),
            footprint: vec![
                [59.9112, 10.7512],
                [59.9112, 10.7515],
                [59.9115, 10.7515],
                [59.9115, 10.7512],
            ],
            height_m: Some(11.5),
            levels: None,
            building_kind: Some("commercial".into()),
            ..Default::default()
        },
    ];

    t.roads = vec![IngestedRoad {
        osm_id: "way/900".into(),
        polyline: vec![[59.9101, 10.7501], [59.9115, 10.7515]],
        highway_class: "residential".into(),
        lanes: Some(2),
    }];

    t.water = vec![IngestedWater {
        osm_id: "way/800".into(),
        polygon: vec![
            [59.9118, 10.7518],
            [59.9118, 10.7520],
            [59.9120, 10.7520],
            [59.9120, 10.7518],
        ],
        kind: "river".into(),
    }];

    t
}

/// Strip the bulky 128*128 terrain grid from the manifest before snapshotting
/// — its dimensions and length are already covered by inline unit tests,
/// and 16 384 zeroes would dominate the .snap file.
fn strip_terrain(mut m: RobloxManifest) -> RobloxManifest {
    m.terrain = None;
    m
}

#[test]
fn snapshot_empty_manifest() {
    let e = RobloxEmitter::default();
    let m = strip_terrain(e.build_manifest(&fixture_empty_tile()));
    insta::assert_json_snapshot!(m);
}

#[test]
fn snapshot_three_building_tile() {
    let e = RobloxEmitter::default();
    let m = strip_terrain(e.build_manifest(&fixture_three_building_tile()));
    insta::with_settings!({ sort_maps => true }, {
        insta::assert_json_snapshot!(m);
    });
}

#[test]
fn snapshot_is_byte_stable_across_runs() {
    // Property check: two independent builds of the same tile must produce
    // byte-identical JSON. Pairs with the snapshot baselines above; if this
    // ever fails, the snapshot diff is no longer trustworthy.
    let e = RobloxEmitter::default();
    let a = serde_json::to_string(&strip_terrain(
        e.build_manifest(&fixture_three_building_tile()),
    ))
    .unwrap();
    let b = serde_json::to_string(&strip_terrain(
        e.build_manifest(&fixture_three_building_tile()),
    ))
    .unwrap();
    assert_eq!(a, b);
}
