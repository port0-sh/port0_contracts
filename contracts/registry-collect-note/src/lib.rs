//! Registry collect note script.
//!
//! Consumed by the pool registry. Note storage, in order:
//!
//! ```text
//! [registry_prefix, registry_suffix,
//!  asset_key_0, asset_key_1, asset_key_2, asset_key_3,
//!  serial_0, serial_1, serial_2, serial_3]
//! ```
//!
//! The script asserts the executing account is the registry named in storage,
//! then calls the registry's `collect`. Collection is permissionless, like a
//! pool's: the destination is the registry's configured protocol account, so a
//! caller can only flush accumulated register fees toward the protocol.

#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

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
            storage.len() == 10,
            "Invalid number of arguments in the note"
        );

        let registry_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&registry_id),
            "Invalid registry account to execute against"
        );

        let asset_key = Word::new([storage[2], storage[3], storage[4], storage[5]]);
        let serial_num = Word::new([storage[6], storage[7], storage[8], storage[9]]);
        account.collect(asset_key, serial_num)
    }
}
