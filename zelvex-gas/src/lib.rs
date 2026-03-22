use std::collections::VecDeque;

use zelvex_types::GasEstimate;

#[derive(Debug, Clone)]
pub struct GasOracle {
    base_fee_history_wei: VecDeque<u64>,
    priority_fee_history_wei: VecDeque<u64>,
    last_base_fee_wei: u64,
    last_priority_fee_wei: u64,
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

    pub fn push_sample(&mut self, base_fee_wei: u64, priority_fee_wei: u64) {
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
        self.last_base_fee_wei
    }

    pub fn get_recommended_priority_fee(&self) -> u64 {
        let mut samples: Vec<u64> = self
            .priority_fee_history_wei
            .iter()
            .rev()
            .take(20)
            .copied()
            .collect();
        if samples.is_empty() {
            return self.last_priority_fee_wei;
        }
        samples.sort_unstable();
        let idx = ((samples.len() - 1) as f64 * 0.90).round() as usize;
        samples[idx]
    }

    pub fn get_current_gas_estimate(&self) -> GasEstimate {
        let priority_wei = self.get_recommended_priority_fee();
        let total_wei = self.last_base_fee_wei.saturating_add(priority_wei);

        GasEstimate {
            base_fee_gwei: self.last_base_fee_wei as f64 / 1_000_000_000.0,
            priority_fee_gwei: priority_wei as f64 / 1_000_000_000.0,
            recommended_total_gwei: total_wei as f64 / 1_000_000_000.0,
        }
    }

    pub fn estimate_gas_cost_usd(&self, gas_units: u64, eth_price_usd: f64) -> f64 {
        let priority_wei = self.get_recommended_priority_fee();
        let total_wei = self.last_base_fee_wei.saturating_add(priority_wei) as f64;

        let eth_cost = (gas_units as f64 * total_wei) / 1_000_000_000_000_000_000.0;
        eth_cost * eth_price_usd
    }
}

impl Default for GasOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_stores_exactly_100_samples() {
        let mut oracle = GasOracle::new();
        for i in 0..150u64 {
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
        for i in 0..25u64 {
            oracle.push_sample(1, i);
        }
        let recommended = oracle.get_recommended_priority_fee();
        assert_eq!(recommended, 22);
    }
}
