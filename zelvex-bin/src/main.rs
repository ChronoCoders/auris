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

    let pools = zelvex_core::sync_subscriber::default_test_pools().to_vec();
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
        let ws_url = config.node.ws_url.clone();
        let pool = pool.clone();
        tokio::spawn(async move {
            let mut oracle = zelvex_gas::GasOracle::new();
            if let Err(e) = zelvex_gas::run_gas_sampler(&ws_url, pool, &mut oracle).await {
                eprintln!("gas sampler fatal: {e}");
            }
        });
    }

    {
        let ws_url = config.node.ws_url.clone();
        let pool_store = pool_store.clone();
        tokio::spawn(async move {
            if let Err(e) =
                zelvex_core::sync_subscriber::run_sync_subscription(&ws_url, pools, pool_store)
                    .await
            {
                eprintln!("sync subscription fatal: {e}");
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
