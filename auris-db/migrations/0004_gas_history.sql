CREATE TABLE IF NOT EXISTS gas_history (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  block_number      INTEGER NOT NULL UNIQUE,
  base_fee_gwei     REAL    NOT NULL,
  priority_fee_gwei REAL    NOT NULL,
  timestamp         INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_gas_block     ON gas_history(block_number DESC);
CREATE INDEX IF NOT EXISTS idx_gas_timestamp ON gas_history(timestamp DESC);
