//! Worldbuilders bake-server binary entry point (Q470).
//!
//! All HTTP layer code lives in `lib.rs` so it can be exercised by
//! `axum-test` integration tests (Q475). This binary just loads env config,
//! wires logging, and runs the server.

use bake_server::{
    build_router,
    sla::{ScriptedExecutor, SYNC_BAKE_SOFT_DEADLINE},
    AppState,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bake_server=info,tower_http=warn".into()),
        )
        .json()
        .init();

    let hmac_hex = std::env::var("BAKE_HMAC_KEY")
        .map_err(|_| anyhow::anyhow!("BAKE_HMAC_KEY env var missing"))?;
    let hmac_key = hex::decode(hmac_hex.trim())
        .map_err(|e| anyhow::anyhow!("BAKE_HMAC_KEY not valid hex: {e}"))?;
    if hmac_key.len() < 32 {
        anyhow::bail!(
            "BAKE_HMAC_KEY must decode to >=32 bytes, got {}",
            hmac_key.len()
        );
    }

    let bind = std::env::var("BAKE_BIND").unwrap_or_else(|_| "127.0.0.1:9090".into());
    let mut state = AppState::new(hmac_key);
    if let Ok(cache) = std::env::var("BAKE_CACHE_DIR") {
        let p = std::path::PathBuf::from(cache);
        info!(cache=%p.display(), "manifest cache directory configured");
        state = state.with_cache_dir(p);
    }
    // Q081: optional bake-queue producer. When `WB_REDIS_URL` is set,
    // the server enqueues cache-miss jobs and returns 202 + Retry-After.
    if let Ok(url) = std::env::var("WB_REDIS_URL") {
        let queue = std::sync::Arc::new(bake_queue::redis::RedisQueue::connect(&url).await?);
        let producer = bake_queue::producer::Producer::new(queue);
        let style_version: u32 = std::env::var("WB_STYLE_VERSION")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let osm_snapshot = std::env::var("WB_OSM_SNAPSHOT").unwrap_or_else(|_| "unknown".into());
        info!(redis = %url, style_version, %osm_snapshot, "bake-queue producer configured");
        state = state.with_producer(producer, style_version, osm_snapshot);
    }
    // Q084: optional in-process bake executor for staging / k6 SLA load
    // tests. Set `WB_SYNC_BAKE_DEMO_MS=<ms>` to enable; production wires
    // a real queue-backed executor instead.
    if let Ok(ms) = std::env::var("WB_SYNC_BAKE_DEMO_MS") {
        let delay_ms: u64 = ms.parse().unwrap_or(50);
        let payload = serde_json::to_vec(&serde_json::json!({
            "manifest_version": "1.1",
            "note": "demo-sync-bake",
            "delay_ms": delay_ms,
        }))?;
        info!(
            delay_ms,
            soft_deadline_secs = SYNC_BAKE_SOFT_DEADLINE.as_secs(),
            "synchronous-bake demo executor enabled (Q084)"
        );
        state = state.with_bake_executor(Arc::new(ScriptedExecutor::ok(
            Duration::from_millis(delay_ms),
            payload,
        )));
    }
    let app = build_router(state);

    let addr: SocketAddr = bind.parse()?;
    info!(%addr, "bake-server listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
