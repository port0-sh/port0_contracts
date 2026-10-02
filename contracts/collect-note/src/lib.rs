#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(univ2amm::Univ2amm)]
pub struct NativeAccount;

#[note]
struct CollectNoteNote;

#[note]
impl CollectNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        let serial_num = Word::from([storage[0], storage[1], storage[2], storage[3]]);
        account.collect(serial_num)
    }
}
