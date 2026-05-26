//! Q081 end-to-end smoke: enqueue via HTTP → consume → cache hit on retry.
//!
//! Drives the bake-server router + bake-queue producer/consumer end-to-end
//! using the in-process [`MockQueue`]. No real Redis required.

use axum_test::TestServer;
use bake_queue::{
    consumer::{BakeHandler, Consumer, ConsumerConfig},
    mock::MockQueue,
    producer::Producer,
    BakeJob, Queue,
};
use bake_server::{build_router, now_secs, sign_payload, AppState};
use std::path::PathBuf;
use std::sync::Arc;

const TEST_KEY: [u8; 32] = [0x42; 32];

struct FsManifestHandler {
    cache_dir: PathBuf,
}
#[async_trait::async_trait]
impl BakeHandler for FsManifestHandler {
    async fn bake(&self, job: &BakeJob) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.cache_dir).await?;
        let manifest = serde_json::json!({
            "manifest_version": "1.0",
            "tile_id": format!("{}-{}-{}", job.z, job.x, job.y),
            "z": job.z, "x": job.x, "y": job.y,
            "style_version": job.style_version.to_string(),
            "osm_snapshot": job.osm_snapshot,
            "terrain": {"grid_size": 0, "heights": []},
            "buildings": [], "roads": [], "water": [],
        });
        tokio::fs::write(
            self.cache_dir.join(job.manifest_filename()),
            serde_json::to_vec(&manifest)?,
        )
        .await?;
        Ok(())
    }
}

#[tokio::test]
async fn q081_e2e_miss_enqueue_consume_then_hit() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().to_path_buf();

    // 1. Wire bake-server with cache dir + bake-queue producer.
    let queue: Arc<dyn Queue> = Arc::new(MockQueue::new());
    let producer = Producer::new(queue.clone());
    let state = AppState::new(TEST_KEY.to_vec())
        .with_cache_dir(cache_dir.clone())
        .with_producer(producer, 7, "2026-05-23".into());
    let server = TestServer::new(build_router(state)).expect("axum-test");

    let path = "/v1/tile/15/17128/9656";
    let sig = |ts: u64| sign_payload(&TEST_KEY, ts, path);

    // 2. First request: cache miss → 202 + Retry-After + enqueue.
    let ts1 = now_secs();
    let r1 = server
        .get(path)
        .add_header("x-wb-ts", ts1.to_string())
        .add_header("x-wb-sig", sig(ts1))
        .await;
    assert_eq!(r1.status_code(), 202);
    assert_eq!(r1.header("retry-after").to_str().unwrap(), "8");

    // 3. Run the worker for one job.
    let handler = Arc::new(FsManifestHandler {
        cache_dir: cache_dir.clone(),
    });
    let consumer = Consumer::new(
        queue.clone(),
        handler,
        ConsumerConfig {
            consumer_name: "worker-e2e".into(),
            block_ms: 100,
            ..Default::default()
        },
    );
    let processed = consumer.run_for(1).await.unwrap();
    assert_eq!(processed, 1);

    // 4. Manifest now on disk → retry hits cache and returns 200.
    let ts2 = now_secs();
    let r2 = server
        .get(path)
        .add_header("x-wb-ts", ts2.to_string())
        .add_header("x-wb-sig", sig(ts2))
        .await;
    r2.assert_status_ok();
    let body: serde_json::Value = r2.json();
    assert_eq!(body["tile_id"], "15-17128-9656");
    assert_eq!(body["style_version"], "7");
}
