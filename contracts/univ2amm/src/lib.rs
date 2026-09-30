// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod amm;
pub mod error;
pub mod utils;

use miden::{
    AccountId, Asset, Felt, NoteIdx, NoteType, StorageValue, Tag, Word, active_account,
    active_note, component, component_storage,
    native_account::{self, NativeAccount},
    note, output_note,
};

use crate::{
    amm::{
        burn::quote_burn, fees::Fees, mint::quote_mint, reserves::Reserves, swap::amount_out,
        token::Tokens,
    },
    utils::asset::{compose_asset, get_asset_key_from_faucet_id},
};

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
    #[storage(description = "P2ID root")]
    p2id_root: StorageValue<Word>,
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
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        min_lp_out: u64,
    );
    #[account_procedure]
    fn burn(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        min_token0_out: u64,
        min_token1_out: u64,
    );
    #[account_procedure]
    fn swap(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        min_amount_out: u64,
    );
    #[account_procedure]
    fn skim(&mut self);
}

impl Univ2ammStorage {
    fn add_asset_to_output_note(&mut self, asset: Asset, note_idx: NoteIdx) {
        output_note::add_asset(asset, note_idx);
    }
    fn prepare_output_note(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        script_root: Word,
    ) -> NoteIdx {
        let note_recipient = note::build_recipient(
            serial_num,
            script_root,
            alloc::vec![recipient.suffix, recipient.prefix],
        );
        let note_idx =
            output_note::create(Tag::from(tag), NoteType::from(note_type), note_recipient);
        note_idx
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
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        min_lp_out: u64,
    ) {
        let mut amount0_in_note = 0u64;
        let mut amount1_in_note = 0u64;
        for asset in active_note::get_initial_assets() {
            assert!(
                asset.is_fungible(),
                "Mint note contains non-fungible tokens"
            );
            let account_id = AccountId::new(asset.key[3], asset.key[2]);
            if account_id.eq(&self.token0()) {
                amount0_in_note = asset.value[0].as_canonical_u64();
            } else if account_id.eq(&self.token1()) {
                amount1_in_note = asset.value[0].as_canonical_u64();
            } else {
                panic!("Mint note has an asset that is neither token0 nor token1");
            }
            native_account::remove_asset(asset);
            self.add_asset(asset);
        }
        let quote = quote_mint(amount0_in_note, amount1_in_note, self.reserves.get()).unwrap();
        assert!(
            quote.liquidity_to_user >= min_lp_out,
            "LP out would be less than desired"
        );
        let account_id = active_account::get_id();
        let lp_key = Word::from([Felt::ZERO, Felt::ZERO, account_id.suffix, account_id.prefix]);
        let lp_asset = Asset::new(
            lp_key,
            Word::from([
                Felt::new_unchecked(quote.liquidity_to_user),
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
            ]),
        );
        let lp_asset = self.issue(lp_asset);
        let note_idx =
            self.prepare_output_note(recipient, serial_num, tag, note_type, self.p2id_root.get());
        self.add_asset_to_output_note(lp_asset, note_idx);
        self.reserves.set(Reserves {
            reserve0: quote.reserve0_after,
            reserve1: quote.reserve1_after,
            lp_supply: quote.lp_supply_after,
        });
    }
    fn burn(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        min_amount0_out: u64,
        min_amount1_out: u64,
    ) {
        let assets = active_note::get_initial_assets();
        assert!(assets.len() == 1, "Burn note should contain only one asset");
        let asset = assets[0];
        assert!(
            asset.is_fungible(),
            "Burn note contains a non-fungible token"
        );
        let account_id = AccountId::new(asset.key[3], asset.key[2]);
        assert!(
            account_id.eq(&active_account::get_id()),
            "Burn note has an asset that is not the LP token"
        );
        let amount_in_note = asset.value[0].as_canonical_u64();
        let quote = quote_burn(amount_in_note, self.reserves.get()).unwrap();
        assert!(
            quote.amount0 >= min_amount0_out,
            "Amount of token0 out would be less than desired"
        );
        assert!(
            quote.amount1 >= min_amount1_out,
            "Amount of token1 out would be less than desired"
        );
        let token0_key = Word::from([
            Felt::ZERO,
            Felt::ZERO,
            self.token0().suffix,
            self.token0().prefix,
        ]);
        let token1_key = Word::from([
            Felt::ZERO,
            Felt::ZERO,
            self.token1().suffix,
            self.token1().prefix,
        ]);
        let asset0 = Asset::new(
            token0_key,
            Word::from([
                Felt::new_unchecked(quote.amount0),
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
            ]),
        );
        let asset1 = Asset::new(
            token1_key,
            Word::from([
                Felt::new_unchecked(quote.amount1),
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
            ]),
        );
        let note_idx =
            self.prepare_output_note(recipient, serial_num, tag, note_type, self.p2id_root.get());
        self.add_asset_to_output_note(asset0, note_idx);
        self.add_asset_to_output_note(asset1, note_idx);
        native_account::remove_asset(asset0);
        native_account::remove_asset(asset1);
        self.redeem(asset);
        self.reserves.set(Reserves {
            reserve0: quote.reserve0_after,
            reserve1: quote.reserve1_after,
            lp_supply: quote.lp_supply_after,
        });
    }
    fn swap(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        note_type: Felt,
        min_amount_out: u64,
    ) {
        let assets = active_note::get_initial_assets();
        assert!(assets.len() == 1, "Swap note should contain only one asset");
        let asset = assets[0];
        assert!(
            asset.is_fungible(),
            "Swap note contains a non-fungible token"
        );
        let token_id_in = AccountId::new(asset.key[3], asset.key[2]);
        let reserves = self.reserves.get();
        let (token_in, token_out, reserve_in, reserve_out) = if token_id_in.eq(&self.token0()) {
            (
                self.token0(),
                self.token1(),
                reserves.reserve0(),
                reserves.reserve1(),
            )
        } else if token_id_in.eq(&self.token1()) {
            (
                self.token1(),
                self.token0(),
                reserves.reserve1(),
                reserves.reserve0(),
            )
        } else {
            panic!("Swap note has an asset that is neither token0 nor token1");
        };
        let amount_in = asset.value[0].as_canonical_u64();
        let quote =
            amount_out(amount_in, reserve_in, reserve_out, self.fees.get().fee_bps).unwrap();
        assert!(
            quote.amount_out >= min_amount_out,
            "Amount out would be less than desired"
        );
        let token_out_key =
            Word::from([Felt::ZERO, Felt::ZERO, token_out.suffix, token_out.prefix]);
        let asset_out = Asset::new(
            token_out_key,
            Word::from([
                Felt::new_unchecked(quote.amount_out),
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
            ]),
        );
        let note_idx =
            self.prepare_output_note(recipient, serial_num, tag, note_type, self.p2id_root.get());
        native_account::remove_asset(asset);
        self.add_asset_to_output_note(asset_out, note_idx);
        self.add_asset(asset);
        let (reserve0_after, reserve1_after) = if token_in.eq(&self.token0()) {
            (quote.reserve_in_after, quote.reserve_out_after)
        } else {
            (quote.reserve_out_after, quote.reserve_in_after)
        };
        self.reserves.set(Reserves {
            reserve0: reserve0_after,
            reserve1: reserve1_after,
            lp_supply: reserves.lp_supply,
        });
    }
    fn skim(&mut self) {
        let token0_word = get_asset_key_from_faucet_id(&self.token0()).unwrap();
        let token1_word = get_asset_key_from_faucet_id(&self.token1()).unwrap();
        let token0_amount = active_account::get_asset(token0_word).a.as_canonical_u64();
        let token1_amount = active_account::get_asset(token1_word).a.as_canonical_u64();
        let reserves = self.reserves.get();
        let mut skim = (0, 0);
        if token0_amount > reserves.reserve0 {
            skim.0 = token0_amount - reserves.reserve0;
        }
        if token1_amount > reserves.reserve1 {
            skim.1 = token1_amount - reserves.reserve1;
        }
        if skim.0 > 0 || skim.1 > 0 {
            let fees = self.fees.get();
            let note_idx = self.prepare_output_note(
                fees.fee_to(),
                Word::empty(),
                Felt::ZERO,
                Felt::new_unchecked(1).into(),
                self.p2id_root.get(),
            );
            if skim.0 > 0 {
                let asset = compose_asset(self.token0(), skim.0).unwrap();
                self.add_asset_to_output_note(asset, note_idx);
                native_account::remove_asset(asset);
            }
            if skim.1 > 0 {
                let asset = compose_asset(self.token1(), skim.1).unwrap();
                self.add_asset_to_output_note(asset, note_idx);
                native_account::remove_asset(asset);
            }
        }
    }
}
