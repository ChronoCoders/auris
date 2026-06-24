CREATE TABLE IF NOT EXISTS bot_state (
  key   TEXT PRIMARY KEY NOT NULL,
  value TEXT NOT NULL
);

INSERT OR IGNORE INTO bot_state(key, value) VALUES
  ('running',          'false'),
  ('last_block',       '0'),
  ('min_profit_usd',   '5.00'),
  ('max_gas_gwei',     '50'),
  ('pools_monitored',  '0');
