CREATE TABLE IF NOT EXISTS opportunities (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  pool_a            TEXT    NOT NULL,
  pool_b            TEXT    NOT NULL,
  token_in          TEXT    NOT NULL,
  token_out         TEXT    NOT NULL,
  spread_bps        INTEGER NOT NULL,
  input_amount      TEXT    NOT NULL,
  estimated_profit  REAL    NOT NULL,
  gas_estimate_usd  REAL    NOT NULL,
  decision          TEXT    NOT NULL
                            CHECK(decision IN ('go','no-go')),
  no_go_reason      TEXT,
  trade_id          INTEGER REFERENCES trades(id),
  timestamp         INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_opp_timestamp ON opportunities(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_opp_decision  ON opportunities(decision);
