# ZELVEX

ZELVEX is a Rust workspace that implements the initial scaffolding for a DeFi arbitrage bot stack: config loading, SQLite persistence/migrations, an API server (REST + WebSocket), and core math and subscription primitives.

This repository enforces a strict quality gate: no warnings, no unused code, formatting checked, tests required.

## Workspace Layout

- `zelvex-types`: shared domain types
- `zelvex-config`: `config.toml` loader + `ZELVEX_*` env overrides
- `zelvex-db`: SQLite migrations + query helpers
- `zelvex-core`: AMM math + websocket subscription scaffolding (new heads, Uniswap V2 Sync logs)
- `zelvex-gas`: gas oracle (ring buffer + p90 priority fee)
- `zelvex-sim`: profit decision engine (`evaluate`)
- `zelvex-exec`: execution helpers (key loading)
- `zelvex-api`: REST + WebSocket API server, JWT auth, static web UI serving
- `zelvex-bin`: binary wiring everything together
- `web-ui`: minimal UI served by the API

## Quickstart

### 1) Install Rust

Install the latest stable Rust toolchain (includes `cargo` and `rustfmt`).

### 2) Create a config file

Create a `config.toml` (example below). Do not commit secrets.

```toml
[node]
ws_url = "ws://127.0.0.1:8546"

[flashbots]
relay_url = "https://relay.flashbots.net"

[bot]
min_profit_usd = 5.0
max_gas_gwei = 50
paper_trade = true

[server]
bind_addr = "127.0.0.1:8080"
web_ui_path = "./web-ui"

[database]
path = "./zelvex.sqlite"

[auth]
jwt_secret = "CHANGE_ME"
jwt_expiry_seconds = 86400

[keys]
signer_key_path = "./keys/signer.key"
flashbots_key_path = "./keys/flashbots.key"
contract_address = "0x0000000000000000000000000000000000000000"
aave_pool_provider = "0x2f39d218133AFaB8F2B819B1066c7E434Ad94E9e"
```

### 3) Run

```bash
cargo run -p zelvex-bin -- --config ./config.toml
```

This will:
- open/create the SQLite database
- run migrations
- start the HTTP server on `server.bind_addr`
- serve the UI from `server.web_ui_path`

## API Overview

### Auth

- `POST /api/auth/register`
  - one-time bootstrap: once a user exists, registration is disabled
- `POST /api/auth/login`
  - returns JWT

Send JWT in:

`Authorization: Bearer <token>`

### WebSocket

- `GET /ws`
  - first message must be `{"type":"auth","token":"<jwt>"}` within 5 seconds
  - server replies with `{"type":"auth_ok"}` or `{"type":"auth_fail","reason":"invalid"}`

### Bot Control

- `POST /api/bot/start`
- `POST /api/bot/stop`
- `GET /api/bot/status`

### Stats / Trades / Wallet

- `GET /api/stats/pnl`
- `GET /api/stats/gas`
- `GET /api/stats/opportunities`
- `GET /api/trades?limit=50&offset=0`
- `GET /api/wallet/balance`
  - uses `node.ws_url` for `eth_getBalance`
  - uses Chainlink ETH/USD feed on mainnet to return USD estimates

## Environment Overrides

Any config key can be overridden via environment variables:

- `ZELVEX_NODE_WS_URL`
- `ZELVEX_SERVER_BIND_ADDR`
- `ZELVEX_DATABASE_PATH`
- `ZELVEX_AUTH_JWT_SECRET`
- `ZELVEX_KEYS_SIGNER_KEY_PATH`
- `ZELVEX_KEYS_CONTRACT_ADDRESS`

Pattern:

`ZELVEX_<SECTION>_<KEY>` (uppercased).

## Quality Gate

These commands must pass cleanly:

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo test --workspace
```
