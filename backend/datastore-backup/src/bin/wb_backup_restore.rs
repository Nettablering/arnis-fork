//! wb-backup-restore — restore an age-encrypted nightly into a local directory.
//!
//! Usage:
//!   wb-backup-restore --date 2026-05-26 --target /tmp/restore-x \
//!                    [--universe-id 1] [--identity-key ~/.../.../*.key]
//!
//! On success the target dir contains the decrypted archive bytes plus an
//! `entries.jsonl` for human inspection.

use anyhow::Result;
use chrono::NaiveDate;
use clap::Parser;
use datastore_backup::{
    restore::{restore_archive, RestoreConfig},
    storage::{LocalStorage, StorageTarget},
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "wb-backup-restore")]
struct Args {
    #[arg(long)]
    date: NaiveDate,
    #[arg(long)]
    target: PathBuf,
    #[arg(long, default_value_t = 1)]
    universe_id: u64,
    #[arg(long, env = "WB_BACKUP_AGE_KEY",
          default_value = "/home/deploy/.claude/shared/api-keys/worldbuilders-backup-age.key")]
    identity_key: PathBuf,
    #[arg(long, env = "WB_BACKUP_LOCAL_ROOT",
          default_value = "/home/deploy/projects/worldbuilders/backups")]
    local_root: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env(),
    ).init();
    let args = Args::parse();
    let source: Arc<dyn StorageTarget> = Arc::new(LocalStorage::new(args.local_root));
    let entries = restore_archive(RestoreConfig {
        universe_id: args.universe_id,
        date: args.date,
        identity_key: args.identity_key,
        target_dir: args.target.clone(),
        source,
    })
    .await?;

    // human-readable side-output
    let jsonl_path = args.target.join("entries.jsonl");
    let mut f = std::fs::File::create(&jsonl_path)?;
    for e in &entries {
        use std::io::Write;
        writeln!(f, "{}", serde_json::to_string(e)?)?;
    }
    println!(
        "restored {} entries → {}",
        entries.len(),
        jsonl_path.display()
    );
    Ok(())
}
