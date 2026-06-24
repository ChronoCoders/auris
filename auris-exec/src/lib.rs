pub mod executor;
pub mod flashbots;
pub mod inclusion;
pub mod tx_builder;

pub use executor::ArbitrageExecutor;
pub use flashbots::{FlashbotsClient, SimulationResult};
pub use tx_builder::build_execute_arb_calldata;

use std::path::Path;

use alloy::{primitives::B256, signers::local::PrivateKeySigner};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("failed reading key file: {0}")]
    ReadKey(#[from] std::io::Error),
    #[error("invalid key hex")]
    InvalidKey,
    #[error("key load error: {0}")]
    KeyLoad(String),
    #[error("signing failed: {0}")]
    SigningFailed(String),
    #[error("provider error: {0}")]
    ProviderError(String),
    #[error("flashbots error: {0}")]
    FlashbotsError(String),
    #[error("database error: {0}")]
    DbError(String),
}

pub fn load_signer_from_file(path: &Path) -> Result<PrivateKeySigner, ExecError> {
    let raw = std::fs::read_to_string(path)?;
    let hex = raw.trim();
    let bytes = hex::decode(hex).map_err(|_| ExecError::InvalidKey)?;
    if bytes.len() != 32 {
        return Err(ExecError::InvalidKey);
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let b256 = B256::from(arr);
    PrivateKeySigner::from_bytes(&b256).map_err(|_| ExecError::InvalidKey)
}
