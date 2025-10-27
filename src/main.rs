use anyhow::Result;
use log::{error, info};
use tokio::signal;

mod config;
mod db;
mod ingest;

#[tokio::main]
async fn main() -> Result<()> {
    // Ensure logs are visible by default
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::init();

    let cfg = config::Config::from_env()?;
    info!("starting dash-indexer");

    let pool = db::init_pool(&cfg.database_url).await?;
    db::ensure_schema(&pool).await?;

    let ingest_handle = tokio::spawn(ingest::run(cfg.clone(), pool.clone()));

    signal::ctrl_c().await.ok();
    info!("shutdown signal received");

    if let Err(e) = ingest_handle.await {
        error!("ingest task join error: {e:?}");
    }
    Ok(())
}
