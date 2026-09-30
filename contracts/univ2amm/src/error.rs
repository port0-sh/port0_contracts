#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolMathError {
    EmptyPool,
    Overflow,
    ZeroAmountIn,
    ZeroAmountOut,
    InsufficientLiquidity,
    Invariant,
}

impl PoolMathError {
    pub fn to_string(&self) -> &'static str {
        match self {
            Self::EmptyPool => "Pool is empty on either or both sides",
            Self::Overflow => "Values would overflow Felt::MAX",
            Self::ZeroAmountIn => "Amount in is zero",
            Self::ZeroAmountOut => "Amount out is zero",
            Self::InsufficientLiquidity => "Not enough liquidity in the pool for the operation",
            Self::Invariant => "Stored reserves and LP supply mismatch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionError {
    U64ToFelt,
    AccountIdToAssetKey,
    AssetKeyToAccountId,
}

impl ConversionError {
    pub fn to_string(&self) -> &'static str {
        match self {
            Self::U64ToFelt => "Conversion error: U64 -> Felt",
            Self::AccountIdToAssetKey => "Conversion error: Asset Key -> AccountId",
            Self::AssetKeyToAccountId => "Conversion error: AccountId -> Asset Key",
        }
    }
}
