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
        if active_note::get_sender().eq(&active_account::get_id()) {
            for asset in active_note::get_initial_assets() {
                native_account::add_asset(asset);
            }
        }
        let storage = active_note::get_storage();
        let pool_id = AccountId::new(storage[0], storage[1]);
        assert!(
            active_account::get_id().eq(&pool_id),
            "Prevented execution on an unauthorized account"
        );
        let recipient = AccountId::new(storage[2], storage[3]);
        let serial_num = Word::from([storage[4], storage[5], storage[6], storage[7]]);
        let tag = storage[8];
        let note_type = storage[9];
        let min_amount_out = storage[10];
        account.swap(
            recipient,
            serial_num,
            tag,
            note_type,
            min_amount_out.as_canonical_u64(),
        );
    }
}
