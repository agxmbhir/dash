use anyhow::{anyhow, Result};
use dotenvy::dotenv;
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub grpc_endpoint: String,
    pub grpc_x_token: String,
    pub json_rpc_url: String,
    pub bot_account: String,
    pub bot_program_id: String,
    pub database_url: String,
    pub commitment: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenv().ok();
        let grpc_endpoint = env::var("GRPC_ENDPOINT").map_err(|e| anyhow!("missing GRPC_ENDPOINT: {e}"))?;
        let grpc_x_token = env::var("GRPC_X_TOKEN").map_err(|e| anyhow!("missing GRPC_X_TOKEN: {e}"))?;
        let json_rpc_url = env::var("JSON_RPC_URL").map_err(|e| anyhow!("missing JSON_RPC_URL: {e}"))?;
        let bot_account = env::var("BOT_ACCOUNT").map_err(|e| anyhow!("missing BOT_ACCOUNT: {e}"))?;
        let bot_program_id = env::var("BOT_PROGRAM_ID").map_err(|e| anyhow!("missing BOT_PROGRAM_ID: {e}"))?;
        let database_url = env::var("DATABASE_URL").map_err(|e| anyhow!("missing DATABASE_URL: {e}"))?;
        let commitment = env::var("COMMITMENT").unwrap_or_else(|_| "confirmed".to_string());
        if !["processed", "confirmed", "finalized"].contains(&commitment.as_str()) {
            return Err(anyhow!("invalid COMMITMENT value"));
        }
        Ok(Self {
            grpc_endpoint,
            grpc_x_token,
            json_rpc_url,
            bot_account,
            bot_program_id,
            database_url,
            commitment,
        })
    }
}
