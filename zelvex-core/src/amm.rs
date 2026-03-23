use alloy::primitives::U256;

pub fn get_amount_out(amount_in: U256, reserve_in: U256, reserve_out: U256) -> Option<U256> {
    if reserve_in.is_zero() || reserve_out.is_zero() {
        return None;
    }
    let amount_in_with_fee = amount_in * U256::from(997u32);
    let numerator = amount_in_with_fee * reserve_out;
    let denominator = reserve_in * U256::from(1000u32) + amount_in_with_fee;
    Some(numerator / denominator)
}

pub fn get_spot_price(reserve_in: U256, reserve_out: U256) -> Option<U256> {
    if reserve_in.is_zero() {
        return None;
    }
    let scale = u256_pow10(18);
    Some((reserve_out * scale) / reserve_in)
}

pub fn calculate_profit(
    amount_in: U256,
    reserve_a_in: U256,
    reserve_a_out: U256,
    reserve_b_in: U256,
    reserve_b_out: U256,
) -> Option<U256> {
    let mid_amount = get_amount_out(amount_in, reserve_a_in, reserve_a_out)?;
    let final_out = get_amount_out(mid_amount, reserve_b_in, reserve_b_out)?;
    if final_out > amount_in {
        Some(final_out - amount_in)
    } else {
        None
    }
}

fn u256_pow10(exp: u32) -> U256 {
    let mut value = U256::from(1u32);
    for _ in 0..exp {
        value *= U256::from(10u32);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_amount_out_zero_amount_returns_zero() {
        let out = get_amount_out(U256::ZERO, U256::from(1000u32), U256::from(1000u32));
        assert_eq!(out, Some(U256::ZERO));
    }

    #[test]
    fn get_amount_out_zero_reserve_returns_none() {
        let result = get_amount_out(U256::from(1u32), U256::ZERO, U256::from(1u32));
        assert_eq!(result, None);
    }

    #[test]
    fn calculate_profit_equal_prices_returns_none() {
        let amount_in = U256::from(1_000u32);
        let reserve_a_in = U256::from(10_000u32);
        let reserve_a_out = U256::from(10_000u32);
        let reserve_b_in = U256::from(10_000u32);
        let reserve_b_out = U256::from(10_000u32);
        let profit = calculate_profit(
            amount_in,
            reserve_a_in,
            reserve_a_out,
            reserve_b_in,
            reserve_b_out,
        );
        assert!(profit.is_none());
    }

    #[test]
    fn get_spot_price_scaled_to_18_decimals() {
        let reserve_in = U256::from(2u32);
        let reserve_out = U256::from(3u32);
        let price = get_spot_price(reserve_in, reserve_out);
        assert_eq!(price, Some(U256::from(1_500_000_000_000_000_000u128)));
    }

    #[test]
    fn calculate_profit_when_pool_a_cheaper_is_positive() {
        let amount_in = U256::from(100u32);

        let reserve_a_in = U256::from(10_000u32);
        let reserve_a_out = U256::from(20_000u32);

        let reserve_b_in = U256::from(10_000u32);
        let reserve_b_out = U256::from(25_000u32);

        let profit = calculate_profit(
            amount_in,
            reserve_a_in,
            reserve_a_out,
            reserve_b_in,
            reserve_b_out,
        );
        match profit {
            Some(p) => assert!(p > U256::ZERO),
            None => unreachable!(),
        }
    }
}
