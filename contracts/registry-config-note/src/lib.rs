//! Registry config note script.
//!
//! Consumed by the pool registry. Note storage, in order:
//!
//! ```text
//! [registry_prefix, registry_suffix, new_register_fee, script_root (4 felts)]
//! ```
//!
//! The script asserts the executing account is the registry named in storage,
//! then calls the registry's `set_register_fee`. The registry checks the note's
//! sender against the admin it was created with, so this note can only change
//! the price when the admin emits it.

#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(pool_registry::PoolRegistry)]
pub struct NativeAccount;

#[note]
struct RegistryConfigNote;

#[note]
impl RegistryConfigNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(storage.len() == 7, "Invalid number of arguments in the note");

        let registry_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&registry_id),
            "Invalid registry account to execute against"
        );

        let script_root = Word::new([storage[3], storage[4], storage[5], storage[6]]);
        account.set_register_fee(storage[2].as_canonical_u64(), script_root);
    }
}
