//! Database initialization

use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use tracing::{error, info};

use crate::Config;

/// Initialize database connection pool
///
/// Note: Migrations should be run separately by the binary crate.
pub async fn init_database(config: &Config) -> Result<PgPool> {
    let database_url = config.database_url();

    info!("Connecting to database: {}", config.database_url());

    let pool: PgPool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .min_connections(config.database.min_connections)
        .acquire_timeout(Duration::from_secs(config.database.connect_timeout_seconds))
        .idle_timeout(Duration::from_secs(config.database.idle_timeout_seconds))
        .connect(database_url)
        .await
        .map_err(|e| {
            error!("Failed to connect to database: {}", e);
            anyhow::anyhow!("Database connection failed: {}", e)
        })?;

    info!("Database connected successfully");

    Ok(pool)
}
