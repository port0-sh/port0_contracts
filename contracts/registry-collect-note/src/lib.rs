//! Registry collect note script.
//!
//! Consumed by the pool registry. Note storage, in order:
//!
//! ```text
//! [registry_prefix, registry_suffix,
//!  serial_0, serial_1, serial_2, serial_3,
//!  asset_key_0 (4 felts), asset_key_1 (4 felts), ...]
//! ```
//!
//! The script asserts the executing account is the registry named in storage, then calls the
//! registry's `collect` once per asset key. Collection is permissionless, like a pool's: the
//! destination is the registry's configured protocol account, so a caller can only flush
//! accumulated value toward it.

#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

// The key cap lives in `contracts/collect-fee-hold.rs` with the holds it is
// coupled to, so the tooling and the scripts cannot disagree.
include!("../../collect-fee-hold.rs");

const MAX_COLLECT_KEYS: usize = COLLECT_MAX_KEYS;

#[account(pool_registry::PoolRegistry)]
pub struct NativeAccount;

#[note]
struct RegistryCollectNote;

#[note]
impl RegistryCollectNote {
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

        let registry_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&registry_id),
            "Invalid registry account to execute against"
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
