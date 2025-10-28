## dash-indexer (Part 1 - Ingest)

Ingest Solana Yellowstone gRPC transaction updates for the competitor bot program and store burn (fee) data in Postgres.

### Run locally

1. Setup the .env, see src/config.rs for required fields.
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
- derive an `arbitrage_success` label from logs and swap activity
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


```

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

### Part 3 - Expand (Additions and rationale)

- **Compute units (CU)**: Captured per transaction to normalize burn across changing fee markets. Enables derived metrics like lamports-per-CU for cost efficiency.
- **Arbitrage success label**: Heuristic boolean inferred from program logs and presence of swap-related instructions. Separates "scans" from executed arbitrage, powering opportunity rate and venue analysis. Caveat: heuristic, not ground truth.
- **Failure taxonomy** (`tx_failures`): Stores first-order error type for each failed tx to track top regressions and reliability drift.
- **Program hotspots** (`tx_instructions`): Aggregates instruction counts by external program per transaction (filters common noise like System/ComputeBudget/Token/Memo/bot itself). Surfaces venue mix and routing patterns.
- **Block time**: Stored to support time-series queries without costly joins.

Notes on design:
- We intentionally keep the schema lean and compute ratios (e.g., lamports-per-CU) in queries instead of storing them as columns.
- Noise program filtering focuses analytics on actionable venues without drowning in boilerplate instructions.

### Notes
- Backfilling was a big bottleneck and a shortcut taken. There is currently no backfilling logic, I attempted to use https://docs.triton.one/project-yellowstone/fumarole and just plain rpc requests per block and then filtering for the program and some other ways. None were reliable enough and could go far back. In production i would push for access to https://docs.triton.one/project-yellowstone/fumarole, didn't know if this was enabled in the current endpoint. Would have done this if i had more time. 
- I found there to be an infinite amounts of metrics to be tracked. Example things like tracking validator-level latency, priority-fee density per block, pool-specific arbitrage profitability, or even live mempool competition stats felt like overkill for this scope. They also required a richer access to the internal data. For example, tracking the error codes, it'd have been great to have access to what these internal errors map to eg: custom program error: 0x1. Given the scope i decided to keep some essential metrics that i deep useful for a competitor to track. 
- one limitation for the takehome because we don't have a backfill is that we cannot see much metric on the successful arbitrages. For example, tracking the pools called by the bot program if a successful arbitrage was found. since there are so few successful arbitrages, unless the ingestor is run for longer we wouldn't find any data on the pools used. So the "Activity" row in the dashboard will have very little data/no data. 


### Dashboard Architecture

- 1. Data Source: NeonDB connected directly in Grafana Cloud.

- 2.  Panels:
  - Derived values like Lamports/CU are computed in queries as `fee_lamports / NULLIF(compute_units,0)`; they are not stored as columns.
  - Cost Efficiency:
    - Lifetime Burn (SOL): total SOL spent by the bot
    - Lamports per CU (p50 / p90)
    - Average Burn (Hourly): fee paid vs fee lost on failed transactions
    - Avg Lamports/CU (Lifetime)
    - Top Burners: top fee payers by total SOL burnt
  - Reliability:
    - Top Failure Types (Pie / Bar)
    - Transactions per Hour
    - Failure Rate (Hourly)
  - Activity & Opportunities:
    - Arbitrage Success (Hourly)
    - Program Hotspots: external programs most used in successful arbitrages
