use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use argon2::{password_hash::PasswordHash, Argon2, PasswordHasher, PasswordVerifier};
use axum::{
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::Mutex;

alloy::sol! {
    #[sol(rpc)]
    interface AggregatorV3Interface {
        function decimals() external view returns (uint8);
        function latestRoundData()
            external
            view
            returns (
                uint80 roundId,
                int256 answer,
                uint256 startedAt,
                uint256 updatedAt,
                uint80 answeredInRound
            );
    }
}

const CHAINLINK_ETH_USD_FEED: Address =
    alloy::primitives::address!("0x5f4ec3df9cbd43714fe2740f5e3616155c5b8419");

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub jwt_secret: String,
    pub jwt_expiry_seconds: u64,
    pub web_ui_path: PathBuf,
    pub node_ws_url: String,
    pub wallet_address: Address,
    pub start_time: Instant,
    pub login_limiter: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/bot/start", post(bot_start))
        .route("/api/bot/stop", post(bot_stop))
        .route("/api/bot/status", get(bot_status))
        .route("/api/stats/pnl", get(stats_pnl))
        .route("/api/stats/gas", get(stats_gas))
        .route("/api/stats/opportunities", get(stats_opportunities))
        .route("/api/wallet/balance", get(wallet_balance))
        .route("/api/trades", get(trades_list))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_jwt,
        ));

    Router::new()
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/register", post(auth_register))
        .route("/ws", get(ws_handler))
        .route("/", get(serve_index))
        .route("/style.css", get(serve_style))
        .route("/app.js", get(serve_app_js))
        .fallback(get(serve_index))
        .merge(protected)
        .with_state(state)
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    RateLimited(String),
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            ApiError::BadRequest(_) => "BAD_REQUEST",
            ApiError::Unauthorized(_) => "UNAUTHORIZED",
            ApiError::Forbidden(_) => "FORBIDDEN",
            ApiError::Conflict(_) => "CONFLICT",
            ApiError::RateLimited(_) => "RATE_LIMITED",
            ApiError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(ErrorResponse {
            error: self.to_string(),
            code: self.code().to_string(),
        });
        (status, body).into_response()
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub uid: i64,
    pub exp: usize,
}

async fn auth_login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    if req.username.is_empty() || req.password.is_empty() {
        return Err(ApiError::BadRequest("missing fields".to_string()));
    }

    enforce_login_rate_limit(&state, addr.ip()).await?;

    let user = zelvex_db::get_user_by_username(&state.pool, &req.username)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("invalid credentials".to_string()))?;

    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|_| ApiError::Unauthorized("invalid credentials".to_string()))?;
    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| ApiError::Unauthorized("invalid credentials".to_string()))?;

    let (token, expires_at) = issue_jwt(&state, user.id, &user.username)?;
    Ok(Json(AuthResponse { token, expires_at }))
}

async fn auth_register(
    State(state): State<AppState>,
    Json(req): Json<AuthRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    if req.username.is_empty() || req.password.is_empty() {
        return Err(ApiError::BadRequest("missing fields".to_string()));
    }
    if req.password.len() < 12 {
        return Err(ApiError::BadRequest("password too short".to_string()));
    }

    let user_count = zelvex_db::get_user_count(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if user_count > 0 {
        return Err(ApiError::Forbidden("registration disabled".to_string()));
    }

    let salt = argon2::password_hash::SaltString::generate(&mut rand::thread_rng());
    let hash = Argon2::default()
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|_| ApiError::Internal("password hash failure".to_string()))?
        .to_string();

    let user_id = zelvex_db::insert_user(&state.pool, &req.username, &hash)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                ApiError::Conflict("username already exists".to_string())
            } else {
                ApiError::Internal(e.to_string())
            }
        })?;

    let (token, expires_at) = issue_jwt(&state, user_id, &req.username)?;
    Ok((
        StatusCode::CREATED,
        Json(AuthResponse { token, expires_at }),
    ))
}

async fn require_jwt(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let token = extract_bearer(&headers)
        .ok_or_else(|| ApiError::Unauthorized("missing jwt".to_string()))?;
    decode_jwt(&state, token)?;
    Ok(next.run(request).await)
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let prefix = "Bearer ";
    value.strip_prefix(prefix).map(|v| v.trim())
}

fn decode_jwt(state: &AppState, token: &str) -> Result<Claims, ApiError> {
    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .map_err(|_| ApiError::Unauthorized("invalid jwt".to_string()))
}

fn issue_jwt(state: &AppState, user_id: i64, username: &str) -> Result<(String, i64), ApiError> {
    let now = chrono::Utc::now().timestamp();
    let exp = now + state.jwt_expiry_seconds as i64;

    let claims = Claims {
        sub: username.to_string(),
        uid: user_id,
        exp: exp as usize,
    };
    let token = jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|_| ApiError::Internal("jwt encode failed".to_string()))?;

    Ok((token, exp))
}

async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(state, socket))
}

async fn handle_ws(state: AppState, mut socket: WebSocket) {
    let auth = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = auth else {
        let _ = socket.close().await;
        return;
    };

    let Ok(msg) = serde_json::from_str::<WsAuthMessage>(&text) else {
        let _ = socket
            .send(Message::Text(
                r#"{"type":"auth_fail","reason":"invalid"}"#.to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return;
    };

    if msg.r#type != "auth" {
        let _ = socket
            .send(Message::Text(
                r#"{"type":"auth_fail","reason":"invalid"}"#.to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return;
    }

    if decode_jwt(&state, &msg.token).is_err() {
        let _ = socket
            .send(Message::Text(
                r#"{"type":"auth_fail","reason":"invalid"}"#.to_string(),
            ))
            .await;
        let _ = socket.close().await;
        return;
    }

    let _ = socket
        .send(Message::Text(r#"{"type":"auth_ok"}"#.to_string()))
        .await;

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let mut last_trade_id = sqlx::query_as::<_, (Option<i64>,)>("SELECT MAX(id) FROM trades")
        .fetch_one(&state.pool)
        .await
        .ok()
        .and_then(|(id,)| id)
        .unwrap_or(0);

    loop {
        tokio::select! {
            Some(Ok(msg)) = socket.recv() => {
                match msg {
                    Message::Text(t) => {
                        let Ok(cmd) = serde_json::from_str::<WsClientCommand>(&t) else {
                            continue;
                        };
                        match cmd.r#type.as_str() {
                            "start_bot" => {
                                let _ = zelvex_db::set_bot_state(&state.pool, "running", "true").await;
                            }
                            "stop_bot" => {
                                let _ = zelvex_db::set_bot_state(&state.pool, "running", "false").await;
                            }
                            _ => {}
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = ticker.tick() => {
                if push_ws_updates(&state, &mut socket, &mut last_trade_id).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WsAuthMessage {
    pub r#type: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct WsClientCommand {
    pub r#type: String,
}

async fn bot_start(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    zelvex_db::set_bot_state(&state.pool, "running", "true")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let block = zelvex_db::get_bot_state(&state.pool, "last_block")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(Json(
        serde_json::json!({ "status": "running", "block": block }),
    ))
}

async fn bot_stop(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    zelvex_db::set_bot_state(&state.pool, "running", "false")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "status": "stopped" })))
}

async fn bot_status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let running = zelvex_db::get_bot_state(&state.pool, "running")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| "false".to_string());
    let status = if running == "true" {
        "running"
    } else {
        "stopped"
    };

    let current_block = zelvex_db::get_bot_state(&state.pool, "last_block")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let pools_monitored = zelvex_db::get_bot_state(&state.pool, "pools_monitored")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let uptime_seconds = state.start_time.elapsed().as_secs();

    Ok(Json(serde_json::json!({
        "status": status,
        "error_message": serde_json::Value::Null,
        "current_block": current_block,
        "uptime_seconds": uptime_seconds,
        "pools_monitored": pools_monitored
    })))
}

async fn stats_pnl(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let pnl = zelvex_db::get_pnl_summary(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "today_usd": pnl.today_usd,
        "week_usd": pnl.week_usd,
        "alltime_usd": pnl.alltime_usd,
        "today_trades": pnl.today_trades,
        "week_trades": pnl.week_trades
    })))
}

async fn stats_gas(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let (total_spent,): (Option<f64>,) = sqlx::query_as("SELECT SUM(gas_cost_usd) FROM trades")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (failed_spent,): (Option<f64>,) =
        sqlx::query_as("SELECT SUM(gas_cost_usd) FROM trades WHERE status != 'success'")
            .fetch_one(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    let (base_fee_gwei, priority_fee_gwei): (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT base_fee_gwei, priority_fee_gwei FROM gas_history ORDER BY block_number DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .unwrap_or((None, None));

    let min_profit_gate_usd = zelvex_db::get_bot_state(&state.pool, "min_profit_usd")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    Ok(Json(serde_json::json!({
        "total_spent_usd": total_spent.unwrap_or(0.0),
        "failed_tx_spent_usd": failed_spent.unwrap_or(0.0),
        "current_base_fee_gwei": base_fee_gwei.unwrap_or(0.0),
        "current_priority_fee_gwei": priority_fee_gwei.unwrap_or(0.0),
        "min_profit_gate_usd": min_profit_gate_usd
    })))
}

async fn stats_opportunities(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (scanned_total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM opportunities")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (executed_total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (profitable_total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM trades WHERE status = 'success' AND net_profit_usd > 0",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (success_total,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM trades WHERE status = 'success'")
            .fetch_one(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    let success_rate_pct = if executed_total == 0 {
        0.0
    } else {
        (success_total as f64 / executed_total as f64) * 100.0
    };

    Ok(Json(serde_json::json!({
        "scanned_total": scanned_total,
        "executed_total": executed_total,
        "profitable_total": profitable_total,
        "frontrun_estimated": 0,
        "success_rate_pct": success_rate_pct
    })))
}

async fn wallet_balance(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ws = alloy::transports::ws::WsConnect::new(&state.node_ws_url);
    let provider = alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let balance_wei = provider
        .get_balance(state.wallet_address)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let eth_balance = format_wei_as_eth(balance_wei);

    let feed = AggregatorV3Interface::new(CHAINLINK_ETH_USD_FEED, &provider);
    let AggregatorV3Interface::decimalsReturn { _0: decimals } = feed
        .decimals()
        .call()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let AggregatorV3Interface::latestRoundDataReturn { answer, .. } = feed
        .latestRoundData()
        .call()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let price_raw: f64 = answer
        .to_string()
        .parse()
        .map_err(|_| ApiError::Internal("invalid price".to_string()))?;
    let eth_usd_price = price_raw / 10f64.powi(decimals as i32);

    let eth_balance_f64 = eth_balance.parse::<f64>().unwrap_or(0.0);
    let usd_value = eth_balance_f64 * eth_usd_price;

    Ok(Json(serde_json::json!({
        "eth_balance": eth_balance,
        "eth_usd_price": eth_usd_price,
        "usd_value": usd_value
    })))
}

#[derive(Debug, Deserialize)]
pub struct TradesQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

async fn trades_list(
    State(state): State<AppState>,
    Query(query): Query<TradesQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);

    let rows = sqlx::query_as::<_, (i64, String, String, String, String, f64, Option<f64>, String, i64, i64)>(
        "SELECT id, tx_hash, route, pool_a, pool_b, gas_cost_usd, net_profit_usd, status, block_number, timestamp
         FROM trades ORDER BY timestamp DESC LIMIT ? OFFSET ?",
    )
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut trades = Vec::with_capacity(rows.len());
    for (
        id,
        tx_hash,
        route,
        pool_a,
        pool_b,
        gas_cost_usd,
        net_profit_usd,
        status,
        block_number,
        timestamp,
    ) in rows
    {
        trades.push(serde_json::json!({
            "id": id,
            "tx_hash": tx_hash,
            "route": route,
            "pool_a": pool_a,
            "pool_b": pool_b,
            "net_profit_usd": net_profit_usd.unwrap_or(0.0),
            "gas_cost_usd": gas_cost_usd,
            "status": status,
            "block_number": block_number,
            "timestamp": timestamp
        }));
    }

    let total = zelvex_db::get_trade_count(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(
        serde_json::json!({ "trades": trades, "total": total }),
    ))
}

async fn serve_index(State(state): State<AppState>) -> Result<Response, ApiError> {
    serve_file(
        &state.web_ui_path.join("index.html"),
        "text/html; charset=utf-8",
    )
    .await
}

async fn serve_style(State(state): State<AppState>) -> Result<Response, ApiError> {
    serve_file(
        &state.web_ui_path.join("style.css"),
        "text/css; charset=utf-8",
    )
    .await
}

async fn serve_app_js(State(state): State<AppState>) -> Result<Response, ApiError> {
    serve_file(
        &state.web_ui_path.join("app.js"),
        "application/javascript; charset=utf-8",
    )
    .await
}

async fn serve_file(path: &Path, content_type: &'static str) -> Result<Response, ApiError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| ApiError::Internal("static file not found".to_string()))?;
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

async fn enforce_login_rate_limit(state: &AppState, ip: IpAddr) -> Result<(), ApiError> {
    let mut limiter = state.login_limiter.lock().await;
    let now = Instant::now();
    let window = Duration::from_secs(60);

    let entry = limiter.entry(ip).or_default();
    entry.retain(|t| now.duration_since(*t) < window);
    if entry.len() >= 5 {
        return Err(ApiError::RateLimited("too many attempts".to_string()));
    }
    entry.push(now);

    Ok(())
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => db.message().to_lowercase().contains("unique"),
        _ => false,
    }
}

async fn ws_send(socket: &mut WebSocket, value: serde_json::Value) -> Result<(), ApiError> {
    socket
        .send(Message::Text(value.to_string()))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn push_ws_updates(
    state: &AppState,
    socket: &mut WebSocket,
    last_trade_id: &mut i64,
) -> Result<(), ApiError> {
    let running = zelvex_db::get_bot_state(&state.pool, "running")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| "false".to_string());
    let bot_status = if running == "true" {
        "running"
    } else {
        "stopped"
    };

    let current_block = zelvex_db::get_bot_state(&state.pool, "last_block")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    ws_send(
        socket,
        serde_json::json!({
            "type": "block_update",
            "block": current_block,
            "timestamp": chrono::Utc::now().timestamp()
        }),
    )
    .await?;

    ws_send(
        socket,
        serde_json::json!({
            "type": "bot_status",
            "status": bot_status,
            "error": serde_json::Value::Null
        }),
    )
    .await?;

    let (base_fee_gwei, priority_fee_gwei): (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT base_fee_gwei, priority_fee_gwei FROM gas_history ORDER BY block_number DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .unwrap_or((None, None));

    let gate_usd = zelvex_db::get_bot_state(&state.pool, "min_profit_usd")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    ws_send(
        socket,
        serde_json::json!({
            "type": "gas_update",
            "base_fee_gwei": base_fee_gwei.unwrap_or(0.0),
            "priority_fee_gwei": priority_fee_gwei.unwrap_or(0.0),
            "gate_usd": gate_usd
        }),
    )
    .await?;

    let (scanned,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM opportunities")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let executed = zelvex_db::get_trade_count(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (profitable,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM trades WHERE status = 'success' AND net_profit_usd > 0",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    ws_send(
        socket,
        serde_json::json!({
            "type": "opportunity_scan",
            "scanned": scanned,
            "executed": executed,
            "profitable": profitable
        }),
    )
    .await?;

    let rows = sqlx::query_as::<_, (i64, String, String, String, String, f64, Option<f64>, String, i64, i64)>(
        "SELECT id, tx_hash, route, pool_a, pool_b, gas_cost_usd, net_profit_usd, status, block_number, timestamp
         FROM trades WHERE id > ? ORDER BY id ASC LIMIT 50",
    )
    .bind(*last_trade_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    for (
        id,
        tx_hash,
        route,
        pool_a,
        pool_b,
        gas_cost_usd,
        net_profit_usd,
        status,
        block_number,
        timestamp,
    ) in rows
    {
        *last_trade_id = id;
        ws_send(
            socket,
            serde_json::json!({
                "type": "new_trade",
                "trade": {
                    "id": id,
                    "tx_hash": tx_hash,
                    "route": route,
                    "pool_a": pool_a,
                    "pool_b": pool_b,
                    "net_profit_usd": net_profit_usd.unwrap_or(0.0),
                    "gas_cost_usd": gas_cost_usd,
                    "status": status,
                    "block_number": block_number,
                    "timestamp": timestamp
                }
            }),
        )
        .await?;
    }

    Ok(())
}

fn format_wei_as_eth(wei: U256) -> String {
    let s = wei.to_string();
    if s == "0" {
        return "0".to_string();
    }
    if s.len() <= 18 {
        let mut frac = "0".repeat(18 - s.len());
        frac.push_str(&s);
        let frac = frac.trim_end_matches('0');
        if frac.is_empty() {
            "0".to_string()
        } else {
            format!("0.{frac}")
        }
    } else {
        let (int_part, frac_part) = s.split_at(s.len() - 18);
        let frac = frac_part.trim_end_matches('0');
        if frac.is_empty() {
            int_part.to_string()
        } else {
            format!("{int_part}.{frac}")
        }
    }
}
