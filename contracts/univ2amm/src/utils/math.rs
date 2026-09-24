use crate::error::PoolMathError;

pub const MAX_AMOUNT: u64 = (1 << 63) - (1 << 31);

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
