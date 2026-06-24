use std::time::Duration;

use alloy::{
    primitives::TxHash,
    providers::{Provider, RootProvider},
    transports::BoxTransport,
};

use crate::ExecError;

/// Poll for transaction inclusion until `target_block + 2`.
///
/// Returns `true` if the transaction was included, `false` if the deadline passed.
pub async fn wait_for_inclusion(
    provider: &RootProvider<BoxTransport>,
    tx_hash: TxHash,
    target_block: u64,
) -> Result<bool, ExecError> {
    let deadline = target_block + 2;

    loop {
        let receipt = provider
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(|e| ExecError::ProviderError(e.to_string()))?;

        if receipt.is_some() {
            return Ok(true);
        }

        let current = provider
            .get_block_number()
            .await
            .map_err(|e| ExecError::ProviderError(e.to_string()))?;

        if current > deadline {
            return Ok(false);
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
