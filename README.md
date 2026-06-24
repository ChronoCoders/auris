# Auris

Autonomous arbitrage engine for Ethereum. Monitors Uniswap V2 liquidity pools in real time, detects mispriced assets, and closes spreads with Aave V3 flash loans — landing transactions privately through Flashbots. Single Rust binary, self-hosted, no external dependencies.

## Architecture

| Crate | Role |
|-------|------|
| `auris-types` | Shared domain types |
| `auris-config` | Config loading and `AURIS_*` env overrides |
| `auris-db` | SQLite migrations and query helpers |
| `auris-core` | WebSocket pool subscriptions, reserve state, AMM math |
| `auris-gas` | Gas oracle — 100-block ring buffer, p90 priority fee |
| `auris-sim` | Profit evaluation and backtesting |
| `auris-exec` | Transaction signing, Flashbots bundle submission |
| `auris-api` | REST + WebSocket API, JWT auth, web UI |
| `auris-bin` | Entry point |

## Quickstart

### 1. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Configure

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
path = "./auris.sqlite"

[auth]
jwt_secret = "CHANGE_ME"
jwt_expiry_seconds = 86400

[keys]
signer_key_path = "./keys/signer.key"
flashbots_key_path = "./keys/flashbots.key"
contract_address = "0x0000000000000000000000000000000000000000"
aave_pool_provider = "0x2f39d218133AFaB8F2B819B1066c7E434Ad94E9e"
```

### 3. Run

```bash
cargo run -p auris-bin -- --config ./config.toml
```

## API

### Auth

```
POST /api/auth/register   — first run only; disabled once a user exists
POST /api/auth/login      — returns JWT
```

All subsequent requests require `Authorization: Bearer <token>`.

### WebSocket

```
GET /ws
```

First message must be `{"type":"auth","token":"<jwt>"}` within 5 seconds.

### Bot Control

```
POST /api/bot/start
POST /api/bot/stop
GET  /api/bot/status
```

### Data

```
GET /api/stats/pnl
GET /api/stats/gas
GET /api/stats/opportunities
GET /api/trades?limit=50&offset=0
GET /api/wallet/balance
```

## Environment Overrides

Any config key can be overridden at runtime:

```
AURIS_NODE_WS_URL
AURIS_SERVER_BIND_ADDR
AURIS_DATABASE_PATH
AURIS_AUTH_JWT_SECRET
AURIS_KEYS_SIGNER_KEY_PATH
AURIS_KEYS_CONTRACT_ADDRESS
```

Pattern: `AURIS_<SECTION>_<KEY>`
