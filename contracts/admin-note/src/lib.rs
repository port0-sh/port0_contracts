#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(univ2amm::Univ2amm)]
pub struct NativeAccount;

#[note]
struct AdminNoteNote;

#[note]
impl AdminNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(
            storage.len() == 10,
            "Invalid number of arguments in the note"
        );
        let pool_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&pool_id),
            "Prevented execution on an unauthorized account"
        );
        let protocol_fee_to = AccountId::new(storage[2], storage[3]);
        let lp_fee = storage[4];
        let protocol_fee = storage[5];
        let p2id_root = Word::new([storage[6], storage[7], storage[8], storage[9]]);
        account.configure(lp_fee, protocol_fee, protocol_fee_to, p2id_root);
    }
}
