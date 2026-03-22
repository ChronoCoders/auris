use std::time::Duration;

use alloy::providers::Provider;
use futures_util::StreamExt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("websocket connection failed")]
    ConnectionFailed,
}

pub async fn subscribe_new_heads(ws_url: &str) -> Result<(), NodeError> {
    let mut attempts = 0u32;
    let mut backoff = Duration::from_secs(1);
    loop {
        match subscribe_new_heads_once(ws_url).await {
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

async fn subscribe_new_heads_once(ws_url: &str) -> Result<(), NodeError> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    let provider = alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|_| NodeError::ConnectionFailed)?;

    let sub = provider
        .subscribe_blocks()
        .await
        .map_err(|_| NodeError::ConnectionFailed)?;

    let mut stream = sub.into_stream();
    while let Some(block) = stream.next().await {
        let block_number = block.header.number;
        let timestamp = block.header.timestamp;
        println!("block={} timestamp={}", block_number, timestamp);
    }

    Err(NodeError::ConnectionFailed)
}
