//! wb-backup — nightly entrypoint invoked by the systemd timer.
//!
//! Usage:
//!   wb-backup --universe-id 12345 [--date 2026-05-26]
//!
//! In the absence of OPEN_CLOUD_API_KEY we use the [`SyntheticClient`] so the
//! nightly job still rehearses the end-to-end pipeline (encrypt/store/manifest)
//! before any universe is enrolled.

use anyhow::Result;
use chrono::NaiveDate;
use clap::Parser;
use datastore_backup::{
    backup::{run_backup, today_utc, BackupConfig},
    roblox::{OpenCloudClient, OpenCloudHttp, SyntheticClient},
    storage::{LocalStorage, StorageTarget},
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "wb-backup")]
struct Args {
    #[arg(long, default_value_t = 0)]
    universe_id: u64,
    #[arg(long)]
    date: Option<NaiveDate>,
    #[arg(long, env = "WB_BACKUP_AGE_PUB",
          default_value = "/home/deploy/.claude/shared/api-keys/worldbuilders-backup-age.pub")]
    age_pub: PathBuf,
    #[arg(long, env = "WB_BACKUP_LOCAL_ROOT",
          default_value = "/home/deploy/projects/worldbuilders/backups")]
    local_root: PathBuf,
    #[arg(long, env = "WB_BACKUP_WORKDIR",
          default_value = "/home/deploy/projects/worldbuilders/backups/_work")]
    work_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let args = Args::parse();
    std::fs::create_dir_all(&args.work_dir)?;

    let client: Box<dyn OpenCloudClient> = match std::env::var("OPEN_CLOUD_API_KEY") {
        Ok(k) if !k.is_empty() => Box::new(OpenCloudHttp::new(k)),
        _ => {
            tracing::warn!("OPEN_CLOUD_API_KEY unset — running rehearsal with synthetic client");
            Box::new(SyntheticClient::ten_key_fixture(
                args.universe_id.max(1),
            ))
        }
    };

    // Remote targets (Hetzner / B2) not yet provisioned — see BLOCKED/needs-human.md.
    let targets: Vec<Arc<dyn StorageTarget>> = vec![Arc::new(LocalStorage::new(args.local_root))];

    let outcome = run_backup(
        client.as_ref(),
        BackupConfig {
            universe_id: args.universe_id.max(1),
            date: args.date.unwrap_or_else(today_utc),
            recipient_pub: args.age_pub,
            targets,
            work_dir: args.work_dir,
        },
    )
    .await?;

    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "universe_id": outcome.universe_id,
        "date": outcome.date.to_string(),
        "entry_count": outcome.entry_count,
        "archive_bytes": outcome.archive_bytes,
        "manifest_bytes": outcome.manifest_bytes,
        "targets_ok": outcome.targets_ok,
        "targets_failed": outcome.targets_failed,
    }))?);
    Ok(())
}
