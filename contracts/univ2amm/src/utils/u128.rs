use miden::{Felt, Word, WordValue, export_type};

#[export_type]
pub struct U128 {
    inner: Word,
}

impl U128 {
    pub fn new(val: u128) -> Self {
        Self {
            inner: u128_to_word(val),
        }
    }
    pub fn as_u128(&self) -> u128 {
        word_to_u128(self.inner)
    }
    pub fn as_word(&self) -> Word {
        self.inner
    }
}

pub fn word_to_u128(w: Word) -> u128 {
    (w.a.as_canonical_u64() as u128)
        | ((w.b.as_canonical_u64() as u128) << 32)
        | ((w.c.as_canonical_u64() as u128) << 64)
        | ((w.d.as_canonical_u64() as u128) << 96)
}
pub fn u128_to_word(v: u128) -> Word {
    Word::new([
        Felt::try_from((v & 0xffff_ffff) as u64).unwrap(),
        Felt::try_from(((v >> 32) & 0xffff_ffff) as u64).unwrap(),
        Felt::try_from(((v >> 64) & 0xffff_ffff) as u64).unwrap(),
        Felt::try_from((v >> 96) as u64).unwrap(),
    ])
}

impl WordValue for U128 {
    fn try_into_word(self) -> Result<Word, &'static str> {
        Ok(self.inner)
    }
    fn try_from_word(word: Word) -> Result<Self, &'static str> {
        Ok(Self { inner: word })
    }
}
