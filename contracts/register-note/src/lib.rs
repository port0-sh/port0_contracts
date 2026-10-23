//! Register note script.
//!
//! Note storage: `[registry_prefix, registry_suffix, pool_prefix, pool_suffix,
//! asset0_prefix, asset0_suffix, asset1_prefix, asset1_suffix]`.
//!
//! Asserts the executing account is the registry named in storage, then calls
//! `register`, which verifies the pool on chain before listing it.

#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(pool_registry::PoolRegistry)]
pub struct NativeAccount;

#[note]
struct RegisterNote;

#[note]
impl RegisterNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(
            storage.len() == 8,
            "Invalid number of arguments in the note"
        );

        let registry_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&registry_id),
            "Invalid registry account to execute against"
        );

        let pool_id = AccountId::new(storage[2], storage[3]);
        let asset_0 = AccountId::new(storage[4], storage[5]);
        let asset_1 = AccountId::new(storage[6], storage[7]);

        account.register(pool_id, asset_0, asset_1);
    }
}
