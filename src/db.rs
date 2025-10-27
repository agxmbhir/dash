use anyhow::Result;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::Config as PgConfig;
use tokio_postgres::config::ChannelBinding;
use tokio_postgres_rustls::MakeRustlsConnect;
use rustls::{ClientConfig, RootCertStore};
use rustls_native_certs::load_native_certs;

pub type DbPool = Pool;

pub async fn init_pool(database_url: &str) -> Result<DbPool> {
    let mut pg_config: PgConfig = database_url.parse()?;
    pg_config.channel_binding(ChannelBinding::Disable);

    let use_tls = database_url.contains("sslmode=require")
        || database_url.contains("sslmode=verify-ca")
        || database_url.contains("sslmode=verify-full");

    let mgr = if use_tls {
        let mut roots = RootCertStore::empty();
        let store = load_native_certs();
        let _ = roots.add_parsable_certificates(store.certs);
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tls = MakeRustlsConnect::new(config);
        Manager::from_config(
            pg_config,
            tls,
            ManagerConfig { recycling_method: RecyclingMethod::Fast },
        )
    } else {
        Manager::from_config(
            pg_config,
            tokio_postgres::NoTls,
            ManagerConfig { recycling_method: RecyclingMethod::Fast },
        )
    };
    let pool = Pool::builder(mgr).max_size(8).build().unwrap();
    Ok(pool)
}

pub async fn ensure_schema(pool: &DbPool) -> Result<()> {
    let client = pool.get().await?;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS burns (
               signature TEXT PRIMARY KEY,
               slot BIGINT NOT NULL,
               success BOOLEAN NOT NULL,
               fee_lamports BIGINT NOT NULL,
               fee_payer TEXT NOT NULL,
               block_time TIMESTAMPTZ NULL,
               ingest_ts TIMESTAMPTZ NOT NULL DEFAULT NOW()
             );
             ALTER TABLE IF EXISTS burns ADD COLUMN IF NOT EXISTS compute_units BIGINT;
             ALTER TABLE IF EXISTS burns ADD COLUMN IF NOT EXISTS arbitrage_success BOOLEAN;
             CREATE TABLE IF NOT EXISTS tx_failures (
               signature TEXT PRIMARY KEY,
               error_type TEXT NOT NULL,
               slot BIGINT NOT NULL,
               ts TIMESTAMPTZ NOT NULL DEFAULT NOW()
             );
             CREATE TABLE IF NOT EXISTS tx_instructions (
               signature TEXT NOT NULL,
               program_id TEXT NOT NULL,
               num_instructions INT NOT NULL,
               PRIMARY KEY(signature, program_id)
             );",
        )
        .await?;
    Ok(())
}

pub async fn upsert_burn(
    pool: &DbPool,
    signature: &str,
    slot: i64,
    success: bool,
    fee_lamports: i64,
    fee_payer: &str,
    block_time: Option<chrono::DateTime<chrono::Utc>>,
    compute_units: Option<i64>,
    arbitrage_success: Option<bool>,
) -> Result<()> {
    let client = pool.get().await?;
    let stmt = client
        .prepare(
            "INSERT INTO burns(signature, slot, success, fee_lamports, fee_payer, block_time, compute_units, arbitrage_success)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (signature) DO UPDATE SET
               slot = EXCLUDED.slot,
               success = EXCLUDED.success,
               fee_lamports = EXCLUDED.fee_lamports,
               fee_payer = EXCLUDED.fee_payer,
               block_time = COALESCE(EXCLUDED.block_time, burns.block_time),
               compute_units = COALESCE(EXCLUDED.compute_units, burns.compute_units),
               arbitrage_success = COALESCE(EXCLUDED.arbitrage_success, burns.arbitrage_success)",
        )
        .await?;
    client
        .execute(
            &stmt,
            &[
                &signature,
                &slot,
                &success,
                &fee_lamports,
                &fee_payer,
                &block_time,
                &compute_units,
                &arbitrage_success,
            ],
        )
        .await?;
    Ok(())
}

pub async fn upsert_tx_failure(
    pool: &DbPool,
    signature: &str,
    error_type: &str,
    slot: i64,
    ts: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<()> {
    let client = pool.get().await?;
    let stmt = client
        .prepare(
            "INSERT INTO tx_failures(signature, error_type, slot, ts)
             VALUES ($1,$2,$3,COALESCE($4, NOW()))
             ON CONFLICT (signature) DO UPDATE SET
               error_type = EXCLUDED.error_type,
               slot = EXCLUDED.slot,
               ts = EXCLUDED.ts",
        )
        .await?;
    client
        .execute(&stmt, &[&signature, &error_type, &slot, &ts])
        .await?;
    Ok(())
}

pub async fn upsert_tx_instructions(
    pool: &DbPool,
    signature: &str,
    program_counts: &[(String, i32)],
) -> Result<()> {
    if program_counts.is_empty() {
        return Ok(());
    }
    let client = pool.get().await?;
    let stmt = client
        .prepare(
            "INSERT INTO tx_instructions(signature, program_id, num_instructions)
             VALUES ($1,$2,$3)
             ON CONFLICT (signature, program_id) DO UPDATE SET
               num_instructions = EXCLUDED.num_instructions",
        )
        .await?;
    for (program_id, count) in program_counts.iter() {
        client
            .execute(&stmt, &[&signature, program_id, count])
            .await?;
    }
    Ok(())
}
