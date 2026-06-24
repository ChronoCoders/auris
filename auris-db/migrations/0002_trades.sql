CREATE TABLE IF NOT EXISTS trades (
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
                         CHECK(status IN ('success','failed','reverted')),
  block_number   INTEGER NOT NULL,
  timestamp      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_trades_timestamp    ON trades(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_trades_status       ON trades(status);
CREATE INDEX IF NOT EXISTS idx_trades_block        ON trades(block_number DESC);
