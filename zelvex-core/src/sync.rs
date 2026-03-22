use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use zelvex_types::Pool;

pub const SYNC_TOPIC: alloy::primitives::B256 =
    alloy::primitives::b256!("0x1c411e9a96e071241c2f21f7726b17ae89e3cab4c78be50e062b03a9fffbbad1");

pub fn decode_sync_reserves(data: &[u8]) -> Option<(U256, U256)> {
    if data.len() != 64 {
        return None;
    }
    let r0 = U256::from_be_slice(&data[0..32]);
    let r1 = U256::from_be_slice(&data[32..64]);
    Some((r0, r1))
}

pub fn decode_sync_log(log: &alloy::rpc::types::eth::Log) -> Option<(Address, U256, U256, u64)> {
    let topic0 = log.topic0()?;
    if *topic0 != SYNC_TOPIC {
        return None;
    }

    let block_number = log.block_number.unwrap_or(0);
    let data = log.data().data.as_ref();
    let (reserve0, reserve1) = decode_sync_reserves(data)?;

    Some((log.address(), reserve0, reserve1, block_number))
}

#[derive(Debug, Default)]
pub struct PoolStore {
    pools: HashMap<Address, Pool>,
}

impl PoolStore {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }

    pub fn upsert_pool(&mut self, pool: Pool) {
        self.pools.insert(pool.pool_address, pool);
    }

    pub fn apply_sync(
        &mut self,
        pool_address: Address,
        reserve0: U256,
        reserve1: U256,
        block: u64,
    ) {
        if let Some(pool) = self.pools.get_mut(&pool_address) {
            pool.reserve0 = reserve0;
            pool.reserve1 = reserve1;
            pool.block_updated = block;
        }
    }

    pub fn get(&self, pool_address: &Address) -> Option<&Pool> {
        self.pools.get(pool_address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_sync_reserves_decodes_two_words() {
        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(1u32).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(2u32).to_be_bytes::<32>());
        assert_eq!(
            decode_sync_reserves(&data),
            Some((U256::from(1u32), U256::from(2u32)))
        );
    }

    #[test]
    fn decode_sync_log_decodes_address_reserves_and_block() {
        let pool = alloy::primitives::address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");

        let mut data = [0u8; 64];
        data[0..32].copy_from_slice(&U256::from(123u32).to_be_bytes::<32>());
        data[32..64].copy_from_slice(&U256::from(456u32).to_be_bytes::<32>());

        let log = alloy::rpc::types::eth::Log {
            inner: alloy::primitives::Log::new_unchecked(
                pool,
                vec![SYNC_TOPIC],
                alloy::primitives::Bytes::from(data.to_vec()),
            ),
            block_number: Some(21847393),
            ..Default::default()
        };

        assert_eq!(
            decode_sync_log(&log),
            Some((pool, U256::from(123u32), U256::from(456u32), 21847393))
        );
    }
}
