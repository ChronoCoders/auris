use alloy::{
    primitives::{Address, Bytes, U256},
    sol,
    sol_types::SolCall,
};

sol! {
    struct ArbParams {
        address tokenIn;
        address tokenOut;
        address poolA;
        address poolB;
        uint256 amountIn;
        uint256 minProfit;
    }

    function executeArb(ArbParams params) external;
}

/// Encode the `executeArb(ArbParams)` calldata.
///
/// Returns the full ABI-encoded calldata including the 4-byte function selector.
pub fn build_execute_arb_calldata(
    token_in: Address,
    token_out: Address,
    pool_a: Address,
    pool_b: Address,
    amount_in: U256,
    min_profit: U256,
) -> Bytes {
    let call = executeArbCall {
        params: ArbParams {
            tokenIn: token_in,
            tokenOut: token_out,
            poolA: pool_a,
            poolB: pool_b,
            amountIn: amount_in,
            minProfit: min_profit,
        },
    };
    Bytes::from(call.abi_encode())
}

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{address, U256},
        sol_types::SolCall,
    };

    use super::*;

    fn sample_calldata() -> Bytes {
        build_execute_arb_calldata(
            address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
            address!("0x397FF1542f962076d0BFE58eA045FfA2d347ACa0"),
            U256::from(1_000_000_000_000_000_000u128),
            U256::from(1_000_000u64),
        )
    }

    #[test]
    fn function_selector_is_correct() {
        let cd = sample_calldata();
        let expected = executeArbCall::SELECTOR;
        assert_eq!(
            &cd[..4],
            &expected,
            "first 4 bytes must equal the executeArb function selector"
        );
    }

    #[test]
    fn calldata_length_is_correct() {
        let cd = sample_calldata();
        // 4 bytes selector + 6 * 32 bytes params = 196 bytes
        assert_eq!(cd.len(), 196, "calldata must be exactly 196 bytes");
    }

    #[test]
    fn calldata_encodes_and_decodes_roundtrip() {
        let token_in = address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
        let token_out = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
        let pool_a = address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");
        let pool_b = address!("0x397FF1542f962076d0BFE58eA045FfA2d347ACa0");
        let amount_in = U256::from(1_000_000_000_000_000_000u128);
        let min_profit = U256::from(1_000_000u64);

        let cd = build_execute_arb_calldata(
            token_in, token_out, pool_a, pool_b, amount_in, min_profit,
        );

        let decoded =
            executeArbCall::abi_decode(&cd, true).expect("must decode successfully");

        assert_eq!(decoded.params.tokenIn, token_in);
        assert_eq!(decoded.params.tokenOut, token_out);
        assert_eq!(decoded.params.poolA, pool_a);
        assert_eq!(decoded.params.poolB, pool_b);
        assert_eq!(decoded.params.amountIn, amount_in);
        assert_eq!(decoded.params.minProfit, min_profit);
    }
}
