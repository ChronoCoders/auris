use std::time::Duration;

use alloy::providers::Provider;
use futures_util::StreamExt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("websocket connection failed: {0}")]
    ConnectionFailed(String),
}

pub async fn get_next_head(ws_url: &str) -> Result<(u64, u64, Option<u128>), NodeError> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    let provider = alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|e| NodeError::ConnectionFailed(e.to_string()))?;

    let sub = provider
        .subscribe_blocks()
        .await
        .map_err(|e| NodeError::ConnectionFailed(e.to_string()))?;

    let mut stream = sub.into_stream();
    let Some(block) = stream.next().await else {
        return Err(NodeError::ConnectionFailed(
            "subscription ended".to_string(),
        ));
    };

    Ok((
        block.header.number,
        block.header.timestamp,
        block.header.base_fee_per_gas,
    ))
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
        .map_err(|e| NodeError::ConnectionFailed(e.to_string()))?;

    let sub = provider
        .subscribe_blocks()
        .await
        .map_err(|e| NodeError::ConnectionFailed(e.to_string()))?;

    let mut stream = sub.into_stream();
    while let Some(block) = stream.next().await {
        let block_number = block.header.number;
        let timestamp = block.header.timestamp;
        println!("block={} timestamp={}", block_number, timestamp);
    }

    Err(NodeError::ConnectionFailed(
        "subscription ended".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Error;

    use futures_util::SinkExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    #[tokio::test]
    async fn mock_ws_new_heads_parses_block_header() -> Result<(), Error> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ws_url = format!("ws://{}", addr);

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .map_err(|e| Error::other(e.to_string()))?;

            let bloom = format!("0x{}", "00".repeat(256));
            let head = serde_json::json!({
                "hash": format!("0x{}", "11".repeat(32)),
                "parentHash": format!("0x{}", "22".repeat(32)),
                "sha3Uncles": format!("0x{}", "33".repeat(32)),
                "miner": format!("0x{}", "44".repeat(20)),
                "stateRoot": format!("0x{}", "55".repeat(32)),
                "transactionsRoot": format!("0x{}", "66".repeat(32)),
                "receiptsRoot": format!("0x{}", "77".repeat(32)),
                "logsBloom": bloom,
                "difficulty": "0x0",
                "number": "0x14d5d61",
                "gasLimit": "0x1c9c380",
                "gasUsed": "0x0",
                "timestamp": "0x67fe736c",
                "extraData": "0x",
                "baseFeePerGas": "0x3b9aca00"
            });

            loop {
                let msg = ws
                    .next()
                    .await
                    .ok_or_else(|| Error::other("missing client request"))?
                    .map_err(|e| Error::other(e.to_string()))?;

                let text = match msg {
                    Message::Text(t) => t,
                    Message::Binary(b) => {
                        String::from_utf8(b).map_err(|e| Error::other(e.to_string()))?
                    }
                    _ => return Err(Error::other("unexpected message type")),
                };

                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| Error::other(e.to_string()))?;
                let id = v.get("id").cloned().unwrap_or_else(|| serde_json::json!(1));
                let method = v.get("method").and_then(|m| m.as_str()).unwrap_or_default();

                if method == "eth_subscribe" {
                    ws.send(Message::Text(
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": "0x1"
                        })
                        .to_string(),
                    ))
                    .await
                    .map_err(|e| Error::other(e.to_string()))?;

                    tokio::time::sleep(Duration::from_millis(50)).await;

                    ws.send(Message::Text(
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "eth_subscription",
                            "params": { "subscription": "0x1", "result": head }
                        })
                        .to_string(),
                    ))
                    .await
                    .map_err(|e| Error::other(e.to_string()))?;

                    break;
                }

                ws.send(Message::Text(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": "0x1"
                    })
                    .to_string(),
                ))
                .await
                .map_err(|e| Error::other(e.to_string()))?;
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<(), Error>(())
        });

        let (block_number, timestamp, base_fee) = get_next_head(&ws_url)
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        assert_eq!(block_number, 21847393);
        assert_eq!(timestamp, 1744728940);
        assert_eq!(base_fee, Some(1_000_000_000u128));

        server.await.map_err(|e| Error::other(e.to_string()))??;

        Ok(())
    }
}
