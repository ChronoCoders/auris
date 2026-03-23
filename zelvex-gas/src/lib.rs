use std::{collections::VecDeque, time::Duration};

use alloy::providers::Provider;
use futures_util::StreamExt;
use sqlx::SqlitePool;
use thiserror::Error;
use tokio::sync::Mutex;
use zelvex_types::GasEstimate;

#[derive(Debug, Clone)]
pub struct GasOracle {
    base_fee_history_wei: VecDeque<u128>,
    priority_fee_history_wei: VecDeque<u128>,
    last_base_fee_wei: u128,
    last_priority_fee_wei: u128,
}

impl GasOracle {
    pub fn new() -> Self {
        Self {
            base_fee_history_wei: VecDeque::with_capacity(100),
            priority_fee_history_wei: VecDeque::with_capacity(100),
            last_base_fee_wei: 0,
            last_priority_fee_wei: 0,
        }
    }

    pub fn push_sample(&mut self, base_fee_wei: u128, priority_fee_wei: u128) {
        self.last_base_fee_wei = base_fee_wei;
        self.last_priority_fee_wei = priority_fee_wei;

        if self.base_fee_history_wei.len() == 100 {
            self.base_fee_history_wei.pop_front();
        }
        if self.priority_fee_history_wei.len() == 100 {
            self.priority_fee_history_wei.pop_front();
        }

        self.base_fee_history_wei.push_back(base_fee_wei);
        self.priority_fee_history_wei.push_back(priority_fee_wei);
    }

    pub fn get_current_base_fee(&self) -> u64 {
        u128_to_u64_saturating(self.last_base_fee_wei)
    }

    pub fn get_recommended_priority_fee(&self) -> u64 {
        let mut samples: Vec<u128> = self
            .priority_fee_history_wei
            .iter()
            .rev()
            .take(20)
            .copied()
            .collect();
        if samples.is_empty() {
            return u128_to_u64_saturating(self.last_priority_fee_wei);
        }
        samples.sort_unstable();
        let idx = ((samples.len() - 1) as f64 * 0.90).round() as usize;
        u128_to_u64_saturating(samples[idx])
    }

    pub fn get_current_gas_estimate(&self) -> GasEstimate {
        let priority_wei = self.get_recommended_priority_fee() as u128;
        let total_wei = self.last_base_fee_wei.saturating_add(priority_wei);

        GasEstimate {
            base_fee_gwei: self.last_base_fee_wei as f64 / 1_000_000_000.0,
            priority_fee_gwei: priority_wei as f64 / 1_000_000_000.0,
            recommended_total_gwei: total_wei as f64 / 1_000_000_000.0,
        }
    }

    pub fn estimate_gas_cost_usd(&self, gas_units: u64, eth_price_usd: f64) -> f64 {
        let priority_wei = self.get_recommended_priority_fee() as u128;
        let total_wei = self.last_base_fee_wei.saturating_add(priority_wei) as f64;

        let eth_cost = (gas_units as f64 * total_wei) / 1_000_000_000_000_000_000.0;
        eth_cost * eth_price_usd
    }
}

fn u128_to_u64_saturating(value: u128) -> u64 {
    if value > u64::MAX as u128 {
        u64::MAX
    } else {
        value as u64
    }
}

impl Default for GasOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Error)]
pub enum GasSamplerError {
    #[error("websocket connection failed")]
    ConnectionFailed,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub async fn run_gas_sampler(
    ws_url: &str,
    pool: SqlitePool,
    oracle: std::sync::Arc<Mutex<GasOracle>>,
) -> Result<(), GasSamplerError> {
    let mut attempts = 0u32;
    let mut backoff = Duration::from_secs(1);

    loop {
        match run_gas_sampler_once(ws_url, &pool, &oracle).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempts += 1;
                if attempts >= 10 {
                    return Err(e);
                }
                tokio::time::sleep(backoff).await;
                backoff = backoff.saturating_mul(2);
            }
        }
    }
}

async fn run_gas_sampler_once(
    ws_url: &str,
    pool: &SqlitePool,
    oracle: &std::sync::Arc<Mutex<GasOracle>>,
) -> Result<(), GasSamplerError> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    let provider = alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|_| GasSamplerError::ConnectionFailed)?;

    let sub = provider
        .subscribe_blocks()
        .await
        .map_err(|_| GasSamplerError::ConnectionFailed)?;

    let mut stream = sub.into_stream();
    while let Some(block) = stream.next().await {
        let block_number = block.header.number;
        let timestamp = block.header.timestamp;
        let base_fee_wei = block.header.base_fee_per_gas.unwrap_or(0);
        let priority_fee_wei = provider
            .get_max_priority_fee_per_gas()
            .await
            .map_err(|_| GasSamplerError::ConnectionFailed)?;

        {
            let mut oracle = oracle.lock().await;
            oracle.push_sample(base_fee_wei, priority_fee_wei);
        }

        let base_gwei = base_fee_wei as f64 / 1_000_000_000.0;
        let priority_gwei = priority_fee_wei as f64 / 1_000_000_000.0;

        zelvex_db::insert_gas_sample(pool, block_number, base_gwei, priority_gwei).await?;
        zelvex_db::set_bot_state(pool, "last_block", &block_number.to_string()).await?;

        tracing::info!(block = block_number, timestamp, "new block");
    }

    Err(GasSamplerError::ConnectionFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_stores_exactly_100_samples() {
        let mut oracle = GasOracle::new();
        for i in 0..150u128 {
            oracle.push_sample(i, i);
        }
        assert_eq!(oracle.base_fee_history_wei.len(), 100);
        assert_eq!(oracle.base_fee_history_wei.front().copied(), Some(50));
        assert_eq!(oracle.base_fee_history_wei.back().copied(), Some(149));
        assert_eq!(oracle.priority_fee_history_wei.len(), 100);
    }

    #[test]
    fn recommended_priority_fee_returns_p90_of_last_20() {
        let mut oracle = GasOracle::new();
        for i in 0..25u128 {
            oracle.push_sample(1, i);
        }
        let recommended = oracle.get_recommended_priority_fee();
        assert_eq!(recommended, 22);
    }

    #[test]
    fn estimate_gas_cost_usd_returns_correct_usd() {
        let mut oracle = GasOracle::new();
        // base 10 gwei, priority 1 gwei → total 11 gwei = 11_000_000_000 wei
        let base_fee_wei = 10_000_000_000u128;
        let priority_fee_wei = 1_000_000_000u128;
        oracle.push_sample(base_fee_wei, priority_fee_wei);

        // 200_000 gas * 11e9 wei/gas / 1e18 wei/ETH = 0.0022 ETH
        // 0.0022 ETH * $3000/ETH = $6.60
        let cost = oracle.estimate_gas_cost_usd(200_000, 3000.0);
        let expected = 6.6_f64;
        assert!(
            (cost - expected).abs() < 1e-6,
            "expected ~{expected} USD, got {cost}"
        );
    }
}
