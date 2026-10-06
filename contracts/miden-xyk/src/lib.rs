// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod amm;
pub mod error;
pub mod utils;

use miden::{
    AccountId, Asset, Felt, NoteIdx, NoteType, StorageValue, Tag, Word, active_account,
    active_note, component, component_storage, faucet,
    native_account::{self, NativeAccount},
    note, output_note, tx,
};

use crate::{
    amm::{
        burn::{BurnQuote, quote_burn},
        fees::{Fees, ProtocolPot},
        mint::{MintQuote, quote_mint},
        reserves::Reserves,
        swap::{SwapQuote, amount_out},
    },
    utils::{asset::compose_asset, math::add_amount, note::account_target_tag, u128::U128},
};

#[component_storage]
struct MidenXykStorage {
    #[storage(description = "LP token asset key")]
    tokenlp_asset_key: StorageValue<Word>,
    #[storage(description = "token0 asset key")]
    token0_asset_key: StorageValue<Word>,
    #[storage(description = "token1 asset key")]
    token1_asset_key: StorageValue<Word>,
    #[storage(description = "Total lp supply")]
    lp_supply: StorageValue<Felt>,
    #[storage(description = "Fee beneficiary and the fee amount")]
    fees: StorageValue<Fees>,
    #[storage(description = "AccountId that is permitted to make admin changes")]
    admin: StorageValue<AccountId>,
    #[storage(description = "P2ID root")]
    p2id_root: StorageValue<Word>,
    #[storage(description = "Accrued protocol fees")]
    protocol_pot: StorageValue<ProtocolPot>,
    #[storage(description = "Cumulative last price0")]
    price0_cumulative_last: StorageValue<U128>,
    #[storage(description = "Cumulative last price1")]
    price1_cumulative_last: StorageValue<U128>,
    #[storage(description = "Last price change timestamp")]
    block_timestamp_last: StorageValue<Felt>,
}

#[component]
trait MidenXyk: NativeAccount {
    #[account_procedure]
    fn tokenlp_asset_key(&self) -> Word;
    #[account_procedure]
    fn token0_asset_key(&self) -> Word;
    #[account_procedure]
    fn token1_asset_key(&self) -> Word;
    #[account_procedure]
    fn lp_supply(&self) -> Felt;
    #[account_procedure]
    fn get_reserves(&self) -> Reserves;
    #[account_procedure]
    fn get_fees(&self) -> Fees;
    #[account_procedure]
    fn mint(&mut self, recipient: AccountId, serial_num: Word, tag: Felt, min_lp_out: u64);
    #[account_procedure]
    fn burn(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        min_token0_out: u64,
        min_token1_out: u64,
    );
    #[account_procedure]
    fn swap(&mut self, recipient: AccountId, serial_num: Word, tag: Felt, min_amount_out: u64);
    #[account_procedure]
    fn quote_swap(&self, token_in_is_0: bool, amount_in: u64) -> SwapQuote;
    #[account_procedure]
    fn collect(&mut self, serial_num: Word);
    #[account_procedure]
    fn configure(
        &mut self,
        lp_fee: u64,
        protocol_fee: u64,
        protocol_fee_to: AccountId,
        p2id_root: Word,
        tokenlp_asset_key: Word,
    );
}

impl MidenXykStorage {
    fn add_asset_to_output_note(&mut self, asset: Asset, note_idx: NoteIdx) {
        output_note::add_asset(asset, note_idx);
    }
    fn prepare_output_note(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        script_root: Word,
    ) -> NoteIdx {
        let note_recipient = note::build_recipient(
            serial_num,
            script_root,
            alloc::vec![recipient.suffix, recipient.prefix],
        );
        let note_idx = output_note::create(
            Tag::from(tag),
            NoteType::from(Felt::new_unchecked(1)),
            note_recipient,
        );
        note_idx
    }
    fn update_oracle(&mut self, reserves: Reserves) {
        let now = tx::get_block_timestamp() as u64;
        let mut last = self.block_timestamp_last.get().as_canonical_u64();
        if now < last {
            last = now;
        }
        self.block_timestamp_last.set(Felt::new_unchecked(now));
        if reserves.reserve0 == 0 || reserves.reserve1 == 0 {
            return;
        }
        let elapsed = (now - last) as u128;
        let p0 = ((reserves.reserve1 as u128) << 64) / reserves.reserve0 as u128;
        let p1 = ((reserves.reserve0 as u128) << 64) / reserves.reserve1 as u128;
        // Saturate rather than wrap: cumulative prices are monotonic and clamp at u128::MAX,
        // so consumers subtract two readings directly instead of doing modular arithmetic.
        let c0 = self
            .price0_cumulative_last
            .get()
            .as_u128()
            .saturating_add(p0.saturating_mul(elapsed));
        let c1 = self
            .price1_cumulative_last
            .get()
            .as_u128()
            .saturating_add(p1.saturating_mul(elapsed));
        self.price0_cumulative_last.set(U128::new(c0));
        self.price1_cumulative_last.set(U128::new(c1));
    }
}

#[component]
impl MidenXyk for MidenXykStorage {
    fn tokenlp_asset_key(&self) -> Word {
        self.tokenlp_asset_key.get()
    }
    fn token0_asset_key(&self) -> Word {
        self.token0_asset_key.get()
    }
    fn token1_asset_key(&self) -> Word {
        self.token1_asset_key.get()
    }
    fn lp_supply(&self) -> Felt {
        self.lp_supply.get()
    }
    fn get_fees(&self) -> Fees {
        self.fees.get()
    }
    fn get_reserves(&self) -> Reserves {
        let protocol_pot = self.protocol_pot.get();
        let token0_asset_key = self.token0_asset_key();
        let token1_asset_key = self.token1_asset_key();
        let amount0 = active_account::get_asset(token0_asset_key)[0].as_canonical_u64();
        let amount1 = active_account::get_asset(token1_asset_key)[0].as_canonical_u64();
        assert!(
            amount0 >= protocol_pot.amount0 && amount1 >= protocol_pot.amount1,
            "Protocol pot has larger amount than balances in the vault"
        );
        Reserves {
            reserve0: amount0 - protocol_pot.amount0,
            reserve1: amount1 - protocol_pot.amount1,
        }
    }
    fn mint(&mut self, recipient: AccountId, serial_num: Word, tag: Felt, min_lp_out: u64) {
        assert!(
            self.tokenlp_asset_key.get() != Word::default(),
            "Pool not configured yet"
        );
        let reserves = self.get_reserves();
        self.update_oracle(reserves);
        let mut amount0_in_note = 0u64;
        let mut amount1_in_note = 0u64;
        let (token0_asset_key, token1_asset_key, tokenlp_asset_key) = (
            self.token0_asset_key(),
            self.token1_asset_key(),
            self.tokenlp_asset_key(),
        );
        for asset in active_note::get_initial_assets() {
            assert!(
                asset.is_fungible(),
                "Mint note contains non-fungible tokens"
            );
            if asset.key.eq(&token0_asset_key) {
                amount0_in_note = asset.value[0].as_canonical_u64();
            } else if asset.key.eq(&token1_asset_key) {
                amount1_in_note = asset.value[0].as_canonical_u64();
            } else {
                panic!("Mint note has an asset that is neither token0 nor token1");
            }
            self.add_asset(asset);
        }

        let MintQuote {
            liquidity_to_user,
            lp_supply_after,
            reserve0_after: _,
            reserve1_after: _,
            refund0,
            refund1,
        } = quote_mint(
            amount0_in_note,
            amount1_in_note,
            reserves,
            self.lp_supply.get().as_canonical_u64(),
        )
        .unwrap();
        assert!(
            liquidity_to_user >= min_lp_out,
            "LP out would be less than desired"
        );
        let note_idx = self.prepare_output_note(recipient, serial_num, tag, self.p2id_root.get());
        let lp_asset = compose_asset(tokenlp_asset_key, liquidity_to_user).unwrap();
        faucet::mint(lp_asset);
        self.add_asset_to_output_note(lp_asset, note_idx);
        if refund0 > 0 {
            let asset = compose_asset(token0_asset_key, refund0).unwrap();
            self.add_asset_to_output_note(asset, note_idx);
            native_account::remove_asset(asset);
        }
        if refund1 > 0 {
            let asset = compose_asset(token1_asset_key, refund1).unwrap();
            self.add_asset_to_output_note(asset, note_idx);
            native_account::remove_asset(asset);
        }
        self.lp_supply.set(Felt::new_unchecked(lp_supply_after));
    }
    fn burn(
        &mut self,
        recipient: AccountId,
        serial_num: Word,
        tag: Felt,
        min_amount0_out: u64,
        min_amount1_out: u64,
    ) {
        assert!(
            self.tokenlp_asset_key.get() != Word::default(),
            "Pool not configured yet"
        );
        self.update_oracle(self.get_reserves());
        let (token0_asset_key, token1_asset_key, tokenlp_asset_key) = (
            self.token0_asset_key(),
            self.token1_asset_key(),
            self.tokenlp_asset_key(),
        );
        let assets = active_note::get_initial_assets();
        assert!(assets.len() == 1, "Burn note should contain only one asset");
        let asset = assets[0];
        assert!(
            asset.is_fungible(),
            "Burn note contains a non-fungible token"
        );
        assert!(
            asset.key.eq(&tokenlp_asset_key),
            "Burn note has an asset that is not the LP token"
        );
        let amount_in_note = asset.value[0].as_canonical_u64();
        let BurnQuote {
            amount0,
            amount1,
            lp_supply_after,
            reserve0_after: _,
            reserve1_after: _,
        } = quote_burn(
            amount_in_note,
            self.get_reserves(),
            self.lp_supply.get().as_canonical_u64(),
        )
        .unwrap();
        assert!(
            amount0 >= min_amount0_out,
            "Amount of token0 out would be less than desired"
        );
        assert!(
            amount1 >= min_amount1_out,
            "Amount of token1 out would be less than desired"
        );
        let asset0 = compose_asset(token0_asset_key, amount0).unwrap();
        let asset1 = compose_asset(token1_asset_key, amount1).unwrap();
        let note_idx = self.prepare_output_note(recipient, serial_num, tag, self.p2id_root.get());
        self.add_asset_to_output_note(asset0, note_idx);
        self.add_asset_to_output_note(asset1, note_idx);
        native_account::remove_asset(asset0);
        native_account::remove_asset(asset1);
        faucet::burn(asset);
        self.lp_supply.set(Felt::new_unchecked(lp_supply_after));
    }
    fn quote_swap(&self, token_in_is_0: bool, amount_in: u64) -> SwapQuote {
        let Reserves { reserve0, reserve1 } = self.get_reserves();
        let (reserve_in, reserve_out) = if token_in_is_0 {
            (reserve0, reserve1)
        } else {
            (reserve1, reserve0)
        };
        let fees = self.fees.get();
        let quote = amount_out(
            amount_in,
            reserve_in,
            reserve_out,
            fees.lp_fee_bps,
            fees.protocol_fee_bps,
        )
        .unwrap();
        quote
    }
    fn swap(&mut self, recipient: AccountId, serial_num: Word, tag: Felt, min_amount_out: u64) {
        assert!(
            self.tokenlp_asset_key.get() != Word::default(),
            "Pool not configured yet"
        );
        let reserves = self.get_reserves();
        self.update_oracle(reserves);
        let assets = active_note::get_initial_assets();
        assert!(assets.len() == 1, "Swap note should contain only one asset");
        let asset = assets[0];
        assert!(
            asset.is_fungible(),
            "Swap note contains a non-fungible token"
        );
        let (token0_asset_key, token1_asset_key) =
            (self.token0_asset_key(), self.token1_asset_key());
        let (token_in, token_out, reserve_in, reserve_out) = if asset.key.eq(&token0_asset_key) {
            (
                token0_asset_key,
                token1_asset_key,
                reserves.reserve0,
                reserves.reserve1,
            )
        } else if asset.key.eq(&token1_asset_key) {
            (
                token1_asset_key,
                token0_asset_key,
                reserves.reserve1,
                reserves.reserve0,
            )
        } else {
            panic!("Swap note has an asset that is neither token0 nor token1");
        };
        let amount_in = asset.value[0].as_canonical_u64();
        let fees = self.fees.get();
        let SwapQuote {
            amount_out,
            reserve_in_after: _,
            reserve_out_after: _,
            protocol_fee,
        } = amount_out(
            amount_in,
            reserve_in,
            reserve_out,
            fees.lp_fee_bps,
            fees.protocol_fee_bps,
        )
        .unwrap();
        assert!(
            amount_out >= min_amount_out,
            "Amount out would be less than desired"
        );
        let asset_out = compose_asset(token_out, amount_out).unwrap();
        let note_idx = self.prepare_output_note(recipient, serial_num, tag, self.p2id_root.get());
        native_account::remove_asset(asset_out);
        self.add_asset_to_output_note(asset_out, note_idx);
        self.add_asset(asset);
        let (amount0_protocol_fee, amount1_protocol_fee) = if token_in.eq(&token0_asset_key) {
            (protocol_fee, 0)
        } else {
            (0, protocol_fee)
        };
        let mut protocol_pot = self.protocol_pot.get();
        protocol_pot.add_to_pot(amount0_protocol_fee, amount1_protocol_fee);
        self.protocol_pot.set(protocol_pot);
    }
    fn collect(&mut self, serial_num: Word) {
        assert!(
            self.tokenlp_asset_key.get() != Word::default(),
            "Pool not configured yet"
        );
        let mut protocol_pot = self.protocol_pot.get();
        let token0_amount = protocol_pot.amount0();
        let token1_amount = protocol_pot.amount1();
        assert!(
            token0_amount > 0 || token1_amount > 0,
            "Protocol pot is empty"
        );
        let (token0_asset_key, token1_asset_key) =
            (self.token0_asset_key(), self.token1_asset_key());
        let fee_to = self.fees.get().protocol_fee_to;
        let note_idx = self.prepare_output_note(
            fee_to,
            serial_num,
            account_target_tag(fee_to),
            self.p2id_root.get(),
        );
        if token0_amount > 0 {
            let asset = compose_asset(token0_asset_key, token0_amount).unwrap();
            native_account::remove_asset(asset);
            self.add_asset_to_output_note(asset, note_idx);
            protocol_pot.reset_amount0();
        }
        if token1_amount > 0 {
            let asset = compose_asset(token1_asset_key, token1_amount).unwrap();
            native_account::remove_asset(asset);
            self.add_asset_to_output_note(asset, note_idx);
            protocol_pot.reset_amount1();
        }
        self.protocol_pot.set(protocol_pot);
    }
    fn configure(
        &mut self,
        lp_fee_bps: u64,
        protocol_fee_bps: u64,
        protocol_fee_to: AccountId,
        p2id_root: Word,
        tokenlp_asset_key: Word,
    ) {
        assert!(
            active_note::get_sender().eq(&self.admin.get()),
            "Configuration can only be with the admin account"
        );
        assert!(
            tokenlp_asset_key.ne(&Word::default()),
            "LP token asset key can't be an empty word"
        );
        assert!(
            self.tokenlp_asset_key.get() == Word::default()
                || self.tokenlp_asset_key.get() == tokenlp_asset_key,
            "LP token asset key is already set"
        );
        assert!(
            self.token0_asset_key.get().ne(&self.token1_asset_key.get()),
            "token0 must be different than token1"
        );
        assert!(
            tokenlp_asset_key.ne(&self.token0_asset_key.get()),
            "LP token must be different than token0"
        );
        assert!(
            tokenlp_asset_key.ne(&self.token1_asset_key.get()),
            "LP token must be different than token1"
        );
        assert!(
            add_amount(protocol_fee_bps, lp_fee_bps).unwrap() < 10000,
            "Total fee bps must be less than 10000"
        );
        self.tokenlp_asset_key.set(tokenlp_asset_key);
        self.fees.set(Fees {
            lp_fee_bps: lp_fee_bps as u32,
            protocol_fee_bps: protocol_fee_bps as u32,
            protocol_fee_to,
        });
        self.p2id_root.set(p2id_root);
    }
}
