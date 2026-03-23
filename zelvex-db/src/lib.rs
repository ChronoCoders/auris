use alloy::primitives::{Address, TxHash, U256};
use sqlx::SqlitePool;
use thiserror::Error;
use zelvex_types::{ArbitrageOpportunity, OpportunityRecord, TradeResult, TradeStatus};

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

pub async fn get_opportunities_for_backtest(
    pool: &SqlitePool,
    limit: u32,
    offset: u32,
) -> Result<Vec<OpportunityRecord>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, f64, f64, i64, String, Option<String>, i64)>(
        "SELECT id, estimated_profit, gas_estimate_usd, spread_bps, decision, no_go_reason, timestamp
         FROM opportunities ORDER BY timestamp ASC, id ASC LIMIT ? OFFSET ?",
    )
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, estimated_profit_usd, gas_estimate_usd, spread_bps, decision, no_go_reason, timestamp)| {
                OpportunityRecord {
                    id,
                    estimated_profit_usd,
                    gas_estimate_usd,
                    spread_bps,
                    original_decision: decision,
                    original_no_go_reason: no_go_reason,
                    timestamp,
                }
            },
        )
        .collect())
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
        TradeStatus::Simulated => "simulated",
    }
}

fn trade_status_from_db(status: &str) -> TradeStatus {
    match status {
        "success" => TradeStatus::Success,
        "failed" => TradeStatus::Failed,
        "reverted" => TradeStatus::Reverted,
        "simulated" => TradeStatus::Simulated,
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

#[cfg(test)]
mod tests {
    use alloy::primitives::{b256, Address, U256};
    use sqlx::SqlitePool;
    use zelvex_types::{TradeResult, TradeStatus};

    use super::*;

    #[sqlx::test]
    async fn users_roundtrip(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        let count = get_user_count(&pool).await?;
        assert_eq!(count, 0);

        let id = insert_user(&pool, "admin", "hash").await?;
        assert!(id > 0);

        let count = get_user_count(&pool).await?;
        assert_eq!(count, 1);

        let user = get_user_by_username(&pool, "admin").await?;
        assert!(user.is_some());

        let missing = get_user_by_username(&pool, "missing").await?;
        assert!(missing.is_none());

        Ok(())
    }

    #[sqlx::test]
    async fn bot_state_roundtrip(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        set_bot_state(&pool, "last_block", "123").await?;
        let v = get_bot_state(&pool, "last_block").await?;
        assert_eq!(v.as_deref(), Some("123"));

        Ok(())
    }

    #[sqlx::test]
    async fn gas_history_insert_and_latest(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        insert_gas_sample(&pool, 10, 1.0, 2.0).await?;
        insert_gas_sample(&pool, 11, 3.0, 4.0).await?;

        let (block, base, priority): (i64, f64, f64) = sqlx::query_as(
            "SELECT block_number, base_fee_gwei, priority_fee_gwei FROM gas_history ORDER BY block_number DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await?;

        assert_eq!(block, 11);
        assert_eq!(base, 3.0);
        assert_eq!(priority, 4.0);

        Ok(())
    }

    #[sqlx::test]
    async fn trades_insert_and_pnl(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        let trade = TradeResult {
            tx_hash: b256!("0x1111111111111111111111111111111111111111111111111111111111111111"),
            route: "A->B".to_string(),
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            input_amount: U256::from(100u32),
            output_amount: Some(U256::from(110u32)),
            gross_profit: Some(U256::from(10u32)),
            gas_cost_usd: 1.5,
            net_profit_usd: Some(8.5),
            gas_used: Some(200_000),
            status: TradeStatus::Success,
            block_number: 1,
            timestamp: 1_700_000_000,
        };

        let _trade_id = insert_trade(&pool, &trade).await?;

        let pnl = get_pnl_summary(&pool).await?;
        assert_eq!(pnl.alltime_usd, 8.5);

        let trades = get_recent_trades(&pool, 10)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].route, "A->B");

        Ok(())
    }

    #[sqlx::test]
    async fn opportunities_insert(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        let opp = zelvex_types::ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 10.0,
            gas_estimate_usd: 2.0,
            spread_bps: 10,
        };

        let id = insert_opportunity(&pool, &opp, "no-go", Some("test"), None).await?;
        assert!(id > 0);

        Ok(())
    }

    #[sqlx::test]
    async fn get_opportunities_for_backtest_returns_rows(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        let base_opp = zelvex_types::ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 0.0,
            gas_estimate_usd: 0.0,
            spread_bps: 10,
        };

        // Row 1: profitable go (profit 20, gas 2)
        let mut opp = base_opp.clone();
        opp.estimated_profit_usd = 20.0;
        opp.gas_estimate_usd = 2.0;
        insert_opportunity(&pool, &opp, "go", None, None).await?;

        // Row 2: no-go (profit 3, gas 2)
        let mut opp = base_opp.clone();
        opp.estimated_profit_usd = 3.0;
        opp.gas_estimate_usd = 2.0;
        insert_opportunity(
            &pool,
            &opp,
            "no-go",
            Some("net_profit_usd 1.0000 <= min_profit_usd 5.0000"),
            None,
        )
        .await?;

        let rows = get_opportunities_for_backtest(&pool, 10, 0).await?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].original_decision, "go");
        assert!((rows[0].estimated_profit_usd - 20.0).abs() < 1e-9);
        assert_eq!(rows[1].original_decision, "no-go");
        assert!(rows[1].original_no_go_reason.is_some());

        Ok(())
    }

    #[sqlx::test]
    async fn backtest_end_to_end(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        let base_opp = zelvex_types::ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 0.0,
            gas_estimate_usd: 0.0,
            spread_bps: 10,
        };

        // 3 go rows at original min_profit=5
        for profit in [20.0_f64, 15.0, 10.0] {
            let mut opp = base_opp.clone();
            opp.estimated_profit_usd = profit;
            opp.gas_estimate_usd = 2.0;
            insert_opportunity(&pool, &opp, "go", None, None).await?;
        }

        // 2 no-go rows at original min_profit=5 (net profit 1 and 2)
        for profit in [3.0_f64, 4.0] {
            let mut opp = base_opp.clone();
            opp.estimated_profit_usd = profit;
            opp.gas_estimate_usd = 2.0;
            insert_opportunity(
                &pool,
                &opp,
                "no-go",
                Some("net_profit_usd <= min_profit_usd 5.0"),
                None,
            )
            .await?;
        }

        let rows = get_opportunities_for_backtest(&pool, 100, 0).await?;
        assert_eq!(rows.len(), 5);

        // Re-run at same threshold → all 5 should match
        let report = zelvex_sim::backtest::run_backtest(&rows, 5.0);
        assert_eq!(report.total, 5);
        assert_eq!(report.matched, 5);
        assert_eq!(report.mismatched, 0);
        assert_eq!(report.match_rate_pct, 100.0);

        // Re-run at raised threshold of 12 → rows with net 8 (10-2) now become no-go
        // net profits: 18, 13, 8, 1, 2 — only 18 and 13 are > 12 → 2 go, 3 no-go
        // Original was: go, go, go, no-go, no-go
        // Recomputed:   go, go, no-go, no-go, no-go
        // Mismatches: row 3 (was go, now no-go)
        let report_raised = zelvex_sim::backtest::run_backtest(&rows, 12.0);
        assert_eq!(report_raised.mismatched, 1);
        assert_eq!(report_raised.mismatches[0].original_decision, "go");
        assert_eq!(report_raised.mismatches[0].recomputed_decision, "no-go");

        Ok(())
    }

    #[sqlx::test]
    async fn backtest_pagination(pool: SqlitePool) -> sqlx::Result<()> {
        run_migrations(&pool).await?;

        let opp = zelvex_types::ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 10.0,
            gas_estimate_usd: 2.0,
            spread_bps: 10,
        };

        for _ in 0..5 {
            insert_opportunity(&pool, &opp, "go", None, None).await?;
        }

        let page1 = get_opportunities_for_backtest(&pool, 3, 0).await?;
        let page2 = get_opportunities_for_backtest(&pool, 3, 3).await?;
        assert_eq!(page1.len(), 3);
        assert_eq!(page2.len(), 2);

        Ok(())
    }
}
