//! Q086 — equivalence test: PostGisSource must produce the same manifest as
//! OverpassSource for the same OSM snapshot + bbox.
//!
//! Strategy:
//!  * Load the same three test elements from `fixtures/aksla-overpass.json`
//!    into a PostGIS DB (DATABASE_URL or fall back to skip).
//!  * Build manifests via both backends.
//!  * Assert they are JSON-equal.
//!
//! Requires the worldbuilders DB created by `backend/scripts/db-create.sh`.
//! If `DATABASE_URL` is unset / unreachable the test SKIPs (prints a notice
//! and returns) so `cargo test --workspace --no-default-features` stays
//! green in environments without Postgres.

use arnis_emitters::roblox::RobloxEmitter;
use osm_postgis_source::{
    classify, slippy_tile_bbox, slippy_tile_for, OsmSource, OverpassSource, PostGisSource,
};
use sqlx::postgres::PgPoolOptions;

fn db_url() -> Option<String> {
    if let Ok(u) = std::env::var("DATABASE_URL") {
        return Some(u);
    }
    let env_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../db/.env")
        .canonicalize()
        .ok()?;
    let text = std::fs::read_to_string(env_file).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("DATABASE_URL=") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

#[tokio::test]
async fn overpass_and_postgis_manifests_match_for_aksla_bbox() {
    let Some(url) = db_url() else {
        eprintln!("SKIP: DATABASE_URL not set; cannot run Q086 equivalence test");
        return;
    };

    // Open the pool — if Postgres is unreachable, SKIP rather than fail.
    let pool = match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect(&url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SKIP: Postgres unreachable: {e}");
            return;
        }
    };

    // Verify the Q085 schema is present; SKIP otherwise.
    if sqlx::query_scalar::<_, i32>("SELECT 1 FROM information_schema.tables WHERE table_schema='osm' AND table_name='planet_osm_polygon'")
        .fetch_optional(&pool).await.ok().flatten().is_none()
    {
        eprintln!("SKIP: osm.planet_osm_polygon missing (run db migrations)");
        return;
    }

    // Seed the three fixture rows in an isolated transaction-like prelude.
    // We clear and reload deterministically so re-runs are idempotent.
    sqlx::query("DELETE FROM osm.planet_osm_polygon WHERE osm_id IN (1001, 1003)")
        .execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM osm.planet_osm_line WHERE osm_id = 1002")
        .execute(&pool).await.unwrap();

    // Place fixtures in a remote ocean bbox to avoid colliding with any
    // real OSM rows a county-extract import may have loaded into the same
    // Q085 tables. (Roughly mid-Atlantic, 40°N 30°W.)
    sqlx::query(
        r#"INSERT INTO osm.planet_osm_polygon (osm_id, tags, way)
           VALUES (1001,
                   '{"building":"house","building:levels":"2"}'::jsonb,
                   ST_GeomFromText('POLYGON((-30.0000 40.0000, -30.0000 40.0001, -29.9999 40.0001, -29.9999 40.0000, -30.0000 40.0000))', 4326))"#,
    )
    .execute(&pool).await.unwrap();

    sqlx::query(
        r#"INSERT INTO osm.planet_osm_line (osm_id, tags, way)
           VALUES (1002,
                   '{"highway":"residential","lanes":"2"}'::jsonb,
                   ST_GeomFromText('LINESTRING(-30.0000 40.0000, -29.9998 40.0001)', 4326))"#,
    )
    .execute(&pool).await.unwrap();

    sqlx::query(
        r#"INSERT INTO osm.planet_osm_polygon (osm_id, tags, way)
           VALUES (1003,
                   '{"natural":"water"}'::jsonb,
                   ST_GeomFromText('POLYGON((-29.9997 40.0000, -29.9997 40.0001, -29.9996 40.0001, -29.9996 40.0000, -29.9997 40.0000))', 4326))"#,
    )
    .execute(&pool).await.unwrap();

    let coord = slippy_tile_for(40.0000, -30.0000, 16);
    let bbox = slippy_tile_bbox(coord).into();

    // --- PostGIS path ---
    let pg_elements = PostGisSource::new(pool.clone())
        .fetch_bbox(bbox).await.expect("PostGisSource");
    let pg_tile = classify(pg_elements, coord);

    // --- Overpass path ---
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/aksla-overpass.json");
    let op_elements = OverpassSource::from_path(&fixture)
        .fetch_bbox(bbox).await.expect("OverpassSource");
    let op_tile = classify(op_elements, coord);

    // --- Build manifests via the SAME emitter both times ---
    let emitter = RobloxEmitter::default();
    let pg_manifest = emitter.build_manifest(&pg_tile);
    let op_manifest = emitter.build_manifest(&op_tile);

    // Sanity: counts non-zero (the fixture has 1 building + 1 road + 1 water).
    assert_eq!(pg_manifest.buildings.len(), 1, "postgis buildings");
    assert_eq!(pg_manifest.roads.len(),     1, "postgis roads");
    assert_eq!(pg_manifest.water.len(),     1, "postgis water");

    // Both manifests must serialise to the same JSON (after sorting in
    // classify()) so the bake-server cache hits are stable regardless of
    // which backend is selected.
    let pg_json = serde_json::to_value(&pg_manifest).unwrap();
    let op_json = serde_json::to_value(&op_manifest).unwrap();
    assert_eq!(pg_json, op_json,
        "manifests differ between OverpassSource and PostGisSource\n\
         postgis = {pg_json:#}\noverpass = {op_json:#}");

    // Cleanup so re-runs stay tidy.
    sqlx::query("DELETE FROM osm.planet_osm_polygon WHERE osm_id IN (1001, 1003)")
        .execute(&pool).await.ok();
    sqlx::query("DELETE FROM osm.planet_osm_line WHERE osm_id = 1002")
        .execute(&pool).await.ok();
}
