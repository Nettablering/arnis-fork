//! `wb-invite` — CLI for the pre-launch invite-code system (Q243).
//!
//! Subcommands:
//!   wb-invite create --count 50 --batch alpha-wave-1 [--stage 1] [--dry-run]
//!   wb-invite redeem --code XXXXXXXXXXXX --roblox-user-id 12345
//!   wb-invite list   --batch alpha-wave-1 [--unredeemed-only]
//!   wb-invite migrate                       (apply ../migrations against $DATABASE_URL)
//!
//! Connection: reads $DATABASE_URL. When unset, `create` still runs in
//! `--dry-run` mode so operators can sanity-check a batch without a DB.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use wb_invite::{generate_batch, persist_batch, redeem_code, Stage, MIGRATOR};

#[derive(Parser)]
#[command(name = "wb-invite", version, about = "Worldbuilders pre-launch invite-code minting + redemption (Q243)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Mint a batch of fresh invite codes and persist them.
    Create {
        #[arg(long, default_value_t = 10)]
        count: usize,
        #[arg(long)]
        batch: String,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(i16).range(1..=3))]
        stage: i16,
        /// Print codes; do not write to Postgres. Useful without $DATABASE_URL.
        #[arg(long)]
        dry_run: bool,
    },
    /// Redeem a code on behalf of a Roblox user id.
    Redeem {
        #[arg(long)]
        code: String,
        #[arg(long)]
        roblox_user_id: i64,
    },
    /// List codes in a batch (or all batches).
    List {
        #[arg(long)]
        batch: Option<String>,
        #[arg(long)]
        unredeemed_only: bool,
    },
    /// Apply the alpha-program migrations against $DATABASE_URL.
    Migrate,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Create { count, batch, stage, dry_run } => {
            if count == 0 {
                return Err(anyhow!("--count must be > 0"));
            }
            let codes = generate_batch(count);
            if dry_run {
                eprintln!("[dry-run] {count} codes for batch '{batch}', stage {stage}:");
                for c in &codes {
                    println!("{c}");
                }
                return Ok(());
            }
            let pool = connect().await?;
            let stage = Stage::from_i16(stage)?;
            let inserted = persist_batch(&pool, &codes, &batch, stage).await?;
            eprintln!("[wb-invite] inserted {inserted}/{count} codes into batch '{batch}'");
            for c in &codes {
                println!("{c}");
            }
        }
        Cmd::Redeem { code, roblox_user_id } => {
            let pool = connect().await?;
            let stage = redeem_code(&pool, &code, roblox_user_id).await?;
            println!("redeemed: stage={:?} user={roblox_user_id}", stage);
        }
        Cmd::List { batch, unredeemed_only } => {
            let pool = connect().await?;
            let rows: Vec<(String, String, i16, Option<chrono::DateTime<chrono::Utc>>, Option<i64>)> =
                sqlx::query_as(
                    "SELECT code, batch, stage, redeemed_at, redeemed_by_user_id
                     FROM wb.invite_codes
                     WHERE ($1::text IS NULL OR batch = $1)
                       AND ($2 = false OR redeemed_at IS NULL)
                     ORDER BY created_at",
                )
                .bind(batch)
                .bind(unredeemed_only)
                .fetch_all(&pool)
                .await?;
            for (code, b, stage, redeemed_at, user) in rows {
                let status = match (redeemed_at, user) {
                    (Some(t), Some(u)) => format!("redeemed at {t} by {u}"),
                    _ => "open".to_string(),
                };
                println!("{code}  batch={b}  stage={stage}  {status}");
            }
        }
        Cmd::Migrate => {
            let pool = connect().await?;
            MIGRATOR
                .run(&pool)
                .await
                .context("running wb-invite migrations")?;
            eprintln!("[wb-invite] migrations applied");
        }
    }

    Ok(())
}

async fn connect() -> Result<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL not set (required for non --dry-run paths)")?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
        .context("connecting to Postgres")
}
