use crate::{error::PoolMathError, utils::math::MAX_AMOUNT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapQuote {
    pub amount_out: u64,
    pub reserve_in_after: u64,
    pub reserve_out_after: u64,
}

pub fn amount_out(
    amount_in: u64,
    reserve_in: u64,
    reserve_out: u64,
    fee_numerator: u128,
    fee_denominator: u128,
) -> Result<SwapQuote, PoolMathError> {
    if amount_in == 0 {
        return Err(PoolMathError::ZeroAmountIn);
    }
    if reserve_in == 0 || reserve_out == 0 {
        return Err(PoolMathError::EmptyPool);
    }
    if amount_in > MAX_AMOUNT || reserve_in > MAX_AMOUNT || reserve_out > MAX_AMOUNT {
        return Err(PoolMathError::Overflow);
    }
    let amount_in_with_fee = amount_in as u128 * fee_numerator;
    let numerator = amount_in_with_fee
        .checked_mul(reserve_out as u128)
        .ok_or(PoolMathError::Overflow)?;
    let denominator = reserve_in as u128 * fee_denominator + amount_in_with_fee;
    let out = numerator / denominator;
    if out == 0 {
        return Err(PoolMathError::ZeroAmountOut);
    }
    if out >= reserve_out as u128 {
        return Err(PoolMathError::InsufficientLiquidity);
    }
    let (reserve_in_after, has_overflown_add) = reserve_in.overflowing_add(amount_in);
    let (reserve_out_after, has_overflown_sub) = reserve_out.overflowing_sub(out as u64);
    if has_overflown_add || has_overflown_sub || reserve_in_after >= MAX_AMOUNT {
        return Err(PoolMathError::Overflow);
    }

    Ok(SwapQuote {
        amount_out: out as u64,
        reserve_in_after,
        reserve_out_after,
    })
}
