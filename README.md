## dash-indexer (Part 1 - Ingest)

Ingest Solana Yellowstone gRPC transaction updates for the competitor bot program and store burn (fee) data in Postgres.

### Run locally

1. Copy `.env.example` to `.env` and set `DATABASE_URL`.
2. Ensure Postgres is running and reachable by `DATABASE_URL`.
3. Build and run:

```
cargo run --release
```

The service will:
- subscribe to Yellowstone gRPC filtered by `BOT_PROGRAM_ID`
- parse transaction fee (lamports), success, slot, signature, fee payer
- fetch block time via JSON-RPC
- upsert into `burns` table

### Environment

Set these variables in `.env`:

```
GRPC_ENDPOINT=https://temporal.rpcpool.com
GRPC_X_TOKEN=4586b536-c930-43f1-91c1-bee31ab3a0a2
JSON_RPC_URL=https://temporal.rpcpool.com/4586b536-c930-43f1-91c1-bee31ab3a0a2
BOT_PROGRAM_ID=MEViEnscUm6tsQRoGd9h6nLQaQspKj7DB2M5FwM3Xvz
BOT_ACCOUNT=MEViEnscUm6tsQRoGd9h6nLQaQspKj7DB2M5FwM3Xvz
DATABASE_URL=postgres://user:pass@host/db
COMMITMENT=confirmed
```

Notes:
- `BOT_ACCOUNT` is used to filter relevant transactions by fee payer or any signer matching this pubkey; both `fee_lamports` and `fee` store the lamports charged (burn).

### Schema

```
CREATE TABLE IF NOT EXISTS burns (
  signature TEXT PRIMARY KEY,
  slot BIGINT NOT NULL,
  success BOOLEAN NOT NULL,
  fee_lamports BIGINT NOT NULL,
  fee BIGINT NOT NULL,
  fee_payer TEXT NOT NULL,
  block_time TIMESTAMPTZ NULL,
  ingest_ts TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Notes
- Preferred yellowstone grpc for data streaming but there was no service offering a free tier. 


# dash
