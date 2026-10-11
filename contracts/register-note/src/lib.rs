//! Register note script.
//!
//! Consumed by the pool registry account. Note storage, in order:
//!
//! ```text
//! [registry_prefix, registry_suffix,
//!  pool_prefix, pool_suffix,
//!  asset_0_prefix, asset_0_suffix,
//!  asset_1_prefix, asset_1_suffix]
//! ```
//!
//! The script asserts the executing account is the registry named in storage,
//! then calls the registry's `register` procedure. Registration verifies the
//! pool's code commitment by FPI and records the pool and its asset pair.

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
