use crate::{
    amm::{MINIMUM_LIQUIDITY, reserves::Reserves},
    error::PoolMathError,
    utils::math::{MAX_AMOUNT, add_amount, isqrt},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MintQuote {
    pub liquidity_to_user: u64,
    pub lp_supply_after: u64,
    pub reserve0_after: u64,
    pub reserve1_after: u64,
}

pub fn quote_mint(
    amount0: u64,
    amount1: u64,
    reserves: Reserves,
) -> Result<MintQuote, PoolMathError> {
    let lp_supply = reserves.lp_supply();
    if amount0 == 0 || amount1 == 0 {
        return Err(PoolMathError::ZeroAmountIn);
    }
    if amount0 > MAX_AMOUNT || amount1 > MAX_AMOUNT || lp_supply > MAX_AMOUNT {
        return Err(PoolMathError::Overflow);
    }
    let reserve0 = reserves.reserve0();
    let reserve1 = reserves.reserve1();
    let reserve0_after = add_amount(reserve0, amount0)?;
    let reserve1_after = add_amount(reserve1, amount1)?;
    let (liquidity_to_user, lp_supply_after) = if lp_supply == 0 {
        if reserve0 != 0 || reserve1 != 0 {
            return Err(PoolMathError::Invariant);
        }
        let root = isqrt(reserves.k());
        if root <= MINIMUM_LIQUIDITY {
            return Err(PoolMathError::InsufficientLiquidity);
        }
        (root - MINIMUM_LIQUIDITY, root)
    } else {
        if reserve0 == 0 || reserve1 == 0 {
            return Err(PoolMathError::Invariant);
        }
        let from0 = (amount0 as u128 * lp_supply as u128) / reserve0 as u128;
        let from1 = (amount1 as u128 * lp_supply as u128) / reserve1 as u128;
        let liquidity = from0.min(from1);
        if liquidity == 0 {
            return Err(PoolMathError::InsufficientLiquidity);
        }
        let lp_supply_after = add_amount(lp_supply, liquidity as u64)?;
        (liquidity as u64, lp_supply_after)
    };
    Ok(MintQuote {
        liquidity_to_user,
        lp_supply_after,
        reserve0_after,
        reserve1_after,
    })
}
