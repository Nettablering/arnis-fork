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

/// Q211 verification: a building tagged with a Wikidata QID + pageview
/// rarity ends up with `rarity_score` + `rarity_tier` in the manifest;
/// untagged buildings keep the old shape.
#[test]
fn snapshot_landmark_with_pageview_rarity() {
    let mut t = fixture_empty_tile();
    t.region_key = Some("NO_rural_subarctic".into());

    // Eiffel-class landmark (high pageview rarity, tall) alongside an
    // ordinary 4-storey block (no Wikipedia article).
    t.buildings = vec![
        IngestedBuilding {
            osm_id: "way/9001".into(),
            footprint: vec![
                [59.9101, 10.7501],
                [59.9101, 10.7503],
                [59.9103, 10.7503],
                [59.9103, 10.7501],
            ],
            height_m: Some(300.0),
            building_kind: Some("tower".into()),
            wikidata_qid: Some("Q243".into()), // Eiffel Tower
            pageview_rarity: Some(0.90),
            ..Default::default()
        },
        IngestedBuilding {
            osm_id: "way/9002".into(),
            footprint: vec![
                [59.9105, 10.7505],
                [59.9105, 10.7510],
                [59.9110, 10.7510],
                [59.9110, 10.7505],
            ],
            levels: Some(4),
            building_kind: Some("apartments".into()),
            ..Default::default()
        },
    ];

    let e = RobloxEmitter::default();
    let m = strip_terrain(e.build_manifest(&t));
    insta::with_settings!({ sort_maps => true }, {
        insta::assert_json_snapshot!(m);
    });

    // Validate the manifest still passes schema (rarity fields are
    // optional + bounded).
    use arnis_core::emitter::Emitter;
    e.validate(&e.build_manifest(&t))
        .expect("schema must accept rarity fields");

    // Sanity: Eiffel building must carry rarity, plain block must not.
    let eiffel = m.buildings.iter().find(|b| b.osm_id == "way/9001").unwrap();
    assert!(eiffel.rarity_score.is_some());
    // 0.40*0.90 + 0.15*height_rarity(300m) ≈ 0.36 + 0.13 ≈ 0.49 → Rare.
    // Other Q211 factors (heritage, age, uniqueness, fictional) ship in
    // later tickets; once they're wired this same Eiffel-class input
    // will climb to Legendary/Mythic.
    assert_eq!(eiffel.rarity_tier.as_deref(), Some("Rare"));
    let plain = m.buildings.iter().find(|b| b.osm_id == "way/9002").unwrap();
    assert!(
        plain.rarity_score.is_none(),
        "plain block should not carry rarity"
    );
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
