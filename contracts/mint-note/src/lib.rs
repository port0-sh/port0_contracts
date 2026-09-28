#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(univ2amm::Univ2amm)]
pub struct NativeAccount;

#[note]
struct MintNoteNote;

#[note]
impl MintNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        let recipient = AccountId::new(storage[0], storage[1]);
        let serial_num = Word::from([storage[2], storage[3], storage[4], storage[5]]);
        let tag = storage[6];
        let note_type = storage[7];
        let min_lp_out = storage[8];
        account.mint(
            min_lp_out.as_canonical_u64(),
            recipient,
            serial_num,
            tag,
            note_type,
        );
    }
}
