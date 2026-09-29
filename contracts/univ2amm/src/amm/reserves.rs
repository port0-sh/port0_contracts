use miden::{Felt, Word, WordValue, export_type};

use crate::error::ConversionError;

#[export_type]
pub struct Reserves {
    pub reserve0: u64,
    pub reserve1: u64,
    pub lp_supply: u64,
}

impl Reserves {
    pub fn k(&self) -> u128 {
        self.reserve0 as u128 * self.reserve1 as u128
    }
    pub fn reserve0(&self) -> u64 {
        self.reserve0
    }
    pub fn reserve1(&self) -> u64 {
        self.reserve1
    }
    pub fn lp_supply(&self) -> u64 {
        self.lp_supply
    }
}

impl WordValue for Reserves {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(Word::from([
            Felt::try_from(self.reserve0).map_err(|_| ConversionError::U64ToFelt.to_string())?,
            Felt::try_from(self.reserve1).map_err(|_| ConversionError::U64ToFelt.to_string())?,
            Felt::try_from(self.lp_supply).map_err(|_| ConversionError::U64ToFelt.to_string())?,
            Felt::ZERO,
        ]))
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(Self {
            reserve0: word.a.as_canonical_u64(),
            reserve1: word.b.as_canonical_u64(),
            lp_supply: word.c.as_canonical_u64(),
        })
    }
}
