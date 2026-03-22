use std::path::Path;

use alloy::primitives::B256;
use alloy::signers::local::PrivateKeySigner;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("failed reading key file: {0}")]
    ReadKey(#[from] std::io::Error),
    #[error("invalid key hex")]
    InvalidKey,
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
