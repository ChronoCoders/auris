use std::{net::SocketAddr, path::Path};

use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = parse_config_path()?;
    let config = zelvex_config::load(&config_path)?;

    let db_url = sqlite_url(&config.database.path);
    let pool = sqlx::SqlitePool::connect(&db_url).await?;
    zelvex_db::run_migrations(&pool).await?;

    let signer = zelvex_exec::load_signer_from_file(&config.keys.signer_key_path)?;
    let wallet_address = signer.address();

    let state = zelvex_api::AppState {
        pool,
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
