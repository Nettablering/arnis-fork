//! HTTP integration tests for bake-server (Q475).
//!
//! Uses `axum-test` to exercise the router in-process — no real socket
//! bind, no flaky port allocation. Covers the four required cases from
//! Q475:
//!   - GET /v1/health 200 (unauthenticated)
//!   - GET /v1/tile/* without HMAC → 401
//!   - GET /v1/tile/* with bad timestamp/sig → 401
//!   - GET /v1/tile/* with valid HMAC → 200

use axum_test::TestServer;
use bake_server::{build_router, now_secs, sign_payload, AppState};

const TEST_KEY: [u8; 32] = [0xAB; 32];

fn test_server() -> TestServer {
    let app = build_router(AppState::new(TEST_KEY.to_vec()));
    TestServer::new(app).expect("axum-test server")
}

#[tokio::test]
async fn health_returns_200_unauthenticated() {
    let server = test_server();
    let resp = server.get("/v1/health").await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "bake-server");
}

#[tokio::test]
async fn tile_unsigned_returns_401() {
    let server = test_server();
    let resp = server.get("/v1/tile/15/1/1").await;
    assert_eq!(resp.status_code(), 401);
}

#[tokio::test]
async fn tile_signed_with_skewed_ts_returns_401() {
    let server = test_server();
    let stale_ts = now_secs().saturating_sub(3600); // 1h ago — well outside 60s window
    let path = "/v1/tile/15/1/1";
    let sig = sign_payload(&TEST_KEY, stale_ts, path);

    let resp = server
        .get(path)
        .add_header("x-wb-ts", stale_ts.to_string())
        .add_header("x-wb-sig", sig)
        .await;
    assert_eq!(resp.status_code(), 401);
}

#[tokio::test]
async fn tile_signed_with_bad_sig_returns_401() {
    let server = test_server();
    let ts = now_secs();
    let path = "/v1/tile/15/1/1";

    let resp = server
        .get(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", "deadbeef".to_string())
        .await;
    assert_eq!(resp.status_code(), 401);
}

#[tokio::test]
async fn tile_signed_ok_returns_200() {
    let server = test_server();
    let ts = now_secs();
    let path = "/v1/tile/15/1/1";
    let sig = sign_payload(&TEST_KEY, ts, path);

    let resp = server
        .get(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sig)
        .await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    // Q102: bake-server's mock manifest reports `LATEST_VERSION` (1.1).
    assert_eq!(body["manifest_version"], bake_server::schema_version::LATEST_VERSION);
    assert_eq!(body["tile_id"], "15-1-1");
    assert_eq!(body["z"], 15);
    assert_eq!(body["x"], 1);
    assert_eq!(body["y"], 1);
}

#[tokio::test]
async fn tile_with_cache_dir_serves_baked_manifest() {
    // Q465: when AppState has a cache dir and the file exists, the server
    // streams the on-disk bytes verbatim.
    let tmp = tempfile::tempdir().expect("tmpdir");
    let dir = tmp.path().to_path_buf();
    let path = "/v1/tile/16/33887/18095";
    let baked = serde_json::json!({
        "manifest_version": "1.0",
        "style_version": 1,
        "tile": {"z": 16, "x": 33887, "y": 18095},
        "stud_scale": 2.0,
        "center_wgs84": [62.472, 6.150],
        "region_key": "NO_rural_subarctic",
        "buildings": [], "roads": [], "water": [],
        "landmarks": [], "assets": [], "terrain": null,
    });
    std::fs::write(
        dir.join("16-33887-18095.json"),
        serde_json::to_vec(&baked).unwrap(),
    )
    .unwrap();

    let state = AppState::new(TEST_KEY.to_vec()).with_cache_dir(dir);
    let server = TestServer::new(build_router(state)).expect("axum-test");

    let ts = now_secs();
    let sig = sign_payload(&TEST_KEY, ts, path);
    let resp = server
        .get(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sig)
        .await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(body["manifest_version"], "1.0");
    assert_eq!(body["tile"]["x"], 33887);
}

#[tokio::test]
async fn tile_with_cache_dir_miss_returns_503() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let state = AppState::new(TEST_KEY.to_vec()).with_cache_dir(tmp.path().to_path_buf());
    let server = TestServer::new(build_router(state)).expect("axum-test");

    let path = "/v1/tile/16/0/0";
    let ts = now_secs();
    let sig = sign_payload(&TEST_KEY, ts, path);
    let resp = server
        .get(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sig)
        .await;
    assert_eq!(resp.status_code(), 503);
}

#[tokio::test]
async fn tile_with_cache_miss_and_producer_returns_202_with_retry_after() {
    // Q081: when a bake-queue producer is attached and the cache misses,
    // the server enqueues a job and responds 202 + Retry-After instead
    // of the legacy 503.
    use bake_queue::{mock::MockQueue, producer::Producer, Queue};
    use std::sync::Arc;

    let tmp = tempfile::tempdir().expect("tmpdir");
    let queue: Arc<dyn Queue> = Arc::new(MockQueue::new());
    let prod = Producer::new(queue.clone());

    let state = AppState::new(TEST_KEY.to_vec())
        .with_cache_dir(tmp.path().to_path_buf())
        .with_producer(prod, 7, "2026-05-23".into());
    let server = TestServer::new(build_router(state)).expect("axum-test");

    let path = "/v1/tile/15/17128/9656";
    let ts = now_secs();
    let sig = sign_payload(&TEST_KEY, ts, path);
    let resp = server
        .get(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sig)
        .await;
    assert_eq!(resp.status_code(), 202);
    assert_eq!(
        resp.header("retry-after").to_str().unwrap(),
        "8",
        "Retry-After should match Q084 SLA budget"
    );
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "accepted");
    assert_eq!(body["tile_id"], "15/17128/9656");

    // And the job actually landed on the cold stream.
    assert_eq!(
        queue
            .read_one(
                bake_queue::STREAM_COLD,
                bake_queue::GROUP_WORKERS,
                "tap",
                50
            )
            .await
            .unwrap()
            .map(|(_, j)| j.tile_id),
        Some("15/17128/9656".into())
    );
}

#[tokio::test]
async fn tile_signed_for_different_path_returns_401() {
    // Signature is over `<ts>\n<path>`; signing path A then submitting it
    // for path B must fail (defeats replay-across-tiles attacks).
    let server = test_server();
    let ts = now_secs();
    let signed_for = "/v1/tile/15/1/1";
    let target = "/v1/tile/15/2/2";
    let sig = sign_payload(&TEST_KEY, ts, signed_for);

    let resp = server
        .get(target)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sig)
        .await;
    assert_eq!(resp.status_code(), 401);
}
