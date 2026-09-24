// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

use miden::*;

// Native account the note runs against. The `#[account(...)]` attribute exposes
// the `add-contract` account component's methods on this wrapper (the path is
// `<package>::<Interface>` with `-` replaced by `_`).
#[account(univ2amm::Univ2amm)]
pub struct NativeAccount;

#[note]
struct MintNoteNote;

#[note]
impl MintNoteNote {
    #[note_script]
    pub fn run(self, _arg: Word, account: &mut NativeAccount) {
        let storage = active_note::get_storage();
        let lp_asset = Asset::new(
            Word::from([storage[0], storage[1], storage[2], storage[3]]),
            Word::from([storage[4], Felt::ZERO, Felt::ZERO, Felt::ZERO]),
        );
        let recipient = AccountId::new(storage[6], storage[7]);
        let serial_num = Word::from([storage[7], storage[8], storage[9], storage[10]]);
        account.mint(lp);
    }
}
