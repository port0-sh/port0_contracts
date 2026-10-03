use miden::{AccountId, Felt, Word, WordValue, export_type};

#[export_type]
pub struct Fees {
    pub lp_fee_bps: u32,
    pub protocol_fee_bps: u32,
    pub protocol_fee_to: AccountId,
}

impl Fees {
    pub fn lp_fee_bps(&self) -> u32 {
        self.lp_fee_bps
    }
    pub fn protocol_fee_bps(&self) -> u32 {
        self.protocol_fee_bps
    }
    pub fn protocol_fee_to(&self) -> AccountId {
        self.protocol_fee_to
    }
    pub fn set_protocol_fee_to(&mut self, fee_to: AccountId) {
        self.protocol_fee_to = fee_to
    }
    pub fn set_lp_fee_bps(&mut self, fee_bps: u32) {
        assert!(
            self.protocol_fee_bps + fee_bps < 10000,
            "Total fee bps must be less than 10000"
        );
        self.lp_fee_bps = fee_bps
    }
    pub fn set_protocol_fee_bps(&mut self, fee_bps: u32) {
        assert!(
            self.lp_fee_bps + fee_bps < 10000,
            "Total fee bps must be less than 10000"
        );
        self.protocol_fee_bps = fee_bps
    }
}

impl WordValue for Fees {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(Word::from([
            Felt::from(self.lp_fee_bps),
            Felt::from(self.protocol_fee_bps),
            Felt::from(self.protocol_fee_to.prefix),
            Felt::from(self.protocol_fee_to.suffix),
        ]))
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(Self {
            lp_fee_bps: word.a.as_canonical_u64() as u32,
            protocol_fee_bps: word.b.as_canonical_u64() as u32,
            protocol_fee_to: AccountId::new(word.c, word.d),
        })
    }
}

#[export_type]
pub struct ProtocolPot {
    pub amount0: u64,
    pub amount1: u64,
}

impl ProtocolPot {
    pub fn amount0(&self) -> u64 {
        self.amount0
    }
    pub fn amount1(&self) -> u64 {
        self.amount1
    }
    pub fn add_to_pot(&mut self, amount0: u64, amount1: u64) {
        self.amount0 = self.amount0.checked_add(amount0).unwrap();
        self.amount1 = self.amount1.checked_add(amount1).unwrap();
    }
    pub fn resetAmount0(&mut self) {
        self.amount0 = 0;
    }
    pub fn resetAmount1(&mut self) {
        self.amount1 = 0;
    }
}

impl WordValue for ProtocolPot {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(Word::from([
            Felt::new_unchecked(self.amount0),
            Felt::new_unchecked(self.amount1),
            Felt::ZERO,
            Felt::ZERO,
        ]))
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(Self {
            amount0: word.a.as_canonical_u64(),
            amount1: word.b.as_canonical_u64(),
        })
    }
}
