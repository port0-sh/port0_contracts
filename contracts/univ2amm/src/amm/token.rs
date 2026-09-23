use miden::{AccountId, Word, WordValue, export_type};

#[export_type]
pub struct Tokens {
    token0: AccountId,
    token1: AccountId,
}

impl Tokens {
    pub fn token0(&self) -> AccountId {
        self.token0
    }
    pub fn token1(&self) -> AccountId {
        self.token1
    }
}

impl WordValue for Tokens {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(Word::from([
            self.token0.prefix,
            self.token0.suffix,
            self.token1.prefix,
            self.token1.suffix,
        ]))
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(Self {
            token0: AccountId::new(word.a, word.b),
            token1: AccountId::new(word.c, word.d),
        })
    }
}
