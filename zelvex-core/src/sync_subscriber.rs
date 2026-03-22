use std::{sync::Arc, time::Duration};

use alloy::primitives::Address;
use alloy::providers::Provider;
use futures_util::StreamExt;
use tokio::sync::{mpsc, Mutex};

use crate::node::NodeError;
use crate::sync::{decode_sync_log, PoolStore, SYNC_TOPIC};
use zelvex_types::Pool;

pub async fn subscribe_sync_events(
    ws_url: &str,
    pools: Vec<Address>,
    store: Arc<Mutex<PoolStore>>,
    updates: mpsc::Sender<()>,
) -> Result<(), NodeError> {
    let ws = alloy::transports::ws::WsConnect::new(ws_url);
    let provider = alloy::providers::ProviderBuilder::new()
        .on_ws(ws)
        .await
        .map_err(|e| NodeError::ConnectionFailed(e.to_string()))?;

    let filter = alloy::rpc::types::eth::Filter::new()
        .address(pools)
        .event_signature(SYNC_TOPIC);

    let sub = provider
        .subscribe_logs(&filter)
        .await
        .map_err(|e| NodeError::ConnectionFailed(e.to_string()))?;

    let mut stream = sub.into_stream();
    while let Some(log) = stream.next().await {
        let Some((pool_address, reserve0, reserve1, block_number)) = decode_sync_log(&log) else {
            continue;
        };

        {
            let mut store = store.lock().await;
            store.apply_sync(pool_address, reserve0, reserve1, block_number);
        }

        let _ = updates.try_send(());

        println!(
            "pool={} reserve0={} reserve1={} block={}",
            pool_address, reserve0, reserve1, block_number
        );
    }

    Err(NodeError::ConnectionFailed(
        "subscription ended".to_string(),
    ))
}

pub fn default_test_pools() -> [Address; 10] {
    [
        alloy::primitives::address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
        alloy::primitives::address!("0xA478c2975Ab1Ea89e8196811F51A7B7Ade33eB11"),
        alloy::primitives::address!("0x397FF1542f962076d0BFE58eA045FfA2d347ACa0"),
        alloy::primitives::address!("0xC3D03e4F041Fd4cD388c549Ee2A29a9E5075882f"),
        alloy::primitives::address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
        alloy::primitives::address!("0xA478c2975Ab1Ea89e8196811F51A7B7Ade33eB11"),
        alloy::primitives::address!("0x397FF1542f962076d0BFE58eA045FfA2d347ACa0"),
        alloy::primitives::address!("0xC3D03e4F041Fd4cD388c549Ee2A29a9E5075882f"),
        alloy::primitives::address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
        alloy::primitives::address!("0xA478c2975Ab1Ea89e8196811F51A7B7Ade33eB11"),
    ]
}

pub fn default_test_pool_metadata() -> Vec<Pool> {
    let weth = alloy::primitives::address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let usdc = alloy::primitives::address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    let dai = alloy::primitives::address!("0x6B175474E89094C44Da98b954EedeAC495271d0F");

    default_test_pools()
        .into_iter()
        .map(|pool_address| {
            let (token0, token1) = match pool_address {
                a if a
                    == alloy::primitives::address!(
                        "0xA478c2975Ab1Ea89e8196811F51A7B7Ade33eB11"
                    ) =>
                {
                    (dai, weth)
                }
                a if a
                    == alloy::primitives::address!(
                        "0xC3D03e4F041Fd4cD388c549Ee2A29a9E5075882f"
                    ) =>
                {
                    (dai, weth)
                }
                _ => (usdc, weth),
            };

            Pool {
                pool_address,
                token0,
                token1,
                reserve0: alloy::primitives::U256::ZERO,
                reserve1: alloy::primitives::U256::ZERO,
                block_updated: 0,
            }
        })
        .collect()
}

pub async fn run_sync_subscription(
    ws_url: &str,
    pools: Vec<Address>,
    store: Arc<Mutex<PoolStore>>,
    updates: mpsc::Sender<()>,
) -> Result<(), NodeError> {
    let mut attempts = 0u32;
    let mut backoff = Duration::from_secs(1);
    loop {
        match subscribe_sync_events(ws_url, pools.clone(), store.clone(), updates.clone()).await {
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
