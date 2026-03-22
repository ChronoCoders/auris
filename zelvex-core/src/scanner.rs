use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::primitives::{Address, U256};
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex};
use zelvex_gas::GasOracle;
use zelvex_sim::evaluate;
use zelvex_types::{ArbitrageOpportunity, ProfitDecision};

use crate::{amm, sync::PoolStore};

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

const WETH: Address = alloy::primitives::address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
const USDC: Address = alloy::primitives::address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
const DAI: Address = alloy::primitives::address!("0x6B175474E89094C44Da98b954EedeAC495271d0F");

#[derive(Debug, Error)]
pub enum ScannerError {
    #[error("node websocket connection failed: {0}")]
    Ws(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub async fn run_scanner(
    ws_url: String,
    mut updates: mpsc::Receiver<()>,
    pool_pairs: Vec<(Address, Address)>,
    store: Arc<Mutex<PoolStore>>,
    db: SqlitePool,
    gas_oracle: Arc<Mutex<GasOracle>>,
    min_profit_usd: f64,
) -> Result<(), ScannerError> {
    let provider = ws_provider(&ws_url).await?;

    let mut eth_price_usd = 0.0;
    let mut last_price_fetch = Instant::now() - Duration::from_secs(60);

    while updates.recv().await.is_some() {
        while updates.try_recv().is_ok() {}

        if last_price_fetch.elapsed() >= Duration::from_secs(30) {
            eth_price_usd = fetch_eth_price_usd(&provider).await?;
            last_price_fetch = Instant::now();
        }

        let gas = {
            let oracle = gas_oracle.lock().await;
            oracle.get_current_gas_estimate()
        };

        let opportunities = {
            let store = store.lock().await;
            scan_pairs(&store, &pool_pairs, eth_price_usd)
        };

        for opp in opportunities {
            let decision = evaluate(&opp, &gas, eth_price_usd, min_profit_usd).await;
            let (decision_str, reason) = match decision {
                ProfitDecision::Go { .. } => ("go", None),
                ProfitDecision::NoGo { reason } => ("no-go", Some(reason)),
            };

            zelvex_db::insert_opportunity(&db, &opp, decision_str, reason.as_deref(), None).await?;
        }
    }

    Ok(())
}

fn scan_pairs(
    store: &PoolStore,
    pool_pairs: &[(Address, Address)],
    eth_price_usd: f64,
) -> Vec<ArbitrageOpportunity> {
    let mut out = Vec::new();

    for (a, b) in pool_pairs {
        let Some(pool_a) = store.get(a) else {
            continue;
        };
        let Some(pool_b) = store.get(b) else {
            continue;
        };
        let Some(token_out) =
            stable_token_out(pool_a.token0, pool_a.token1, pool_b.token0, pool_b.token1)
        else {
            continue;
        };

        let amount_in = U256::from(1_000_000_000_000_000_000u128);

        let opp_ab = build_opportunity(pool_a, pool_b, token_out, amount_in, eth_price_usd);
        let opp_ba = build_opportunity(pool_b, pool_a, token_out, amount_in, eth_price_usd);

        match (opp_ab, opp_ba) {
            (Some(a), Some(b)) => {
                if a.estimated_profit_usd >= b.estimated_profit_usd {
                    out.push(a);
                } else {
                    out.push(b);
                }
            }
            (Some(a), None) => out.push(a),
            (None, Some(b)) => out.push(b),
            (None, None) => {}
        }
    }

    out
}

fn stable_token_out(a0: Address, a1: Address, b0: Address, b1: Address) -> Option<Address> {
    let pair_a = (a0, a1);
    let pair_b = (b0, b1);
    if pair_a != pair_b {
        return None;
    }
    if a0 == Address::ZERO || a1 == Address::ZERO {
        return None;
    }
    if a0 != WETH && a1 != WETH {
        return None;
    }
    if a0 == USDC || a1 == USDC {
        Some(USDC)
    } else if a0 == DAI || a1 == DAI {
        Some(DAI)
    } else {
        None
    }
}

fn build_opportunity(
    pool_a: &zelvex_types::Pool,
    pool_b: &zelvex_types::Pool,
    token_out: Address,
    amount_in: U256,
    eth_price_usd: f64,
) -> Option<ArbitrageOpportunity> {
    let (a_in, a_out) = reserves_for_swap(pool_a, WETH, token_out)?;
    let (b_in, b_out) = reserves_for_swap(pool_b, token_out, WETH)?;

    if a_in.is_zero() || a_out.is_zero() || b_in.is_zero() || b_out.is_zero() {
        return None;
    }

    let profit_wei = amm::calculate_profit(amount_in, a_in, a_out, b_in, b_out)?;
    if profit_wei.is_zero() {
        return None;
    }

    let estimated_profit_usd = wei_to_eth_f64(profit_wei) * eth_price_usd;
    if estimated_profit_usd <= 0.0 {
        return None;
    }

    let price_a = amm::get_spot_price(a_in, a_out);
    let price_b = amm::get_spot_price(b_out, b_in);
    let spread_bps = spread_bps(price_a, price_b);

    Some(ArbitrageOpportunity {
        pool_a: pool_a.pool_address,
        pool_b: pool_b.pool_address,
        token_in: WETH,
        token_out,
        input_amount: amount_in,
        estimated_profit_usd,
        gas_estimate_usd: 0.0,
        spread_bps,
    })
}

fn reserves_for_swap(
    pool: &zelvex_types::Pool,
    token_in: Address,
    token_out: Address,
) -> Option<(U256, U256)> {
    if pool.token0 == token_in && pool.token1 == token_out {
        Some((pool.reserve0, pool.reserve1))
    } else if pool.token1 == token_in && pool.token0 == token_out {
        Some((pool.reserve1, pool.reserve0))
    } else {
        None
    }
}

fn spread_bps(price_a: U256, price_b: U256) -> i64 {
    let min = if price_a < price_b { price_a } else { price_b };
    if min.is_zero() {
        return 0;
    }
    let diff = if price_a > price_b {
        price_a - price_b
    } else {
        price_b - price_a
    };
    let bps = (diff * U256::from(10_000u32)) / min;
    let s = bps.to_string();
    s.parse::<i64>().unwrap_or(i64::MAX)
}

fn wei_to_eth_f64(wei: U256) -> f64 {
    let s = wei.to_string();
    let Ok(v) = s.parse::<f64>() else {
        return 0.0;
    };
    v / 1_000_000_000_000_000_000.0
}

type WsProvider = alloy::providers::RootProvider<alloy::pubsub::PubSubFrontend>;

async fn ws_provider(ws_url: &str) -> Result<WsProvider, ScannerError> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|e| ScannerError::Ws(e.to_string()))
}

async fn fetch_eth_price_usd(provider: &WsProvider) -> Result<f64, ScannerError> {
    let feed = AggregatorV3Interface::new(CHAINLINK_ETH_USD_FEED, provider);

    let AggregatorV3Interface::decimalsReturn { _0: decimals } = feed
        .decimals()
        .call()
        .await
        .map_err(|e| ScannerError::Ws(e.to_string()))?;

    let AggregatorV3Interface::latestRoundDataReturn { answer, .. } = feed
        .latestRoundData()
        .call()
        .await
        .map_err(|e| ScannerError::Ws(e.to_string()))?;

    let raw: f64 = answer.to_string().parse().unwrap_or(0.0);
    Ok(raw / 10f64.powi(decimals as i32))
}

#[cfg(test)]
mod tests {
    use alloy::primitives::address;

    use super::*;

    #[test]
    fn scan_pairs_detects_profitable_roundtrip() {
        let weth_unit = U256::from(1_000_000_000_000_000_000u128);
        let usdc_unit = U256::from(1_000_000u32);

        let pool_a = zelvex_types::Pool {
            pool_address: address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
            token0: USDC,
            token1: WETH,
            reserve0: U256::from(20_000_000u64) * usdc_unit,
            reserve1: U256::from(10_000u64) * weth_unit,
            block_updated: 1,
        };

        let pool_b = zelvex_types::Pool {
            pool_address: address!("0x397FF1542f962076d0BFE58eA045FfA2d347ACa0"),
            token0: USDC,
            token1: WETH,
            reserve0: U256::from(18_000_000u64) * usdc_unit,
            reserve1: U256::from(10_000u64) * weth_unit,
            block_updated: 1,
        };

        let mut store = PoolStore::new();
        store.upsert_pool(pool_a);
        store.upsert_pool(pool_b);

        let pairs = vec![(
            address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
            address!("0x397FF1542f962076d0BFE58eA045FfA2d347ACa0"),
        )];

        let opps = scan_pairs(&store, &pairs, 3000.0);
        assert_eq!(opps.len(), 1);
        assert!(opps[0].estimated_profit_usd >= 0.0);
    }
}
