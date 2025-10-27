## dash-indexer (Part 1 - Ingest)

Ingest Solana Yellowstone gRPC transaction updates for the competitor bot program and store burn (fee) data in Postgres.

### Run locally

1. Setup the .env, looks src/config.rs for the fields.
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
- record compute units consumed and failure causes
- aggregate instruction program IDs per transaction

### Environment

Set these variables in `.env`:

```
GRPC_ENDPOINT=
GRPC_X_TOKEN=
JSON_RPC_URL=
BOT_PROGRAM_ID=MEViEnscUm6tsQRoGd9h6nLQaQspKj7DB2M5FwM3Xvz
DATABASE_URL=postgres://user:pass@host/db
COMMITMENT=confirmed


### Schema

```
CREATE TABLE IF NOT EXISTS burns (
  signature TEXT PRIMARY KEY,
  slot BIGINT NOT NULL,
  success BOOLEAN NOT NULL,
  fee_lamports BIGINT NOT NULL,
  fee_payer TEXT NOT NULL,
  block_time TIMESTAMPTZ NULL,
  arbitrage_success BOOLEAN,
  compute_units BIGINT,
  ingest_ts TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

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
);
```

### Notes
- Backfilling was a big bottleneck and a shortcut taken. There is currently no backfilling logic, I attempted to use https://docs.triton.one/project-yellowstone/fumarole and just plain rpc requests per block and then filtering for the program and some other ways. None were reliable enough and could go far back. In production i would push for access to https://docs.triton.one/project-yellowstone/fumarole, didn't know if this was enabled in the current endpoint. Would have done this if i had more time. 
- I found there to be an infinite amounts of metrics to be tracked. Example things like tracking validator-level latency, priority-fee density per block, pool-specific arbitrage profitability, or even live mempool competition stats felt like overkill for this scope. They also required a richer access to the internal data. For example, tracking the error codes, it'd have been great to have access to what these internal errors map to eg: custom program error: 0x1. Given the scope i decided to keep some essential metrics that i deep useful for a competitor to track. 
- one limitation for the takehome because we don't have a backfill is that we cannot see much metric on the successful trades. For example, tracking the pools called by the bot program. since there are so few successful arbitrages, unless the ingestor is run for longer we wouldn't find any data on the pools used. So the "Activity" row in the dashboard will have very little data/no data. 







# dash
