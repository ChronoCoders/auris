-- Migration: add 'simulated' to trades.status for paper trading support
PRAGMA foreign_keys=off;

CREATE TABLE IF NOT EXISTS trades_new (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  tx_hash        TEXT    NOT NULL UNIQUE,
  route          TEXT    NOT NULL,
  pool_a         TEXT    NOT NULL,
  pool_b         TEXT    NOT NULL,
  input_amount   TEXT    NOT NULL,
  output_amount  TEXT,
  gross_profit   TEXT,
  gas_cost_usd   REAL    NOT NULL,
  net_profit_usd REAL,
  gas_used       INTEGER,
  status         TEXT    NOT NULL
                         CHECK(status IN ('success','failed','reverted','simulated')),
  block_number   INTEGER NOT NULL,
  timestamp      INTEGER NOT NULL DEFAULT (unixepoch())
);

INSERT INTO trades_new(
  id, tx_hash, route, pool_a, pool_b,
  input_amount, output_amount, gross_profit,
  gas_cost_usd, net_profit_usd, gas_used,
  status, block_number, timestamp
)
SELECT
  id, tx_hash, route, pool_a, pool_b,
  input_amount, output_amount, gross_profit,
  gas_cost_usd, net_profit_usd, gas_used,
  status, block_number, timestamp
FROM trades;

DROP TABLE trades;
ALTER TABLE trades_new RENAME TO trades;

CREATE INDEX IF NOT EXISTS idx_trades_timestamp ON trades(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_trades_status    ON trades(status);
CREATE INDEX IF NOT EXISTS idx_trades_block     ON trades(block_number DESC);

PRAGMA foreign_keys=on;
