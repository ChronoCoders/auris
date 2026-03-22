use std::sync::Arc;

use alloy::primitives::Address;
use alloy::providers::Provider;
use futures_util::StreamExt;
use tokio::sync::Mutex;

use crate::node::NodeError;
use crate::sync::{decode_sync_reserves, PoolStore};

pub async fn subscribe_sync_events(
    ws_url: &str,
    pools: Vec<Address>,
    store: Arc<Mutex<PoolStore>>,
) -> Result<(), NodeError> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    let provider = alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|_| NodeError::ConnectionFailed)?;

    let filter = alloy::rpc::types::eth::Filter::new()
        .address(pools)
        .event_signature(SYNC_TOPIC);

    let sub = provider
        .subscribe_logs(&filter)
        .await
        .map_err(|_| NodeError::ConnectionFailed)?;

    let mut stream = sub.into_stream();
    while let Some(log) = stream.next().await {
        let pool_address = log.address();
        let block_number = log.block_number.unwrap_or(0);
        let data = log.data().data.as_ref();
        let Some((reserve0, reserve1)) = decode_sync_reserves(data) else {
            continue;
        };

        {
            let mut store = store.lock().await;
            store.apply_sync(pool_address, reserve0, reserve1, block_number);
        }

        println!(
            "pool={} reserve0={} reserve1={} block={}",
            pool_address, reserve0, reserve1, block_number
        );
    }

    Err(NodeError::ConnectionFailed)
}

pub const SYNC_TOPIC: alloy::primitives::B256 =
    alloy::primitives::b256!("0x1c411e9a96e071241c2f21f7726b17ae89e3cab4c78be50e062b03a9fffbbad1");
