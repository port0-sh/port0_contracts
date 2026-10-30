#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

#[account(port0::Port0)]
pub struct NativeAccount;

#[note]
struct SwapNoteNote;

#[note]
impl SwapNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(
            storage.len() == 15,
            "Invalid number of arguments in the note"
        );
        let pool_id = AccountId::new(storage[0], storage[1]);
        if active_account::get_id().eq(&pool_id) {
            self.run_swap(account, &storage)
        } else {
            self.reclaim();
        }
    }

    fn reclaim(self) {
        // A spot swap never needs this branch: the pool consumes the note and
        // refunds a trade that cannot fill. It remains for notes that never
        // reach the pool, and it is implemented once the SDK can call the
        // standards wallet's `receive_asset`.
        return;
    }

    fn run_swap(self, account: &mut NativeAccount, storage: &[Felt]) {
        let pool_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&pool_id),
            "Invalid pool account to execute against"
        );
        let recipient = AccountId::new(storage[2], storage[3]);
        let serial_num = Word::from([storage[4], storage[5], storage[6], storage[7]]);
        let tag = storage[8];
        let deadline = storage[9].as_canonical_u64();
        let min_amount_out = storage[10].as_canonical_u64();
        let refund_tag = storage[11];
        account.swap(recipient, serial_num, tag, deadline, min_amount_out, refund_tag);
    }
}
