//! Integration tests: spin up an isolated PostGIS container via
//! testcontainers-rs, run the migration suite, and assert that the
//! schema looks the way `docs/grill/q085-postgis-schema.md` says it should.
//!
//! These tests require a running Docker daemon. If Docker is unavailable
//! the test is skipped via `#[ignore]`-style early return + eprintln.

use sqlx::Row;
use testcontainers::core::ContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ImageExt, GenericImage};

async fn boot_postgis() -> anyhow::Result<(testcontainers::ContainerAsync<GenericImage>, String)> {
    // postgis/postgis:16-3.4 mirrors the version on hetzner-prod
    // (PostgreSQL 16 + PostGIS 3.4). Using GenericImage so we can pin the
    // exact repository (the `testcontainers_modules::postgres::Postgres`
    // helper hard-codes the upstream postgres image which lacks PostGIS).
    let image = GenericImage::new("postgis/postgis", "16-3.4")
        .with_wait_for(testcontainers::core::WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_exposed_port(ContainerPort::Tcp(5432))
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "postgres")
        .with_env_var("POSTGRES_USER", "postgres");

    let container = image.start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    // Give PG a couple of seconds to settle after the "ready" log line; the
    // first connection just after the message occasionally races on slow CI.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok((container, url))
}

#[tokio::test]
async fn migrations_apply_cleanly_on_fresh_postgis() {
    if std::env::var("DOCKER_HOST").is_err() && !std::path::Path::new("/var/run/docker.sock").exists() {
        eprintln!("SKIP: Docker socket not available");
        return;
    }

    let (_container, url) = boot_postgis()
        .await
        .expect("boot postgis testcontainer");

    let pool = wb_db::connect(&url).await.expect("connect to test postgres");
    wb_db::migrate(&pool).await.expect("run migrations");

    // Both logical schemas exist.
    let schemas: Vec<String> = sqlx::query_scalar(
        "SELECT schema_name FROM information_schema.schemata
         WHERE schema_name IN ('osm','wb') ORDER BY schema_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(schemas, vec!["osm".to_string(), "wb".to_string()]);

    // postgis extension present.
    let postgis: Option<String> =
        sqlx::query_scalar("SELECT extname FROM pg_extension WHERE extname='postgis'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(postgis.as_deref(), Some("postgis"));

    // Core wb.* tables exist.
    let expected_wb = [
        "universes",
        "players",
        "tiles_cold",
        "hot_tile_registry",
        "overlays",
        "bake_jobs",
    ];
    for t in expected_wb {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
                            WHERE table_schema='wb' AND table_name=$1)",
        )
        .bind(t)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "wb.{t} missing");
    }

    // Core osm.* tables.
    for t in ["planet_osm_polygon", "planet_osm_line", "planet_osm_point", "planet_osm_rels"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
                            WHERE table_schema='osm' AND table_name=$1)",
        )
        .bind(t)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "osm.{t} missing");
    }

    // GIST(way) — the critical index per Q085 — exists on the polygon table.
    let gist_idx: Option<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname='osm' AND tablename='planet_osm_polygon'
           AND indexname='planet_osm_polygon_way_gist'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(gist_idx.is_some(), "GIST(way) on osm.planet_osm_polygon missing");

    // Idempotency: re-running migrate is a no-op.
    wb_db::migrate(&pool).await.expect("second migrate run no-op");
}

#[tokio::test]
async fn overlay_roundtrip_with_fk_cascades() {
    if std::env::var("DOCKER_HOST").is_err() && !std::path::Path::new("/var/run/docker.sock").exists() {
        eprintln!("SKIP: Docker socket not available");
        return;
    }

    let (_container, url) = boot_postgis().await.expect("boot postgis");
    let pool = wb_db::connect(&url).await.unwrap();
    wb_db::migrate(&pool).await.unwrap();

    // Insert a universe -> player -> tile -> overlay row and round-trip it.
    sqlx::query(
        "INSERT INTO wb.universes (universe_id, hmac_key_current)
         VALUES (42, decode('00','hex'))",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO wb.players (universe_id, user_id, country_hint)
         VALUES (42, 1001, 'NO')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO wb.tiles_cold
            (tile_id, z, x, y, style_version, osm_snapshot, manifest, manifest_hash, attribution)
         VALUES ('15/16384/10922', 15, 16384, 10922, 1, '2026-05-26',
                 '{\"version\":1}'::jsonb, decode('aa','hex'),
                 'OpenStreetMap contributors')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO wb.overlays
            (universe_id, user_id, tile_id, osm_snapshot, claimed)
         VALUES (42, 1001, '15/16384/10922', '2026-05-26',
                 '{\"way/123\":{\"level\":1}}'::jsonb)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let row = sqlx::query(
        "SELECT claimed::text AS claimed_text FROM wb.overlays
         WHERE universe_id=42 AND user_id=1001 AND tile_id='15/16384/10922'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let claimed: String = row.get("claimed_text");
    assert!(claimed.contains("way/123"));

    // Deleting the universe should cascade to player + overlay.
    sqlx::query("DELETE FROM wb.universes WHERE universe_id=42")
        .execute(&pool)
        .await
        .unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM wb.overlays WHERE universe_id=42")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0, "overlay row should cascade-delete with universe");
}

#[tokio::test]
async fn osm_bbox_query_uses_gist_index() {
    if std::env::var("DOCKER_HOST").is_err() && !std::path::Path::new("/var/run/docker.sock").exists() {
        eprintln!("SKIP: Docker socket not available");
        return;
    }

    let (_container, url) = boot_postgis().await.expect("boot postgis");
    let pool = wb_db::connect(&url).await.unwrap();
    wb_db::migrate(&pool).await.unwrap();

    // Insert one building polygon in Oslo, then bbox-query the same area.
    sqlx::query(
        "INSERT INTO osm.planet_osm_polygon (osm_id, tags, way) VALUES
         (1, '{\"building\":\"yes\"}'::jsonb,
          ST_GeomFromText('POLYGON((10.74 59.91, 10.75 59.91, 10.75 59.92, 10.74 59.92, 10.74 59.91))', 4326))",
    )
    .execute(&pool)
    .await
    .unwrap();

    let hits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM osm.planet_osm_polygon
         WHERE way && ST_MakeEnvelope(10.74, 59.91, 10.75, 59.92, 4326)
           AND tags ? 'building'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hits, 1, "bbox+building filter should hit our one inserted row");

    // Plan should reference the gist index (or partial gist) for this query shape.
    // Note: planner may pick seq scan on a 1-row table; with index forced it works.
    sqlx::query("SET enable_seqscan = off").execute(&pool).await.unwrap();
    let plan: Vec<(String,)> = sqlx::query_as(
        "EXPLAIN SELECT osm_id FROM osm.planet_osm_polygon
         WHERE way && ST_MakeEnvelope(10.74, 59.91, 10.75, 59.92, 4326)
           AND tags ? 'building'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let plan_text = plan.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join("\n");
    assert!(
        plan_text.contains("planet_osm_polygon_way") || plan_text.contains("building_partial"),
        "EXPLAIN should mention the GIST index, got:\n{plan_text}"
    );
}
