//! Launch note script.
//!
//! Note storage: `[faucet_prefix, faucet_suffix, recipient_prefix,
//! recipient_suffix, serial_0..3, tag]`.
//!
//! Asserts the executing account is the faucet named in storage, then calls
//! `launch`, which checks the note's sender against the faucet's owner.

#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(network_faucet::NetworkFaucet)]
pub struct NativeAccount;

#[note]
struct LaunchNote;

#[note]
impl LaunchNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(storage.len() == 9, "Invalid number of arguments in the note");

        let faucet = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&faucet),
            "Invalid faucet account to execute against"
        );

        let recipient = AccountId::new(storage[2], storage[3]);
        let serial_num = Word::new([storage[4], storage[5], storage[6], storage[7]]);
        account.launch(recipient, serial_num, storage[8]);
    }
}
