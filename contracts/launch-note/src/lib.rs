//! Launch note script.
//!
//! Consumed by a network faucet. Note storage, in order:
//!
//! ```text
//! [faucet_prefix, faucet_suffix, recipient_prefix, recipient_suffix,
//!  serial_0, serial_1, serial_2, serial_3, tag]
//! ```
//!
//! The script asserts the executing account is the faucet named in storage, then
//! calls the faucet's `launch`, which is gated on the faucet's owner and on the
//! token not having been launched yet. The note's own sender is what the faucet
//! checks, so a launch note emitted by anyone but the owner aborts.

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
