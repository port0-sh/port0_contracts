use miden::{AccountId, Word, WordValue};

use crate::utils::u128::{u128_to_word, word_to_u128};

pub struct KLast {
    k_last: u128,
}

impl WordValue for KLast {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(u128_to_word(self.k_last))
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(word_to_u128(word))
    }
}
