use std::{path::Path, sync::Arc};

use alloy::{
    consensus::{SignableTransaction, TxEip1559, TxEnvelope},
    eips::{eip2718::Encodable2718, eip2930::AccessList},
    primitives::{keccak256, Address, Bytes, TxHash, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::BlockNumberOrTag,
    signers::{local::PrivateKeySigner, Signer},
    transports::BoxTransport,
};
use sqlx::SqlitePool;
use tracing::info;
use zelvex_types::{ArbitrageOpportunity, TradeResult, TradeStatus};

use crate::{
    flashbots::FlashbotsClient,
    inclusion::wait_for_inclusion,
    load_signer_from_file,
    tx_builder::build_execute_arb_calldata,
    ExecError,
};

pub struct ArbitrageExecutor {
    signer: PrivateKeySigner,
    flashbots: FlashbotsClient,
    provider: Arc<RootProvider<BoxTransport>>,
    contract_address: Address,
    paper_trade: bool,
    db: SqlitePool,
}

impl ArbitrageExecutor {
    pub async fn new(
        ws_url: &str,
        signer_key_path: &Path,
        flashbots_key_path: &Path,
        relay_url: &str,
        contract_address: Address,
        paper_trade: bool,
        db: SqlitePool,
    ) -> Result<Self, ExecError> {
        let signer = load_signer_from_file(signer_key_path)?;
        let flashbots = FlashbotsClient::new(relay_url.to_string(), flashbots_key_path)?;

        let provider = ProviderBuilder::new()
            .on_builtin(ws_url)
            .await
            .map_err(|e| ExecError::ProviderError(e.to_string()))?;

        Ok(Self {
            signer,
            flashbots,
            provider: Arc::new(provider),
            contract_address,
            paper_trade,
            db,
        })
    }

    /// Build, sign, and either simulate or submit an arbitrage transaction.
    ///
    /// Returns the database row ID of the inserted trade record.
    pub async fn execute(
        &self,
        opp: &ArbitrageOpportunity,
        min_profit: U256,
        current_block: u64,
    ) -> Result<i64, ExecError> {
        let calldata = build_execute_arb_calldata(
            opp.token_in,
            opp.token_out,
            opp.pool_a,
            opp.pool_b,
            opp.input_amount,
            min_profit,
        );

        let signed_bytes = self.build_signed_tx(calldata).await?;

        // Derive tx hash from the raw signed bytes
        let tx_hash: TxHash = keccak256(&signed_bytes);

        let target_block = current_block + 1;
        let route = format!("{:#x}->{:#x}", opp.pool_a, opp.pool_b);

        if self.paper_trade {
            let sim = self
                .flashbots
                .simulate_bundle(&signed_bytes, target_block)
                .await?;

            let gas_used = if sim.gas_used > 0 {
                Some(sim.gas_used)
            } else {
                None
            };

            let trade = TradeResult {
                tx_hash,
                route,
                pool_a: opp.pool_a,
                pool_b: opp.pool_b,
                input_amount: opp.input_amount,
                output_amount: None,
                gross_profit: None,
                gas_cost_usd: opp.gas_estimate_usd,
                net_profit_usd: None,
                gas_used,
                status: TradeStatus::Simulated,
                block_number: target_block,
                timestamp: current_timestamp(),
            };

            info!(
                tx_hash = ?tx_hash,
                sim_success = sim.success,
                gas_used = sim.gas_used,
                sim_error = ?sim.error,
                "paper trade simulation complete"
            );

            let trade_id = zelvex_db::insert_trade(&self.db, &trade)
                .await
                .map_err(|e| ExecError::DbError(e.to_string()))?;

            Ok(trade_id)
        } else {
            let bundle_hash = self
                .flashbots
                .submit_bundle(&signed_bytes, target_block)
                .await?;

            info!(
                tx_hash = ?tx_hash,
                bundle_hash = ?bundle_hash,
                target_block,
                "bundle submitted to Flashbots"
            );

            let included =
                wait_for_inclusion(&self.provider, tx_hash, target_block).await?;

            let status = if included {
                TradeStatus::Success
            } else {
                TradeStatus::Failed
            };

            let trade = TradeResult {
                tx_hash,
                route,
                pool_a: opp.pool_a,
                pool_b: opp.pool_b,
                input_amount: opp.input_amount,
                output_amount: None,
                gross_profit: None,
                gas_cost_usd: opp.gas_estimate_usd,
                net_profit_usd: None,
                gas_used: None,
                status,
                block_number: target_block,
                timestamp: current_timestamp(),
            };

            let trade_id = zelvex_db::insert_trade(&self.db, &trade)
                .await
                .map_err(|e| ExecError::DbError(e.to_string()))?;

            info!(trade_id, included, "trade recorded");

            Ok(trade_id)
        }
    }

    /// Build and sign an EIP-1559 transaction targeting the ZelvexArb contract.
    async fn build_signed_tx(&self, input: Bytes) -> Result<Bytes, ExecError> {
        let from = self.signer.address();

        let nonce = self
            .provider
            .get_transaction_count(from)
            .await
            .map_err(|e| ExecError::ProviderError(e.to_string()))?;

        let block = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Latest, false)
            .await
            .map_err(|e| ExecError::ProviderError(e.to_string()))?
            .ok_or_else(|| {
                ExecError::ProviderError("latest block not found".to_string())
            })?;

        let base_fee = block
            .header
            .base_fee_per_gas
            .ok_or_else(|| {
                ExecError::ProviderError("base_fee_per_gas missing from block".to_string())
            })?;

        let priority_fee: u128 = 1_500_000_000; // 1.5 gwei
        let max_fee = base_fee as u128 * 2 + priority_fee;

        let tx = TxEip1559 {
            chain_id: 1,
            nonce,
            gas_limit: 250_000,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: priority_fee,
            to: alloy::primitives::TxKind::Call(self.contract_address),
            value: U256::ZERO,
            access_list: AccessList::default(),
            input,
        };

        let sig = self
            .signer
            .sign_hash(&tx.signature_hash())
            .await
            .map_err(|e| ExecError::SigningFailed(e.to_string()))?;

        let signed_tx = tx.into_signed(sig);
        let envelope = TxEnvelope::Eip1559(signed_tx);

        let mut raw = Vec::new();
        envelope.encode_2718(&mut raw);

        Ok(Bytes::from(raw))
    }
}

fn current_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
