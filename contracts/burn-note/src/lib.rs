#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(univ2amm::Univ2amm)]
pub struct NativeAccount;

#[note]
struct BurnNoteNote;

#[note]
impl BurnNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        let recipient = AccountId::new(storage[0], storage[1]);
        let serial_num = Word::from([storage[2], storage[3], storage[4], storage[5]]);
        let tag = storage[6];
        let note_type = storage[7];
        let min_amount0 = storage[8];
        let min_amount1 = storage[9];
        account.burn(
            min_amount0.as_canonical_u64(),
            min_amount1.as_canonical_u64(),
            recipient,
            serial_num,
            tag,
            note_type,
        );
    }
}
