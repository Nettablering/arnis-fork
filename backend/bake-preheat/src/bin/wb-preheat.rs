//! `wb-preheat` — Q083 preheat cron CLI.
//!
//! Reads the pinned-tile sorted set (Q082) and the bundled landmarks
//! manifest, dedupes, and enqueues `Priority::HotPreheat` bake jobs to
//! `wb:bake.requests.hot` for the worker pool (Q081) to consume.
//!
//! Runs as a one-shot from `systemd/wb-preheat.timer` (hourly) or
//! manually for ops triage.

use bake_preheat::{
    LandmarkManifest, PreheatRunner, RedisPinnedReader, Tier,
    DEFAULT_OSM_SNAPSHOT, DEFAULT_RATE_LIMIT, DEFAULT_STYLE_VERSION,
};
use bake_queue::{redis::RedisQueue, Producer, Queue};
use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "wb-preheat",
    about = "Worldbuilders hot-tile preheat (Q083). Reads wb:cache.hot.tiles + landmarks.toml, enqueues to wb:bake.requests.hot."
)]
struct Cli {
    /// Redis connection URL.
    #[arg(long, env = "WB_REDIS_URL", default_value = "redis://127.0.0.1:6379/")]
    redis_url: String,

    /// Which tier to walk.
    #[arg(long, value_enum, default_value_t = Tier::Both)]
    tier: Tier,

    /// Style version embedded into each enqueued job.
    #[arg(long, env = "WB_STYLE_VERSION", default_value_t = DEFAULT_STYLE_VERSION)]
    style_version: u32,

    /// OSM snapshot tag embedded into each enqueued job.
    #[arg(long, env = "WB_OSM_SNAPSHOT", default_value_t = DEFAULT_OSM_SNAPSHOT.to_string())]
    osm_snapshot: String,

    /// Cap on enqueues per second.
    #[arg(long, default_value_t = DEFAULT_RATE_LIMIT)]
    rate_limit: u32,

    /// Plan-only: log enqueue intents but do not write to Redis.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let manifest = LandmarkManifest::bundled()?;
    tracing::info!(
        bundled_landmarks = manifest.len(),
        curated = manifest.curated_count(),
        "loaded landmarks manifest"
    );

    let runner = PreheatRunner::new(cli.tier)
        .with_dry_run(cli.dry_run)
        .with_rate_limit(cli.rate_limit)
        .with_style(cli.style_version)
        .with_osm_snapshot(cli.osm_snapshot.clone());

    // Dry-run path: never touch Redis. Wire an empty pinned reader so
    // the CLI works on a developer laptop without redis-server.
    if cli.dry_run {
        let pinned = Arc::new(bake_preheat::pinned::InMemoryPinnedReader::default());
        // Producer is still needed by the signature but won't be hit.
        let q: Arc<dyn Queue> = Arc::new(bake_queue::mock::MockQueue::new());
        let p = Producer::new(q);
        let out = runner.run(pinned, &manifest, &p).await?;
        tracing::info!(
            candidates = out.candidates,
            deduped = out.deduped,
            enqueued = out.enqueued,
            "dry-run done"
        );
        return Ok(());
    }

    // Real run — connect to Redis on both sides.
    let pinned = Arc::new(RedisPinnedReader::connect(&cli.redis_url).await?);
    let q: Arc<dyn Queue> = Arc::new(
        RedisQueue::connect(&cli.redis_url)
            .await
            .map_err(|e| anyhow::anyhow!("connect bake-queue redis: {e}"))?,
    );
    let producer = Producer::new(q);
    let out = runner.run(pinned, &manifest, &producer).await?;
    tracing::info!(
        candidates = out.candidates,
        deduped = out.deduped,
        enqueued = out.enqueued,
        skipped_idempotent = out.skipped_idempotent,
        "preheat run finished"
    );
    Ok(())
}
