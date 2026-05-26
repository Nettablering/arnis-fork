//! Q465 — end-to-end integration test: cached Overpass JSON →
//! [`arnis_emitters::overpass_ingest::ingest_overpass`] → [`RobloxEmitter`]
//! → schema-valid manifest containing real OSM way IDs.
//!
//! The fixture is committed to the crate under `tests/fixtures/` so the
//! test runs offline. The same JSON shape is what
//! `backend/scripts/fetch-overpass.sh` writes to
//! `backend/cache/overpass/<bbox-hash>.json` against the live Overpass API.

use arnis_core::emitter::Emitter;
use arnis_emitters::overpass_ingest::{ingest_overpass, slippy_tile_for};
use arnis_emitters::roblox::RobloxEmitter;

const AKSLA_LAT: f64 = 62.4720;
const AKSLA_LON: f64 = 6.1500;
const Z: u8 = 16;

#[test]
fn e2e_overpass_aksla_real_ways() {
    let raw = include_bytes!("fixtures/aksla.overpass.json");

    let coord = slippy_tile_for(AKSLA_LAT, AKSLA_LON, Z);
    assert_eq!(coord.z, Z);

    let mut tile = ingest_overpass(raw, coord).expect("ingest");
    tile.region_key = Some("NO_rural_subarctic".into());

    // Sanity: the fixture contains real OSM data — at least one building.
    assert!(
        !tile.buildings.is_empty(),
        "expected real buildings in Aksla fixture"
    );

    let emitter = RobloxEmitter::default();
    let manifest = emitter.build_manifest(&tile);
    emitter
        .validate(&manifest)
        .expect("Aksla manifest must pass schema");

    // Manifest must carry real OSM way IDs (string prefix "way/").
    for b in &manifest.buildings {
        assert!(b.osm_id.starts_with("way/"), "bad osm_id: {}", b.osm_id);
    }

    // Spot-check expected counts (fixture is fixed at ingest time).
    assert!(
        manifest.buildings.len() >= 50,
        "buildings: {}",
        manifest.buildings.len()
    );
    assert!(
        manifest.roads.len() >= 10,
        "roads: {}",
        manifest.roads.len()
    );

    // Tile coord matches the slippy expectation for Aksla at z=16.
    assert_eq!(manifest.tile.z, Z);
}

#[test]
fn e2e_overpass_round_trip_serialises_cleanly() {
    let raw = include_bytes!("fixtures/aksla.overpass.json");
    let coord = slippy_tile_for(AKSLA_LAT, AKSLA_LON, Z);
    let tile = ingest_overpass(raw, coord).expect("ingest");
    let manifest = RobloxEmitter::default().build_manifest(&tile);
    let json = serde_json::to_string(&manifest).expect("serialise");
    let back: serde_json::Value = serde_json::from_str(&json).expect("parse back");
    assert_eq!(back["manifest_version"], "1.0");
    assert_eq!(back["tile"]["z"], Z);
}
