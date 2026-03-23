use std::{net::SocketAddr, path::Path};

use alloy::primitives::Address;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

alloy::sol! {
    #[sol(rpc)]
    interface UniswapV2Pair {
        function token0() external view returns (address);
        function token1() external view returns (address);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let config_path = parse_config_path()?;
    let config = zelvex_config::load(&config_path)?;

    let db_url = sqlite_url(&config.database.path);
    let pool = sqlx::SqlitePool::connect(&db_url).await?;
    zelvex_db::run_migrations(&pool).await?;

    let signer = zelvex_exec::load_signer_from_file(&config.keys.signer_key_path)?;
    let wallet_address = signer.address();

    let pools = resolve_pool_addresses(&config)?;
    let pool_metadata = fetch_pool_metadata(&config.node.ws_url, pools.clone()).await?;
    zelvex_db::set_bot_state(&pool, "pools_monitored", &pools.len().to_string()).await?;
    zelvex_db::set_bot_state(
        &pool,
        "min_profit_usd",
        &config.bot.min_profit_usd.to_string(),
    )
    .await?;
    zelvex_db::set_bot_state(&pool, "max_gas_gwei", &config.bot.max_gas_gwei.to_string()).await?;

    let pool_store = std::sync::Arc::new(Mutex::new(zelvex_core::sync::PoolStore::new()));
    {
        let mut store = pool_store.lock().await;
        for p in pool_metadata {
            store.upsert_pool(p);
        }
    }

    let unique_pools = unique_addresses(&pools);
    let mut pool_pairs = Vec::new();
    for i in 0..unique_pools.len() {
        for j in (i + 1)..unique_pools.len() {
            pool_pairs.push((unique_pools[i], unique_pools[j]));
        }
    }

    let (updates_tx, updates_rx) = tokio::sync::mpsc::channel::<()>(1024);
    let gas_oracle = std::sync::Arc::new(Mutex::new(zelvex_gas::GasOracle::new()));

    {
        let ws_url = config.node.ws_url.clone();
        let pool = pool.clone();
        let gas_oracle = gas_oracle.clone();
        tokio::spawn(async move {
            if let Err(e) = zelvex_gas::run_gas_sampler(&ws_url, pool, gas_oracle).await {
                eprintln!("gas sampler fatal: {e}");
            }
        });
    }

    {
        let ws_url = config.node.ws_url.clone();
        let pool_store = pool_store.clone();
        let updates_tx = updates_tx.clone();
        let pools = pools.clone();
        tokio::spawn(async move {
            if let Err(e) = zelvex_core::sync_subscriber::run_sync_subscription(
                &ws_url, pools, pool_store, updates_tx,
            )
            .await
            {
                eprintln!("sync subscription fatal: {e}");
            }
        });
    }

    let executor = zelvex_exec::ArbitrageExecutor::new(
        &config.node.ws_url,
        &config.keys.signer_key_path,
        &config.keys.flashbots_key_path,
        &config.flashbots.relay_url,
        config.keys.contract_address,
        config.bot.paper_trade,
        pool.clone(),
    )
    .await?;
    let executor = std::sync::Arc::new(executor);

    {
        let scanner_cfg = zelvex_core::scanner::ScannerConfig {
            ws_url: config.node.ws_url.clone(),
            pool_pairs,
            store: pool_store.clone(),
            db: pool.clone(),
            gas_oracle: gas_oracle.clone(),
            min_profit_usd: config.bot.min_profit_usd,
            executor: Some(executor.clone()),
        };
        tokio::spawn(async move {
            if let Err(e) = zelvex_core::scanner::run_scanner(
                scanner_cfg,
                updates_rx,
            )
            .await
            {
                eprintln!("scanner fatal: {e}");
            }
        });
    }

    let state = zelvex_api::AppState {
        pool: pool.clone(),
        jwt_secret: config.auth.jwt_secret,
        jwt_expiry_seconds: config.auth.jwt_expiry_seconds,
        web_ui_path: config.server.web_ui_path,
        node_ws_url: config.node.ws_url,
        wallet_address,
        start_time: std::time::Instant::now(),
        login_limiter: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
    };

    let app = zelvex_api::router(state);

    let listener = TcpListener::bind(config.server.bind_addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn parse_config_path() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            let Some(path) = args.next() else {
                return Err("--config requires a value".into());
            };
            return Ok(std::path::PathBuf::from(path));
        }
    }
    Err("missing --config <path>".into())
}

fn sqlite_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    format!("sqlite://{s}")
}

fn resolve_pool_addresses(
    config: &zelvex_config::Config,
) -> Result<Vec<Address>, Box<dyn std::error::Error>> {
    if !config.pools.seed_pairs.is_empty() {
        let mut out = Vec::with_capacity(config.pools.seed_pairs.len());
        for s in &config.pools.seed_pairs {
            out.push(s.parse::<Address>()?);
        }
        return Ok(out);
    }

    Ok(zelvex_core::sync_subscriber::default_test_pools().to_vec())
}

async fn fetch_pool_metadata(
    ws_url: &str,
    pool_addresses: Vec<Address>,
) -> Result<Vec<zelvex_types::Pool>, Box<dyn std::error::Error>> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    let provider = alloy::providers::ProviderBuilder::new().on_ws(ws).await?;

    let mut out = Vec::with_capacity(pool_addresses.len());
    for pool_address in pool_addresses {
        let pair = UniswapV2Pair::new(pool_address, &provider);
        let UniswapV2Pair::token0Return { _0: token0 } = pair.token0().call().await?;
        let UniswapV2Pair::token1Return { _0: token1 } = pair.token1().call().await?;

        out.push(zelvex_types::Pool {
            pool_address,
            token0,
            token1,
            reserve0: alloy::primitives::U256::ZERO,
            reserve1: alloy::primitives::U256::ZERO,
            block_updated: 0,
        });
    }

    Ok(out)
}

fn unique_addresses(addresses: &[Address]) -> Vec<Address> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &a in addresses {
        if seen.insert(a) {
            out.push(a);
        }
    }
    out
}
