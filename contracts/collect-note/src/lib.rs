//! Pool collect note script.
//!
//! Note storage: `[pool_prefix, pool_suffix, serial_0..3, asset_key_0..n]`, four
//! felts per key, at most four keys.
//!
//! Asserts the executing account is the pool named in storage, then calls
//! `collect` once per key. Collection is permissionless: the destination is the
//! registry's configured protocol account.

#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

// The key cap lives in `contracts/collect-fee-hold.rs` with the holds it is
// coupled to, so the tooling and the scripts cannot disagree.
include!("../../collect-fee-hold.rs");

const MAX_COLLECT_KEYS: usize = COLLECT_MAX_KEYS;

#[account(port0::Port0)]
pub struct NativeAccount;

#[note]
struct CollectNoteNote;

#[note]
impl CollectNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(
            storage.len() >= 6 && (storage.len() - 6) % 4 == 0,
            "Invalid number of arguments in the note"
        );
        assert!(
            (storage.len() - 6) / 4 <= MAX_COLLECT_KEYS,
            "Too many asset keys in the note"
        );
        let pool_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&pool_id),
            "Invalid pool account to execute against"
        );
        let serial_num = Word::new([storage[2], storage[3], storage[4], storage[5]]);
        let mut offset = 6;
        while offset + 4 <= storage.len() {
            let asset_key = Word::new([
                storage[offset],
                storage[offset + 1],
                storage[offset + 2],
                storage[offset + 3],
            ]);
            account.collect(asset_key, serial_num);
            offset += 4;
        }
    }
}
