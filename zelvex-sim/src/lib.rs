pub mod backtest;

use zelvex_types::{ArbitrageOpportunity, GasEstimate, ProfitDecision};

pub fn evaluate(
    opp: &ArbitrageOpportunity,
    gas: &GasEstimate,
    eth_price_usd: f64,
    min_profit_usd: f64,
) -> ProfitDecision {
    let gas_units = 200_000u64;
    let gas_cost_eth = (gas.recommended_total_gwei * gas_units as f64) / 1_000_000_000.0;
    let gas_cost_usd = gas_cost_eth * eth_price_usd;
    let net_profit_usd = opp.estimated_profit_usd - gas_cost_usd;

    if net_profit_usd > min_profit_usd {
        ProfitDecision::Go {
            net_profit_usd,
            gas_cost_usd,
        }
    } else {
        ProfitDecision::NoGo {
            reason: format!(
                "net_profit_usd {:.4} <= min_profit_usd {:.4}",
                net_profit_usd, min_profit_usd
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::{Address, U256};

    use super::*;

    #[test]
    fn net_profit_above_threshold_returns_go() {
        let opp = ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 20.0,
            gas_estimate_usd: 2.0,
            spread_bps: 10,
        };
        let gas = GasEstimate {
            base_fee_gwei: 10.0,
            priority_fee_gwei: 1.0,
            recommended_total_gwei: 11.0,
        };
        let decision = evaluate(&opp, &gas, 3000.0, 5.0);
        assert!(matches!(decision, ProfitDecision::Go { .. }));
    }

    #[test]
    fn net_profit_below_threshold_returns_no_go() {
        let opp = ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 3.0,
            gas_estimate_usd: 2.5,
            spread_bps: 10,
        };
        let gas = GasEstimate {
            base_fee_gwei: 100.0,
            priority_fee_gwei: 5.0,
            recommended_total_gwei: 105.0,
        };
        let decision = evaluate(&opp, &gas, 3000.0, 1.0);
        assert!(matches!(decision, ProfitDecision::NoGo { .. }));
    }

    #[test]
    fn zero_profit_returns_no_go() {
        // estimated_profit_usd exactly equals gas_cost_usd → net = 0, not > min_profit 0
        // gas: 200_000 units * 10 gwei total / 1e9 * 3000 USD/ETH = 6.0 USD
        let opp = ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 6.0,
            gas_estimate_usd: 6.0,
            spread_bps: 0,
        };
        let gas = GasEstimate {
            base_fee_gwei: 10.0,
            priority_fee_gwei: 0.0,
            recommended_total_gwei: 10.0,
        };
        // net = 6.0 - (200_000 * 10 / 1e9 * 3000) = 6.0 - 6.0 = 0.0
        // 0.0 is not > 0.0, so NoGo
        let decision = evaluate(&opp, &gas, 3000.0, 0.0);
        assert!(matches!(decision, ProfitDecision::NoGo { .. }));
    }

    #[test]
    fn negative_profit_gas_exceeds_gross_returns_no_go() {
        // Gas cost exceeds estimated profit → negative net
        let opp = ArbitrageOpportunity {
            pool_a: Address::ZERO,
            pool_b: Address::ZERO,
            token_in: Address::ZERO,
            token_out: Address::ZERO,
            input_amount: U256::from(1u32),
            estimated_profit_usd: 1.0,
            gas_estimate_usd: 20.0,
            spread_bps: 5,
        };
        let gas = GasEstimate {
            base_fee_gwei: 200.0,
            priority_fee_gwei: 10.0,
            recommended_total_gwei: 210.0,
        };
        let decision = evaluate(&opp, &gas, 3000.0, 0.0);
        assert!(matches!(decision, ProfitDecision::NoGo { .. }));
    }
}
