use miden::{AccountId, Felt, Word, WordValue, export_type};

#[export_type]
pub struct Fees {
    pub fee_bps: u32,
    pub fee_to: AccountId,
}

impl Fees {
    pub fn fee_bps(&self) -> u32 {
        self.fee_bps
    }
    pub fn fee_to(&self) -> AccountId {
        self.fee_to
    }
    pub fn set_fee_bps(&mut self, fee_bps: u32) {
        self.fee_bps = fee_bps
    }
    pub fn set_fee_to(&mut self, fee_to: AccountId) {
        self.fee_to = fee_to
    }
}

impl WordValue for Fees {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(Word::from([
            Felt::from(self.fee_bps),
            Felt::from(self.fee_to.prefix),
            Felt::from(self.fee_to.suffix),
            Felt::ZERO,
        ]))
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(Self {
            fee_bps: word.a.as_canonical_u64() as u32,
            fee_to: AccountId::new(word.b, word.c),
        })
    }
}
