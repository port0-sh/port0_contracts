use miden::{AccountId, Felt};

pub fn account_target_tag(id: AccountId) -> Felt {
    const TAG_LEN: u32 = 14;
    let high = (id.prefix.as_canonical_u64() >> 32) as u32;
    Felt::from(high & (u32::MAX << (32 - TAG_LEN)))
}
