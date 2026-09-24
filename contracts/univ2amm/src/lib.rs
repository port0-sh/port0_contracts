// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod amm;
pub mod error;
pub mod utils;

use miden::{
    AccountId, Asset, Felt, NoteIdx, NoteType, StorageValue, Tag, Word, active_note, component,
    component_storage, input_note, native_account::NativeAccount, note, output_note,
};

use crate::amm::{fees::Fees, mint::quote_mint, reserves::Reserves, token::Tokens};

#[component_storage]
struct Univ2ammStorage {
    #[storage(description = "AccountIds of token0 and token1")]
    tokens: StorageValue<Tokens>,
    #[storage(description = "Reserves of token0, token1 and total lp supply")]
    reserves: StorageValue<Reserves>,
    #[storage(description = "Fee beneficiary and the fee amount")]
    fees: StorageValue<Fees>,
    #[storage(description = "AccountId that is permitted to make admin changes")]
    admin: StorageValue<AccountId>,
}

#[component(lp_faucet::LpFaucet)]
trait Univ2amm: NativeAccount + LpFaucet {
    #[account_procedure]
    fn token0(&self) -> AccountId;
    #[account_procedure]
    fn token1(&self) -> AccountId;
    #[account_procedure]
    fn get_reserves(&self) -> Reserves;
    #[account_procedure]
    fn get_fees(&self) -> Fees;
    #[account_procedure]
    fn mint(
        &mut self,
        lp: Asset,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        script_root: Word,
    );
    #[account_procedure]
    fn burn(&mut self) -> Felt;
    #[account_procedure]
    fn swap(&mut self) -> Felt;
}

impl Univ2ammStorage {
    fn send(
        &mut self,
        asset: Asset,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        script_root: Word,
    ) {
        let note_recipient = note::build_recipient(
            serial_num,
            script_root,
            alloc::vec![recipient.suffix, recipient.prefix],
        );
        let note_idx =
            output_note::create(Tag::from(tag), NoteType::from(note_type), note_recipient);
        output_note::add_asset(asset, note_idx);
    }
}

#[component]
impl Univ2amm for Univ2ammStorage {
    fn token0(&self) -> AccountId {
        self.tokens.get().token0()
    }
    fn token1(&self) -> AccountId {
        self.tokens.get().token1()
    }
    fn get_reserves(&self) -> Reserves {
        self.reserves.get()
    }
    fn get_fees(&self) -> Fees {
        self.fees.get()
    }
    fn mint(
        &mut self,
        lp_token: Asset,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        script_root: Word,
    ) -> Felt {
        for asset in active_note::get_initial_assets() {
            self.add_asset(asset);
        }
        let quote = quote_mint(0, 0, self.reserves.get()).unwrap();
        self.issue(quote.liquidity_to_user);
        self.send(
            quote.liquidity_to_user,
            recipient,
            serial_num,
            tag,
            note_type,
            script_root,
        );
    }
    fn burn(&mut self) -> Felt {
        Felt::ZERO
    }
    fn swap(&mut self) -> Felt {
        Felt::ZERO
    }
}
