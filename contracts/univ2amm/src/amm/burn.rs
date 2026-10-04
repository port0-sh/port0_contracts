use crate::{
    amm::{MINIMUM_LIQUIDITY, reserves::Reserves},
    error::PoolMathError,
    utils::math::MAX_AMOUNT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurnQuote {
    pub amount0: u64,
    pub amount1: u64,
    pub lp_supply_after: u64,
    pub reserve0_after: u64,
    pub reserve1_after: u64,
}

pub fn quote_burn(
    liquidity: u64,
    reserves: Reserves,
    lp_supply: u64,
) -> Result<BurnQuote, PoolMathError> {
    let Reserves { reserve0, reserve1 } = reserves;
    if liquidity == 0 {
        return Err(PoolMathError::ZeroAmountIn);
    }
    if reserve0 == 0 || reserve1 == 0 || lp_supply == 0 {
        return Err(PoolMathError::EmptyPool);
    }
    if liquidity > MAX_AMOUNT
        || reserve0 > MAX_AMOUNT
        || reserve1 > MAX_AMOUNT
        || lp_supply > MAX_AMOUNT
    {
        return Err(PoolMathError::Overflow);
    }
    if liquidity > lp_supply - MINIMUM_LIQUIDITY.min(lp_supply) {
        return Err(PoolMathError::InsufficientLiquidity);
    }
    let amount0 = (liquidity as u128 * reserve0 as u128) / lp_supply as u128;
    let amount1 = (liquidity as u128 * reserve1 as u128) / lp_supply as u128;
    if amount0 == 0 || amount1 == 0 {
        return Err(PoolMathError::ZeroAmountOut);
    }
    Ok(BurnQuote {
        amount0: amount0 as u64,
        amount1: amount1 as u64,
        lp_supply_after: lp_supply - liquidity,
        reserve0_after: reserve0 - amount0 as u64,
        reserve1_after: reserve1 - amount1 as u64,
    })
}
