#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(miden_xyk::MidenXyk)]
pub struct NativeAccount;

#[note]
struct AdminNote;

#[note]
impl AdminNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(
            storage.len() == 14,
            "Invalid number of arguments in the note"
        );
        let pool_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&pool_id),
            "Invalid pool account to execute against"
        );
        let protocol_fee_to = AccountId::new(storage[2], storage[3]);
        let lp_fee = storage[4].as_canonical_u64();
        let protocol_fee = storage[5].as_canonical_u64();
        let p2id_root = Word::new([storage[6], storage[7], storage[8], storage[9]]);
        let tokenlp_asset_word = Word::new([storage[10], storage[11], storage[12], storage[13]]);
        account.configure(
            lp_fee,
            protocol_fee,
            protocol_fee_to,
            p2id_root,
            tokenlp_asset_word,
        );
    }
}
