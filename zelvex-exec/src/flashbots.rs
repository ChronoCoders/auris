use std::path::Path;

use alloy::{
    primitives::{keccak256, Bytes, B256},
    signers::{local::PrivateKeySigner, Signer},
};
use serde_json::json;
use tracing::{debug, warn};

use crate::ExecError;

pub struct FlashbotsClient {
    relay_url: String,
    signing_key: PrivateKeySigner,
    http_client: reqwest::Client,
}

/// Result of simulating a bundle via `eth_callBundle`.
pub struct SimulationResult {
    pub success: bool,
    pub gas_used: u64,
    pub error: Option<String>,
}

impl FlashbotsClient {
    /// Load a Flashbots identity key from a file (single line, lowercase hex, no 0x prefix).
    pub fn new(relay_url: String, flashbots_key_path: &Path) -> Result<Self, ExecError> {
        let raw = std::fs::read_to_string(flashbots_key_path)
            .map_err(|e| ExecError::KeyLoad(e.to_string()))?;
        let hex = raw.trim();
        let signing_key: PrivateKeySigner = hex
            .parse()
            .map_err(|_| ExecError::KeyLoad("invalid flashbots key hex".to_string()))?;

        Ok(Self {
            relay_url,
            signing_key,
            http_client: reqwest::Client::new(),
        })
    }

    /// Submit a bundle to the Flashbots relay.
    ///
    /// Returns the bundle hash assigned by the relay.
    pub async fn submit_bundle(
        &self,
        signed_tx: &Bytes,
        target_block: u64,
    ) -> Result<B256, ExecError> {
        let bundle_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_sendBundle",
            "params": [{
                "txs": [format!("0x{}", hex::encode(signed_tx.as_ref()))],
                "blockNumber": format!("0x{target_block:x}"),
                "minTimestamp": 0,
                "maxTimestamp": 0,
            }]
        });

        let body_str = bundle_body.to_string();
        let header_val = self.sign_bundle_header(&body_str).await?;

        debug!(target_block, "submitting bundle to Flashbots relay");

        let resp = self
            .http_client
            .post(&self.relay_url)
            .header("X-Flashbots-Signature", header_val)
            .header("Content-Type", "application/json")
            .json(&bundle_body)
            .send()
            .await
            .map_err(|e| ExecError::FlashbotsError(e.to_string()))?;

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ExecError::FlashbotsError(e.to_string()))?;

        if let Some(err) = result.get("error") {
            return Err(ExecError::FlashbotsError(err.to_string()));
        }

        let bundle_hash = result["result"]["bundleHash"]
            .as_str()
            .ok_or_else(|| ExecError::FlashbotsError("no bundleHash in response".to_string()))?
            .parse::<B256>()
            .map_err(|e| ExecError::FlashbotsError(e.to_string()))?;

        Ok(bundle_hash)
    }

    /// Simulate a bundle via `eth_callBundle`.
    pub async fn simulate_bundle(
        &self,
        signed_tx: &Bytes,
        target_block: u64,
    ) -> Result<SimulationResult, ExecError> {
        let bundle_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_callBundle",
            "params": [{
                "txs": [format!("0x{}", hex::encode(signed_tx.as_ref()))],
                "blockNumber": format!("0x{target_block:x}"),
                "stateBlockNumber": "latest",
            }]
        });

        let body_str = bundle_body.to_string();
        let header_val = self.sign_bundle_header(&body_str).await?;

        debug!(target_block, "simulating bundle via Flashbots relay");

        let resp = self
            .http_client
            .post(&self.relay_url)
            .header("X-Flashbots-Signature", header_val)
            .header("Content-Type", "application/json")
            .json(&bundle_body)
            .send()
            .await
            .map_err(|e| ExecError::FlashbotsError(e.to_string()))?;

        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ExecError::FlashbotsError(e.to_string()))?;

        if let Some(err) = result.get("error") {
            return Err(ExecError::FlashbotsError(err.to_string()));
        }

        // Parse the first result entry
        let first = result["result"]["results"]
            .get(0)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let error_msg = first["error"].as_str().map(|s| s.to_string());
        let success = error_msg.is_none();
        let gas_used = first["gasUsed"].as_u64().unwrap_or(0);

        if !success {
            warn!(
                error = ?error_msg,
                "bundle simulation returned error"
            );
        }

        Ok(SimulationResult {
            success,
            gas_used,
            error: error_msg,
        })
    }

    /// Sign the bundle JSON body for the `X-Flashbots-Signature` header.
    ///
    /// Returns `"<address>:0x<sig>"`.
    async fn sign_bundle_header(&self, body: &str) -> Result<String, ExecError> {
        let hash = keccak256(body.as_bytes());
        let sig = self
            .signing_key
            .sign_hash(&hash)
            .await
            .map_err(|e| ExecError::SigningFailed(e.to_string()))?;
        let address = self.signing_key.address();
        Ok(format!("{address}:0x{}", hex::encode(sig.as_bytes())))
    }
}
