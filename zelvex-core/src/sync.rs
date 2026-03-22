use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use zelvex_types::Pool;

pub fn decode_sync_reserves(data: &[u8]) -> Option<(U256, U256)> {
    if data.len() != 64 {
        return None;
    }
    let r0 = U256::from_be_slice(&data[0..32]);
    let r1 = U256::from_be_slice(&data[32..64]);
    Some((r0, r1))
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
}
