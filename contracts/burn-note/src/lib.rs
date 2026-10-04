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
        if active_note::get_sender().eq(&active_account::get_id()) {
            self.reclaim();
        } else {
            self.run_burn(account)
        }
    }

    fn reclaim(self) {
        for asset in active_note::get_initial_assets() {
            todo!("How to actually call wallet?");
        }
    }

    fn run_burn(self, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        assert!(
            storage.len() == 12,
            "Invalid number of arguments in the note"
        );
        let pool_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&pool_id),
            "Prevented execution on an unauthorized account"
        );
        let recipient = AccountId::new(storage[2], storage[3]);
        let serial_num = Word::from([storage[4], storage[5], storage[6], storage[7]]);
        let tag = storage[8];
        let deadline = storage[9];
        assert!(
            deadline.as_canonical_u64() > tx::get_block_timestamp() as u64,
            "Note is past its deadline",
        );
        let min_amount0 = storage[10];
        let min_amount1 = storage[11];
        account.burn(
            recipient,
            serial_num,
            tag,
            min_amount0.as_canonical_u64(),
            min_amount1.as_canonical_u64(),
        );
    }
}
