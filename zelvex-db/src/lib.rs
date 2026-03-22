use alloy::primitives::{Address, TxHash, U256};
use sqlx::SqlitePool;
use thiserror::Error;
use zelvex_types::{ArbitrageOpportunity, TradeResult, TradeStatus};

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid u256 string: {0}")]
    InvalidU256(String),
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("invalid tx hash: {0}")]
    InvalidTxHash(String),
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct PnlSummary {
    pub today_usd: f64,
    pub week_usd: f64,
    pub alltime_usd: f64,
    pub today_trades: i64,
    pub week_trades: i64,
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))
}

pub async fn insert_user(
    pool: &SqlitePool,
    username: &str,
    hash: &str,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query("INSERT INTO users(username, password_hash) VALUES (?, ?)")
        .bind(username)
        .bind(hash)
        .execute(pool)
        .await?;
    Ok(result.last_insert_rowid())
}

pub async fn get_user_by_username(
    pool: &SqlitePool,
    username: &str,
) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, String, String, i64)>(
        "SELECT id, username, password_hash, created_at FROM users WHERE username = ? LIMIT 1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(id, username, password_hash, created_at)| User {
        id,
        username,
        password_hash,
        created_at,
    }))
}

pub async fn get_user_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn insert_trade(pool: &SqlitePool, trade: &TradeResult) -> Result<i64, sqlx::Error> {
    let input_amount = u256_to_dec(trade.input_amount);
    let output_amount = trade.output_amount.map(u256_to_dec);
    let gross_profit = trade.gross_profit.map(u256_to_dec);

    let status = trade_status_to_db(&trade.status);

    let result = sqlx::query(
        "INSERT INTO trades(tx_hash, route, pool_a, pool_b, input_amount, output_amount, gross_profit, gas_cost_usd, net_profit_usd, gas_used, status, block_number, timestamp)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(format!("{:#x}", trade.tx_hash))
    .bind(&trade.route)
    .bind(format!("{:#x}", trade.pool_a))
    .bind(format!("{:#x}", trade.pool_b))
    .bind(input_amount)
    .bind(output_amount)
    .bind(gross_profit)
    .bind(trade.gas_cost_usd)
    .bind(trade.net_profit_usd)
    .bind(trade.gas_used.map(|v| v as i64))
    .bind(status)
    .bind(trade.block_number as i64)
    .bind(trade.timestamp)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn insert_opportunity(
    pool: &SqlitePool,
    opp: &ArbitrageOpportunity,
    decision: &str,
    no_go_reason: Option<&str>,
    trade_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    let input_amount = u256_to_dec(opp.input_amount);
    let result = sqlx::query(
        "INSERT INTO opportunities(pool_a, pool_b, token_in, token_out, spread_bps, input_amount, estimated_profit, gas_estimate_usd, decision, no_go_reason, trade_id, timestamp)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, unixepoch())",
    )
    .bind(format!("{:#x}", opp.pool_a))
    .bind(format!("{:#x}", opp.pool_b))
    .bind(format!("{:#x}", opp.token_in))
    .bind(format!("{:#x}", opp.token_out))
    .bind(opp.spread_bps)
    .bind(input_amount)
    .bind(opp.estimated_profit_usd)
    .bind(opp.gas_estimate_usd)
    .bind(decision)
    .bind(no_go_reason)
    .bind(trade_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get_recent_trades(pool: &SqlitePool, limit: u32) -> Result<Vec<TradeResult>, DbError> {
    get_trades_page(pool, limit, 0).await
}

pub async fn get_trades_page(
    pool: &SqlitePool,
    limit: u32,
    offset: u32,
) -> Result<Vec<TradeResult>, DbError> {
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            f64,
            Option<f64>,
            Option<i64>,
            String,
            i64,
            i64,
        ),
    >(
        "SELECT id, tx_hash, route, pool_a, pool_b, input_amount, output_amount, gross_profit, gas_cost_usd, net_profit_usd, gas_used, status, block_number, timestamp
         FROM trades ORDER BY timestamp DESC LIMIT ? OFFSET ?",
    )
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await?;

    let mut trades = Vec::with_capacity(rows.len());
    for (
        id,
        tx_hash,
        route,
        pool_a,
        pool_b,
        input_amount,
        output_amount,
        gross_profit,
        gas_cost_usd,
        net_profit_usd,
        gas_used,
        status,
        block_number,
        timestamp,
    ) in rows
    {
        let trade = TradeResult {
            tx_hash: parse_tx_hash(&tx_hash)?,
            route,
            pool_a: parse_address(&pool_a)?,
            pool_b: parse_address(&pool_b)?,
            input_amount: parse_u256_required(&input_amount)?,
            output_amount: parse_u256_opt(output_amount)?,
            gross_profit: parse_u256_opt(gross_profit)?,
            gas_cost_usd,
            net_profit_usd,
            gas_used: gas_used.map(|v| v as u64),
            status: trade_status_from_db(&status),
            block_number: block_number as u64,
            timestamp,
        };

        let _ = id;
        trades.push(trade);
    }

    Ok(trades)
}

pub async fn get_trade_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn get_pnl_summary(pool: &SqlitePool) -> Result<PnlSummary, sqlx::Error> {
    let (alltime_usd,): (Option<f64>,) =
        sqlx::query_as("SELECT SUM(net_profit_usd) FROM trades WHERE status = 'success'")
            .fetch_one(pool)
            .await?;

    let (today_usd, today_trades): (Option<f64>, i64) = sqlx::query_as(
        "SELECT SUM(net_profit_usd), COUNT(*) FROM trades
         WHERE status = 'success' AND date(timestamp,'unixepoch') = date('now')",
    )
    .fetch_one(pool)
    .await?;

    let (week_usd, week_trades): (Option<f64>, i64) = sqlx::query_as(
        "SELECT SUM(net_profit_usd), COUNT(*) FROM trades
         WHERE status = 'success' AND timestamp >= unixepoch('now','-7 day')",
    )
    .fetch_one(pool)
    .await?;

    Ok(PnlSummary {
        today_usd: today_usd.unwrap_or(0.0),
        week_usd: week_usd.unwrap_or(0.0),
        alltime_usd: alltime_usd.unwrap_or(0.0),
        today_trades,
        week_trades,
    })
}

pub async fn insert_gas_sample(
    pool: &SqlitePool,
    block: u64,
    base: f64,
    priority: f64,
) -> Result<(), sqlx::Error> {
    let _ = sqlx::query(
        "INSERT OR REPLACE INTO gas_history(block_number, base_fee_gwei, priority_fee_gwei) VALUES (?, ?, ?)",
    )
    .bind(block as i64)
    .bind(base)
    .bind(priority)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_bot_state(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String,)>("SELECT value FROM bot_state WHERE key = ? LIMIT 1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(value,)| value))
}

pub async fn set_bot_state(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    let _ = sqlx::query("INSERT OR REPLACE INTO bot_state(key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

fn trade_status_to_db(status: &TradeStatus) -> &'static str {
    match status {
        TradeStatus::Success => "success",
        TradeStatus::Failed => "failed",
        TradeStatus::Reverted => "reverted",
    }
}

fn trade_status_from_db(status: &str) -> TradeStatus {
    match status {
        "success" => TradeStatus::Success,
        "failed" => TradeStatus::Failed,
        "reverted" => TradeStatus::Reverted,
        _ => TradeStatus::Failed,
    }
}

fn u256_to_dec(value: U256) -> String {
    value.to_string()
}

fn parse_u256_opt(value: Option<String>) -> Result<Option<U256>, DbError> {
    let Some(v) = value else {
        return Ok(None);
    };
    Ok(Some(parse_u256(&v)?))
}

fn parse_u256_required(value: &str) -> Result<U256, DbError> {
    parse_u256(value)
}

fn parse_u256(value: &str) -> Result<U256, DbError> {
    U256::from_str_radix(value, 10).map_err(|_| DbError::InvalidU256(value.to_string()))
}

fn parse_address(value: &str) -> Result<Address, DbError> {
    value
        .parse::<Address>()
        .map_err(|_| DbError::InvalidAddress(value.to_string()))
}

fn parse_tx_hash(value: &str) -> Result<TxHash, DbError> {
    value
        .parse::<TxHash>()
        .map_err(|_| DbError::InvalidTxHash(value.to_string()))
}
