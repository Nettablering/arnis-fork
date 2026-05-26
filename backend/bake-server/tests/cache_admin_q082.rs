//! Integration tests for the Q082 layered cache + admin endpoints.
//!
//! Exercises the full HTTP surface (pin / unpin / stats) including HMAC
//! auth, tier promotion through the live `LayeredCache`, and the
//! interaction with the `X-WB-Cache-Tier` response header on
//! `GET /v1/tile/:z/:x/:y`.

use axum_test::TestServer;
use bake_server::{
    build_router,
    cache::{LayeredCache, TileCache, TileCacheWriter, TileKey},
    now_secs, sign_payload, AppState,
};
use std::sync::Arc;

const TEST_KEY: [u8; 32] = [0xAB; 32];

fn signed(server: &TestServer, method: &'static str, path: &str) -> axum_test::TestRequest {
    let ts = now_secs();
    let sig = sign_payload(&TEST_KEY, ts, path);
    let req = match method {
        "GET" => server.get(path),
        "POST" => server.post(path),
        "DELETE" => server.delete(path),
        _ => panic!("method"),
    };
    req.add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sig)
}

fn boot_with_cache(cache: Arc<LayeredCache>) -> TestServer {
    let state = AppState::new(TEST_KEY.to_vec()).with_layered_cache(cache);
    TestServer::new(build_router(state)).expect("axum-test server")
}

#[tokio::test]
async fn pin_endpoint_promotes_tile_and_persists_across_get() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
    // Seed manifest into HOT via the writer trait.
    cache
        .put(
            TileKey::new(15, 17128, 9656),
            br#"{"manifest":"eiffel"}"#.to_vec(),
        )
        .await
        .unwrap();

    let server = boot_with_cache(cache.clone());

    // POST pin
    let resp = signed(&server, "POST", "/v1/admin/cache/pin/15/17128/9656").await;
    assert_eq!(resp.status_code(), 201, "first pin should be 201 Created");
    let body: serde_json::Value = resp.json();
    assert_eq!(body["pinned"], true);
    assert_eq!(body["newly_created"], true);

    // GET tile — must come back from PINNED tier.
    let path = "/v1/tile/15/17128/9656";
    let resp = signed(&server, "GET", path).await;
    resp.assert_status_ok();
    assert_eq!(resp.header("x-wb-cache-tier").to_str().unwrap(), "pinned");

    // DELETE unpin
    let resp = signed(&server, "DELETE", "/v1/admin/cache/pin/15/17128/9656").await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["pinned"], false);
}

#[tokio::test]
async fn stats_endpoint_reports_counters() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
    cache
        .put(TileKey::new(15, 1, 1), b"{}".to_vec())
        .await
        .unwrap();
    cache.pin(TileKey::new(15, 1, 1)).await.unwrap();

    let server = boot_with_cache(cache);
    let resp = signed(&server, "GET", "/v1/admin/cache/stats").await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["stats"]["pinned_entries"], 1);
    assert_eq!(body["pins"][0], "15/1/1");
}

#[tokio::test]
async fn admin_endpoints_require_hmac() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
    let server = boot_with_cache(cache);

    let resp = server.post("/v1/admin/cache/pin/15/1/1").await;
    assert_eq!(resp.status_code(), 401, "unsigned pin must be 401");
    let resp = server.delete("/v1/admin/cache/pin/15/1/1").await;
    assert_eq!(resp.status_code(), 401, "unsigned unpin must be 401");
    let resp = server.get("/v1/admin/cache/stats").await;
    assert_eq!(resp.status_code(), 401, "unsigned stats must be 401");
}

#[tokio::test]
async fn layered_cache_cold_disk_hit_via_http() {
    // Manifest exists on cold disk only — first GET should serve it and
    // promote to HOT; second GET should report tier=hot via the header.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("15-2-2.json"),
        br#"{"manifest":"from-disk"}"#,
    )
    .unwrap();
    let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
    let server = boot_with_cache(cache);

    let resp = signed(&server, "GET", "/v1/tile/15/2/2").await;
    resp.assert_status_ok();
    assert_eq!(resp.header("x-wb-cache-tier").to_str().unwrap(), "cold");

    let resp = signed(&server, "GET", "/v1/tile/15/2/2").await;
    resp.assert_status_ok();
    assert_eq!(resp.header("x-wb-cache-tier").to_str().unwrap(), "hot");
}

#[tokio::test]
async fn unpin_nonexistent_returns_404() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
    let server = boot_with_cache(cache);

    let resp = signed(&server, "DELETE", "/v1/admin/cache/pin/15/9/9").await;
    assert_eq!(resp.status_code(), 404);
}

#[tokio::test]
async fn double_pin_returns_200_not_201() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = Arc::new(LayeredCache::with_cold_dir(tmp.path().to_path_buf()));
    cache
        .put(TileKey::new(15, 3, 3), b"{}".to_vec())
        .await
        .unwrap();
    let server = boot_with_cache(cache);

    let r1 = signed(&server, "POST", "/v1/admin/cache/pin/15/3/3").await;
    assert_eq!(r1.status_code(), 201);
    let r2 = signed(&server, "POST", "/v1/admin/cache/pin/15/3/3").await;
    assert_eq!(r2.status_code(), 200);
}
