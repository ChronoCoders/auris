use std::{net::SocketAddr, path::Path};

use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = parse_config_path()?;
    let config = zelvex_config::load(&config_path)?;

    let db_url = sqlite_url(&config.database.path);
    let pool = sqlx::SqlitePool::connect(&db_url).await?;
    zelvex_db::run_migrations(&pool).await?;

    let signer = zelvex_exec::load_signer_from_file(&config.keys.signer_key_path)?;
    let wallet_address = signer.address();

    let pool_metadata = zelvex_core::sync_subscriber::default_test_pool_metadata();
    let pools: Vec<_> = pool_metadata.iter().map(|p| p.pool_address).collect();
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

    let mut unique_pools = Vec::new();
    for p in &pools {
        if !unique_pools.contains(p) {
            unique_pools.push(*p);
        }
    }
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

    {
        let ws_url = config.node.ws_url.clone();
        let pool_store = pool_store.clone();
        let db = pool.clone();
        let gas_oracle = gas_oracle.clone();
        let min_profit_usd = config.bot.min_profit_usd;
        tokio::spawn(async move {
            if let Err(e) = zelvex_core::scanner::run_scanner(
                ws_url,
                updates_rx,
                pool_pairs,
                pool_store,
                db,
                gas_oracle,
                min_profit_usd,
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
