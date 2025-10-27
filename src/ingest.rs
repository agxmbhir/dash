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
use solana_sdk::transaction::TransactionError;
use bincode::deserialize;

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

fn is_noise_program_id(pid: &str) -> bool {
    matches!(
        pid,
        // System Program
        "11111111111111111111111111111111"
            // Compute Budget Program
            | "ComputeBudget111111111111111111111111111111"
            // SPL Token Program
            | "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            
            // SPL Memo Programs (current and legacy)
            | "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
            | "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo"
         // The bot program ID
            | "MEViEnscUm6tsQRoGd9h6nLQaQspKj7DB2M5FwM3Xvz"
    )
}

async fn stream_once(cfg: &Config, pool: &DbPool) -> Result<()> {
    let mut client = build_client(&cfg.grpc_endpoint, &cfg.grpc_x_token).await?;

    let mut transactions = HashMap::new();
    transactions.insert(
        "txs".to_string(),
        SubscribeRequestFilterTransactions {
            account_include: vec![cfg.bot_program_id.clone()],
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

    fn classify_error_bytes(bytes: &[u8]) -> String {
        if let Ok(tx_err) = deserialize::<TransactionError>(bytes) {
            format!("{:?}", tx_err)
        } else {
            format!("Unknown({:?})", bytes)
        }
    }

    let (success, fee_lamports, compute_units, error_type) = if let Some(meta) = &wrapped.meta {
        let success = meta.err.is_none();
        let fee_lamports = meta.fee as i64;
        // Yellowstone proto exposes compute_units_consumed akin to RPC
        let compute_units = meta
            .compute_units_consumed
            .as_ref()
            .map(|v| *v as i64);
        let error_type = meta.err.as_ref().map(|e| match e.err.as_slice() {
            bytes if !bytes.is_empty() => classify_error_bytes(bytes),
            _ => format!("{:?}", e),
        });
        (success, fee_lamports, compute_units, error_type)
    } else {
        (true, 0, None, None)
    };
    let mut fee_payer_b58 = String::new();
    if let Some(tx) = &wrapped.transaction {
        if let Some(msg) = &tx.message {
            if let Some(first_key) = msg.account_keys.get(0) {
                fee_payer_b58 = bs58::encode(first_key).into_string();
            }
        }
    }
    // Aggregate instruction program IDs used
    let mut program_counts: HashMap<String, i32> = HashMap::new();
    // Determine arbitrage success based on bot logs
    let mut arbitrage_success: Option<bool> = None;
    if let Some(tx) = &wrapped.transaction {
        if let Some(msg) = &tx.message {
            let account_keys = &msg.account_keys;
            for inst in &msg.instructions {
                let idx = inst.program_id_index as usize;
                if let Some(program_key) = account_keys.get(idx) {
                    let pid = bs58::encode(program_key).into_string();
                    if is_noise_program_id(&pid) {
                        continue;
                    }
                    *program_counts.entry(pid).or_insert(0) += 1;
                }
            }
        }
    }
    // Parse logs when available to detect MEV bot outcomes
    if let Some(meta) = &wrapped.meta {
        // Explicit negative signal from bot
        let mut saw_negative = false;
        for logline in &meta.log_messages {
            if logline.contains("No profitable arbitrage opportunity found") {
                saw_negative = true;
                break;
            }
        }
        if saw_negative {
            arbitrage_success = Some(false);
        } else {
            // Look for evidence of DEX activity
            let mut saw_pool_activity = false;
            for logline in &meta.log_messages {
                if logline.contains("Instruction: Swap")
                    || logline.contains("Instruction: SwapBaseInput")
                    || logline.contains("Instruction: TransferChecked")
                {
                    saw_pool_activity = true;
                }
            }
            if saw_pool_activity {
                arbitrage_success = Some(true);
            }
        }
    }
    let block_time = fetch_block_time(&cfg.json_rpc_url, slot).await.ok();
    info!(
        "\n==================== Incoming Transaction ====================\nsignature        : {}\nslot             : {}\nsuccess          : {}\nfee_lamports     : {}\ncompute_units    : {}\nerror_type       : {}\nfee_payer        : {}\nblock_time       : {}\narbitrage_success: {}\n==============================================================\n",
        signature_b58,
        slot,
        success,
        fee_lamports,
        compute_units.map(|v| v.to_string()).unwrap_or_else(|| "n/a".to_string()),
        error_type.clone().unwrap_or_else(|| "n/a".to_string()),
        fee_payer_b58,
        block_time
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "n/a".to_string()),
        arbitrage_success.map(|v| v.to_string()).unwrap_or_else(|| "n/a".to_string()),
    );
    crate::db::upsert_burn(
        pool,
        &signature_b58,
        slot,
        success,
        fee_lamports,
        &fee_payer_b58,
        block_time,
        compute_units,
        arbitrage_success,
    )
    .await?;
    // Write failure cause when present
    if !success {
        if let Some(err_ty) = error_type.as_deref() {
            crate::db::upsert_tx_failure(pool, &signature_b58, err_ty, slot, block_time).await?;
        }
    }
    // Write instruction program counts
    if !program_counts.is_empty() {
        let mut vec_counts: Vec<(String, i32)> = Vec::with_capacity(program_counts.len());
        for (k, v) in program_counts.into_iter() {
            vec_counts.push((k, v));
        }
        crate::db::upsert_tx_instructions(pool, &signature_b58, &vec_counts).await?;
    }
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
