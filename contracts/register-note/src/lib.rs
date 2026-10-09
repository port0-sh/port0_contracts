#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(registry::Registry)]
pub struct NativeAccount;

#[note]
struct RegisterNote;

#[note]
impl RegisterNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        if active_note::get_sender().eq(&active_account::get_id()) {
            self.reclaim();
        } else {
            self.run_mint(account)
        }
    }
}
