use crate::error::PoolMathError;

/// Upper bound on every reserve, amount, and `lp_supply` in the pool.
///
/// Chosen so that every `u128` intermediate in `quote_mint`, `quote_burn`, and `amount_out`
/// stays below `u128::MAX`. The binding constraint is the swap numerator
/// `amount_in_with_fee * reserve_out`, where `amount_in_with_fee <= amount_in * 10_000` (the
/// basis-point fee scale), giving a worst case of `MAX_AMOUNT * 10_000 * MAX_AMOUNT`. For that
/// to fit in a `u128` we need `MAX_AMOUNT < 2^64 / 100 ~= 1.84e17`; `2^57` is the largest power
/// of two below it, leaving ~28% safety margin.
///
/// Once a 256-bit multiply/divide is available, this can be raised back toward
/// `(1 << 63) - (1 << 31)`.
pub const MAX_AMOUNT: u64 = 1 << 57;

/// Floor square root. The largest square that fits in `u128` is `(2^64 - 1)^2`,
/// so the result always fits in a `u64`.
pub fn isqrt(mut n: u128) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut bit = 1u128 << ((127 - n.leading_zeros()) & !1);
    let mut root = 0u128;
    while bit != 0 {
        let trial = root + bit;
        root >>= 1;
        if n >= trial {
            n -= trial;
            root += bit;
        }
        bit >>= 2;
    }
    root as u64
}

pub fn add_amount(left: u64, right: u64) -> Result<u64, PoolMathError> {
    let sum = left.checked_add(right).ok_or(PoolMathError::Overflow)?;
    if sum > MAX_AMOUNT {
        return Err(PoolMathError::Overflow);
    }
    Ok(sum)
}
