//! `wb-db` — Worldbuilders database access layer.
//!
//! Wraps the sqlx Postgres pool and embeds the migration set so the
//! bake-server binary can run migrations on startup against its dedicated
//! `worldbuilders` database on 127.0.0.1:5432. NEVER touch /srv/shared/.
//!
//! See `docs/db-schema.md` and `docs/grill/q085-postgis-schema.md`.

use anyhow::{Context, Result};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Migrations embedded at compile time from `backend/db/migrations/`.
/// Used by both the bake-server startup hook and by integration tests so
/// the testcontainers-based suite exercises the *same* schema as production.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Connect to Postgres with worldbuilders-tuned pool options.
///
/// Defaults:
///   max_connections = 16   (bake-server is bursty; pgbouncer fronts production)
///   connect_timeout = 5 s  (fail fast; bake-server retries via systemd Restart=)
///   statement_timeout = 30 s set on the role via db-create.sh
pub async fn connect(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(16)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .with_context(|| format!("connecting to Postgres at {database_url}"))
}

/// Run all pending migrations. Idempotent; safe to call on every boot.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("running sqlx migrations against worldbuilders DB")?;
    Ok(())
}

/// Read DATABASE_URL from the environment (loaded from backend/db/.env by the
/// shell scripts, or supplied by the systemd unit in production).
pub fn database_url_from_env() -> Result<String> {
    std::env::var("DATABASE_URL").context("DATABASE_URL not set; run backend/scripts/db-create.sh")
}
