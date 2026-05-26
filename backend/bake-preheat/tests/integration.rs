//! End-to-end integration test for Q083 preheat:
//!   pinned-set + landmarks  →  preheat runner  →  hot-stream
//!   →  bake-queue worker writes manifests.

use bake_preheat::{
    landmarks::{Landmark, SeedSource},
    pinned::InMemoryPinnedReader,
    LandmarkManifest, PreheatRunner, Tier,
};
use bake_queue::{
    consumer::{BakeHandler, Consumer, ConsumerConfig},
    mock::MockQueue,
    producer::Producer,
    BakeJob, Queue, STREAM_HOT,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct FsManifestHandler {
    cache_dir: PathBuf,
    seen: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl BakeHandler for FsManifestHandler {
    async fn bake(&self, job: &BakeJob) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.cache_dir).await?;
        let path = self.cache_dir.join(job.manifest_filename());
        let body = serde_json::json!({
            "tile_id": job.tile_id,
            "z": job.z, "x": job.x, "y": job.y,
            "osm_snapshot": job.osm_snapshot,
            "style_version": job.style_version,
            "priority": job.priority.as_str(),
        });
        tokio::fs::write(&path, serde_json::to_vec(&body)?).await?;
        self.seen.lock().unwrap().push(job.tile_id.clone());
        Ok(())
    }
}

fn small_manifest() -> LandmarkManifest {
    LandmarkManifest {
        landmarks: vec![
            Landmark {
                name: "Eiffel Tower".into(),
                lat: 48.8584,
                lon: 2.2945,
                seed_source: SeedSource::Curated,
                z: None,
                x: None,
                y: None,
            },
            Landmark {
                name: "Sydney Opera".into(),
                lat: -33.8568,
                lon: 151.2153,
                seed_source: SeedSource::Curated,
                z: None,
                x: None,
                y: None,
            },
        ],
    }
}

#[tokio::test]
async fn preheat_enqueues_to_hot_and_worker_writes_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().to_path_buf();

    // 1. Producer side: shared MockQueue for both preheat (producer) and
    //    worker (consumer) — matches Q081 cross-stream contract.
    let mock = Arc::new(MockQueue::new());
    let q: Arc<dyn Queue> = mock.clone();
    let producer = Producer::new(q.clone());

    // 2. Pinned reader returns one tile that doesn't collide with the
    //    landmarks list — verifies dedupe across sources.
    let pinned = Arc::new(InMemoryPinnedReader::new(vec!["15/9647/12318".into()]));
    let manifest = small_manifest();

    // 3. Run the preheat runner against the mock backend.
    let runner = PreheatRunner::new(Tier::Both)
        .with_dry_run(false)
        .with_rate_limit(1000)
        .with_style(7)
        .with_osm_snapshot("integration-test");
    let outcome = runner.run(pinned, &manifest, &producer).await.unwrap();

    assert_eq!(outcome.deduped, 3, "1 pinned + 2 landmarks, no overlap");
    assert_eq!(outcome.enqueued, 3, "all fresh; no idempotency hits");
    assert_eq!(
        mock.backlog_len(STREAM_HOT).await,
        3,
        "preheat must target wb:bake.requests.hot"
    );

    // 4. Consumer side: stand up a Q081 worker against the same queue
    //    and let it process all three jobs.
    let handler = Arc::new(FsManifestHandler {
        cache_dir: cache_dir.clone(),
        seen: Mutex::new(vec![]),
    });
    let consumer = Consumer::new(
        q.clone(),
        handler.clone(),
        ConsumerConfig {
            consumer_name: "preheat-itest".into(),
            block_ms: 50,
            ..Default::default()
        },
    );
    let processed = consumer.run_for(3).await.unwrap();
    assert_eq!(processed, 3);

    // 5. Three manifests on disk, all priority=hot-preheat.
    let seen = handler.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    for tid in &seen {
        let parts: Vec<&str> = tid.split('/').collect();
        let fname = format!("{}-{}-{}.json", parts[0], parts[1], parts[2]);
        let path = cache_dir.join(&fname);
        assert!(path.exists(), "manifest missing for {tid}");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["priority"], "hot-preheat");
        assert_eq!(v["osm_snapshot"], "integration-test");
    }
}

#[tokio::test]
async fn bundled_manifest_drives_dry_run_intents_around_1000() {
    let manifest = LandmarkManifest::bundled().expect("manifest parses");
    assert!(manifest.len() >= 1000);

    let pinned = Arc::new(InMemoryPinnedReader::default());
    let runner = PreheatRunner::new(Tier::Landmarks).with_dry_run(true);

    // Producer is a no-op on dry-run but required by signature.
    let q: Arc<dyn Queue> = Arc::new(MockQueue::new());
    let producer = Producer::new(q);
    let out = runner.run(pinned, &manifest, &producer).await.unwrap();
    assert!(
        out.enqueued >= 950,
        "dry-run should report ~1000 enqueue intents, got {}",
        out.enqueued
    );
    assert!(out.dry_run);
}
