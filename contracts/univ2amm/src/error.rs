#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolMathError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionError {
    U64ToFelt,
}

impl ConversionError {
    pub fn to_string(&self) -> &'static str {
        match self {
            Self::U64ToFelt => "Conversion error U64 -> Felt",
        }
    }
}
