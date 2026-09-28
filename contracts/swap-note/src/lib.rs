#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(univ2amm::Univ2amm)]
pub struct NativeAccount;

#[note]
struct SwapNoteNote;

#[note]
impl SwapNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        let min_amount_out = storage[0];
        let recipient = AccountId::new(storage[1], storage[2]);
        let serial_num = Word::from([storage[3], storage[4], storage[5], storage[6]]);
        let tag = storage[7];
        let note_type = storage[8];
        account.swap(
            min_amount_out.as_canonical_u64(),
            recipient,
            serial_num,
            tag,
            note_type,
        );
    }
}
