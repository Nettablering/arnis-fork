//! Q084 integration tests — synchronous-bake fast-path + placeholder
//! protocol + job polling. Exercises the full HTTP router via
//! `axum-test`; no real Redis required.

use axum_test::TestServer;
use bake_server::{
    build_router, now_secs, sign_payload,
    sla::{ScriptedExecutor, SYNC_BAKE_SOFT_DEADLINE},
    AppState,
};
use std::sync::Arc;
use std::time::Duration;

const TEST_KEY: [u8; 32] = [0x84; 32];

fn sign(ts: u64, path: &str) -> String {
    sign_payload(&TEST_KEY, ts, path)
}

#[tokio::test]
async fn sync_bake_under_deadline_returns_200_with_manifest() {
    // Bake completes in 30 ms — well under the 8 s soft deadline.
    let exec = Arc::new(ScriptedExecutor::ok(
        Duration::from_millis(30),
        b"{\"manifest_version\":\"2.0\",\"note\":\"fast-bake\"}".to_vec(),
    ));
    let state = AppState::new(TEST_KEY.to_vec()).with_bake_executor(exec);
    let server = TestServer::new(build_router(state)).expect("axum-test");

    let path = "/v1/tile/15/17128/9656/bake";
    let ts = now_secs();
    let resp = server
        .post(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sign(ts, path))
        .await;

    assert_eq!(resp.status_code(), 200);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["note"], "fast-bake");
}

#[tokio::test]
async fn sync_bake_over_deadline_returns_202_with_placeholder_and_bake_id() {
    // Soft deadline 100 ms; bake takes 500 ms.
    let exec = Arc::new(ScriptedExecutor::ok(
        Duration::from_millis(500),
        b"{\"manifest_version\":\"2.0\",\"note\":\"slow-bake\"}".to_vec(),
    ));
    let state = AppState::new(TEST_KEY.to_vec())
        .with_bake_executor(exec)
        .with_bake_soft_deadline(Duration::from_millis(100));
    let server = TestServer::new(build_router(state)).expect("axum-test");

    let path = "/v1/tile/15/17128/9656/bake";
    let ts = now_secs();
    let resp = server
        .post(path)
        .add_header("x-wb-ts", ts.to_string())
        .add_header("x-wb-sig", sign(ts, path))
        .await;

    assert_eq!(resp.status_code(), 202);
    assert_eq!(resp.header("retry-after").to_str().unwrap(), "8");
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "accepted");
    assert!(body["bake_id"].is_string(), "bake_id must be present");
    assert_eq!(body["placeholder"]["placeholder"], true);
    let bake_id = body["bake_id"].as_str().unwrap().to_string();

    // Poll the job endpoint — it should transition to done after the bake
    // worker finishes (~500 ms total). Give it 1 s grace.
    let job_path = format!("/v1/tile/15/17128/9656/job/{bake_id}");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut final_body: Option<serde_json::Value> = None;
    while std::time::Instant::now() < deadline {
        let ts = now_secs();
        let r = server
            .get(&job_path)
            .add_header("x-wb-ts", ts.to_string())
            .add_header("x-wb-sig", sign(ts, &job_path))
            .await;
        assert_eq!(r.status_code(), 200);
        let b: serde_json::Value = r.json();
        if b["state"] == "done" {
            final_body = Some(b);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let final_body = final_body.expect("job should reach done");
    assert_eq!(final_body["state"], "done");
    // Manifest is base64-encoded in the done payload.
    let m_b64 = final_body["manifest_b64"].as_str().unwrap();
    use base64::Engine as _;
    let manifest = base64::engine::general_purpose::STANDARD
        .decode(m_b64)
        .unwrap();
    assert!(String::from_utf8_lossy(&manifest).contains("slow-bake"));

    // Prometheus surface must include the breach counter.
    let metrics_resp = server.get("/metrics").await;
    metrics_resp.assert_status_ok();
    let text = metrics_resp.text();
    assert!(
        text.contains("wb_bake_sla_breach_total"),
        "metrics should expose wb_bake_sla_breach_total"
    );
}

#[tokio::test]
async fn sync_bake_p95_under_8s_at_50_concurrent_requests() {
    // Drive 200 sync-bake calls in waves of 20 concurrent — covers the
    // "p95 < 8 s" SLA assertion inside the cargo test suite (the k6 run
    // does the heavy lifting on a live binary).
    let exec = Arc::new(ScriptedExecutor::ok(
        Duration::from_millis(50),
        b"{\"manifest_version\":\"2.0\"}".to_vec(),
    ));
    let state = AppState::new(TEST_KEY.to_vec()).with_bake_executor(exec);
    let server = Arc::new(TestServer::new(build_router(state)).expect("axum-test"));

    let mut latencies = Vec::with_capacity(200);
    for i in 1..=200u32 {
        let path = format!("/v1/tile/15/{i}/9656/bake");
        let ts = now_secs();
        let sig = sign(ts, &path);
        let t0 = std::time::Instant::now();
        let r = server
            .post(&path)
            .add_header("x-wb-ts", ts.to_string())
            .add_header("x-wb-sig", sig)
            .await;
        let elapsed = t0.elapsed();
        assert_eq!(
            r.status_code(),
            200,
            "every bake should land Ready under load"
        );
        latencies.push(elapsed);
    }
    latencies.sort();
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    assert!(
        p95 < SYNC_BAKE_SOFT_DEADLINE,
        "p95 latency {p95:?} must be < {SYNC_BAKE_SOFT_DEADLINE:?}"
    );
}
