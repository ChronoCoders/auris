use alloy::primitives::{Address, TxHash, U256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pool {
    pub pool_address: Address,
    pub token0: Address,
    pub token1: Address,
    pub reserve0: U256,
    pub reserve1: U256,
    pub block_updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    pub pool_a: Address,
    pub pool_b: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub input_amount: U256,
    pub estimated_profit_usd: f64,
    pub gas_estimate_usd: f64,
    pub spread_bps: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    pub tx_hash: TxHash,
    pub route: String,
    pub pool_a: Address,
    pub pool_b: Address,
    pub input_amount: U256,
    pub output_amount: Option<U256>,
    pub gross_profit: Option<U256>,
    pub gas_cost_usd: f64,
    pub net_profit_usd: Option<f64>,
    pub gas_used: Option<u64>,
    pub status: TradeStatus,
    pub block_number: u64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeStatus {
    Success,
    Failed,
    Reverted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "error", rename_all = "lowercase")]
pub enum BotStatus {
    Running,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub address0: Address,
    pub address1: Address,
    pub decimals0: u8,
    pub decimals1: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub base_fee_gwei: f64,
    pub priority_fee_gwei: f64,
    pub recommended_total_gwei: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "lowercase")]
pub enum ProfitDecision {
    Go {
        net_profit_usd: f64,
        gas_cost_usd: f64,
    },
    #[serde(rename_all = "lowercase")]
    NoGo { reason: String },
}
