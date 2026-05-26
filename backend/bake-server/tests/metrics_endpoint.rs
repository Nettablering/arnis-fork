//! Integration test: hit `/metrics` after a sample workload and assert the
//! Prometheus exposition contains populated histograms + counters (Q097).

use axum_test::TestServer;
use bake_server::{build_router, now_secs, sign_payload, AppState};

const TEST_KEY: [u8; 32] = [0xCD; 32];

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_exposition_after_workload() {
    let state = AppState::new(TEST_KEY.to_vec()).with_universe("itest");
    let app = build_router(state);
    let server = TestServer::new(app).expect("axum-test");

    // Sample workload: 5 signed tile fetches (legacy mock path → pinned tier).
    for i in 0..5 {
        let path = format!("/v1/tile/15/{i}/1");
        let ts = now_secs();
        let sig = sign_payload(&TEST_KEY, ts, &path);
        let resp = server
            .get(&path)
            .add_header("x-wb-ts", ts.to_string())
            .add_header("x-wb-sig", sig)
            .await;
        assert_eq!(resp.status_code(), 200);
    }

    // And one unsigned request to populate the 5xx-class counter family
    // (returns 401 → 4xx, not 5xx, but still exercises observe_response).
    server.get("/v1/tile/15/0/0").await;

    // Scrape /metrics.
    let metrics = server.get("/metrics").await;
    metrics.assert_status_ok();
    let body = metrics.text();

    // Content-Type must be Prometheus exposition text.
    let ct = metrics.header("content-type");
    let ct_str = ct.to_str().unwrap();
    assert!(
        ct_str.starts_with("text/plain"),
        "expected text/plain content-type, got {ct_str}"
    );

    // Histograms + counters populated by the workload.
    assert!(
        body.contains("wb_tile_fetch_duration_seconds_bucket"),
        "missing fetch histogram in /metrics body:\n{body}"
    );
    assert!(
        body.contains("wb_tile_fetch_duration_seconds_count"),
        "missing fetch count in /metrics body"
    );
    assert!(
        body.contains("universe=\"itest\""),
        "missing universe label in /metrics body"
    );
    assert!(
        body.contains("wb_api_requests_total"),
        "missing api_requests counter family"
    );
    // 5 OK + 1 401 (4xx).
    assert!(
        body.contains("result=\"2xx\"") && body.contains("result=\"4xx\""),
        "expected 2xx + 4xx classes recorded"
    );
}
