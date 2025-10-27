use crate::config::Config;
use crate::db::DbPool;
use anyhow::{Context, Result};
use futures::{sink::SinkExt, stream::StreamExt};
use log::{info, warn};
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, SubscribeRequest, SubscribeRequestFilterTransactions,
    SubscribeUpdateTransaction,
};
use yellowstone_grpc_proto::prelude::{CommitmentLevel, SubscribeRequestPing};

pub async fn run(cfg: Config, pool: DbPool) -> Result<()> {
    loop {
        match stream_once(&cfg, &pool).await {
            Ok(_) => {
                warn!("stream ended, restarting in 2s");
                sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                warn!("stream error: {e:?}; backing off 5s");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn build_client(
    endpoint: &str,
    x_token: &str,
) -> Result<GeyserGrpcClient<impl yellowstone_grpc_client::Interceptor>> {
    let mut builder = GeyserGrpcClient::build_from_shared(endpoint.to_string())?
        .x_token(Some(x_token.to_string()))?;
    if endpoint.starts_with("https://") {
        builder = builder.tls_config(ClientTlsConfig::new().with_native_roots())?;
    }
    let client = builder.connect().await?;
    Ok(client)
}

fn commitment_to_i32(c: &str) -> i32 {
    match c {
        "processed" => CommitmentLevel::Processed as i32,
        "finalized" => CommitmentLevel::Finalized as i32,
        _ => CommitmentLevel::Confirmed as i32,
    }   
}

async fn stream_once(cfg: &Config, pool: &DbPool) -> Result<()> {
    let mut client = build_client(&cfg.grpc_endpoint, &cfg.grpc_x_token).await?;

    let mut transactions = HashMap::new();
    transactions.insert(
        "txs".to_string(),
        SubscribeRequestFilterTransactions {
            account_include: vec![cfg.bot_program_id.clone(), cfg.bot_account.clone()],
            vote: Some(false),
            failed: Some(true),
            ..Default::default()
        },
    );

    let subscribe_request = SubscribeRequest {
        transactions,
        commitment: Some(commitment_to_i32(&cfg.commitment)),
        ..Default::default()
    };

    let (mut subscribe_tx, mut stream) = client.subscribe_with_request(Some(subscribe_request)).await?;
    info!(
        "subscribed to transactions for program {}",
        cfg.bot_program_id
    );

    while let Some(msg) = stream.next().await {
        let update = msg?;
        match update.update_oneof {
            Some(UpdateOneof::Transaction(tx)) => {
                if let Err(e) = handle_tx(&cfg, pool, &tx).await {
                    warn!("handle_tx error: {e:?}");
                }
            }
            Some(UpdateOneof::Ping(_)) => {
                subscribe_tx
                    .send(SubscribeRequest {
                        ping: Some(SubscribeRequestPing { id: 1 }),
                        ..Default::default()
                    })
                    .await
                    .ok();
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_tx(cfg: &Config, pool: &DbPool, tx_update: &SubscribeUpdateTransaction) -> Result<()> {
    let Some(wrapped) = &tx_update.transaction else { return Ok(()); };
    let signature_b58 = bs58::encode(&wrapped.signature).into_string();
    let slot = tx_update.slot as i64;

    let (success, fee_lamports) = if let Some(meta) = &wrapped.meta {
        (meta.err.is_none(), meta.fee as i64)
    } else {
        (true, 0)
    };
    let mut fee_payer_b58 = String::new();
    if let Some(tx) = &wrapped.transaction {
        if let Some(msg) = &tx.message {
            if let Some(first_key) = msg.account_keys.get(0) {
                fee_payer_b58 = bs58::encode(first_key).into_string();
            }
        }
    }
    let block_time = fetch_block_time(&cfg.json_rpc_url, slot).await.ok();
    info!(
        "\n==================== Incoming Transaction ====================\nsignature    : {}\nslot         : {}\nsuccess      : {}\nfee_lamports : {}\nfee_payer    : {}\nblock_time   : {}\n==============================================================\n",
        signature_b58,
        slot,
        success,
        fee_lamports,
        fee_payer_b58,
        block_time
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "n/a".to_string()),
    );
    crate::db::upsert_burn(
        pool,
        &signature_b58,
        slot,
        success,
        fee_lamports,
        &fee_payer_b58,
        block_time,
    )
    .await?;
    Ok(())
}

async fn fetch_block_time(rpc_url: &str, slot: i64) -> Result<chrono::DateTime<chrono::Utc>> {
    #[derive(serde::Serialize)]
    struct RpcReq<'a> {
        jsonrpc: &'a str,
        id: u32,
        method: &'a str,
        params: (i64,),
    }
    #[derive(serde::Deserialize)]
    struct RpcResp {
        result: Option<i64>,
    }
    let req = RpcReq { jsonrpc: "2.0", id: 1, method: "getBlockTime", params: (slot,) };
    let resp = reqwest::Client::new()
        .post(rpc_url)
        .json(&req)
        .send()
        .await?
        .error_for_status()?;
    let parsed: RpcResp = resp.json().await?;
    let ts = parsed.result.context("no block time")?;
    Ok(chrono::DateTime::from_timestamp(ts, 0).context("ts convert")?)
}
