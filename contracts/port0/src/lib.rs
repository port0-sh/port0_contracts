// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod amm;
pub mod error;
pub mod utils;

use miden::{
    AccountId, Asset, AssetId, Felt, NoteIdx, NoteType, StorageValue, Tag, Word, active_account,
    active_note, component, component_storage, export_type, faucet,
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
    utils::{asset::compose_asset, note::account_target_tag, u128::U128},
};

/// The pool's identity, as the registry sees it before listing the pool.
///
/// Field order is the order the values occupy when the procedure returns, which
/// is what a foreign caller reads: the registry decodes twelve felts as
/// `[prefix, suffix, token0[0..4], token1[0..4], protocol_fee_share_bps, lp_fee_bps]`.
#[export_type]
pub struct PoolIdentity {
    pub registry_id: AccountId,
    pub token0_key: Word,
    pub token1_key: Word,
    pub protocol_fee_share_bps: u32,
    pub lp_fee_bps: u32,
}

#[component_storage]
struct Port0Storage {
    #[storage(description = "token0 asset key")]
    token0_asset_key: StorageValue<Word>,
    #[storage(description = "token1 asset key")]
    token1_asset_key: StorageValue<Word>,
    #[storage(description = "Total lp supply")]
    lp_supply: StorageValue<Felt>,
    #[storage(description = "Fee beneficiary and the fee amount")]
    fees: StorageValue<Fees>,
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
    #[storage(description = "Registry that administers this pool")]
    registry_id: StorageValue<AccountId>,
}

/// Root of the pool-registry's `protocol_fee_to` procedure.
///
/// Printed by `cargo run -p integration --bin registry_roots --release`. The
/// pool reads the protocol's fee destination from the registry over foreign
/// procedure invocation, so the registry holds that address once for every
/// pool instead of each pool storing its own copy.
fn registry_protocol_fee_to_root() -> Word {
    Word::new([
        Felt::new_unchecked(15483598750818023257),
        Felt::new_unchecked(336910118429041028),
        Felt::new_unchecked(12277962169024785536),
        Felt::new_unchecked(13342452442204916062),
    ])
}

#[component]
trait Port0: NativeAccount {
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
    /// The pool's registry, token keys, and fee rates, for registration checks.
    #[account_procedure]
    fn get_registry_identity(&self) -> PoolIdentity;
}

impl Port0Storage {
    /// The pool's LP asset key, derived from the pool's own account id.
    ///
    /// The key cannot be part of the account's initial storage: it embeds the
    /// account id, and the account id is derived from the initial storage
    /// commitment, so seeding it would be circular. Deriving it here removes the
    /// configuration step that existed only to write this one word.
    ///
    /// The layout matches the SDK's `fungibleAssetIdWord`: `[0, 0, suffix |
    /// 0x11, prefix]`, where `0x11` is the fungible asset id metadata byte
    /// (composition 1, version 1).
    fn lp_asset_key(&self) -> Word {
        let id = active_account::get_id();
        Word::new([
            Felt::ZERO,
            Felt::ZERO,
            Felt::new_unchecked(id.suffix.as_canonical_u64() | 0x11),
            id.prefix,
        ])
    }

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
            alloc::vec![recipient.suffix, recipient.prefix, Felt::ZERO, Felt::ZERO],
        );
        let note_idx = output_note::create(
            Tag::from(tag),
            NoteType::from(Felt::new_unchecked(1)),
            note_recipient,
        );
        note_idx
    }
    /// Reads the protocol fee destination from this pool's registry.
    ///
    /// The registry returns the account id as `[prefix, suffix, 0, 0]`.
    fn registry_protocol_fee_to(&self) -> AccountId {
        let outputs = tx::execute_foreign_procedure(
            self.registry_id.get(),
            registry_protocol_fee_to_root(),
            tx::ForeignProcedureInputs::new([]),
        );
        AccountId::new(outputs.get(0), outputs.get(1))
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
impl Port0 for Port0Storage {
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
        let amount0 = active_account::get_asset(AssetId::from(token0_asset_key))[0].as_canonical_u64();
        let amount1 = active_account::get_asset(AssetId::from(token1_asset_key))[0].as_canonical_u64();
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
        let reserves = self.get_reserves();
        self.update_oracle(reserves);
        let mut amount0_in_note = 0u64;
        let mut amount1_in_note = 0u64;
        let (token0_asset_key, token1_asset_key, tokenlp_asset_key) = (
            self.token0_asset_key(),
            self.token1_asset_key(),
            self.lp_asset_key(),
        );
        for asset in active_note::get_initial_assets() {
            assert!(
                asset.is_fungible(),
                "Mint note contains non-fungible tokens"
            );
            if Word::from(asset.id).eq(&token0_asset_key) {
                amount0_in_note = asset.value[0].as_canonical_u64();
            } else if Word::from(asset.id).eq(&token1_asset_key) {
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
        self.update_oracle(self.get_reserves());
        let (token0_asset_key, token1_asset_key, tokenlp_asset_key) = (
            self.token0_asset_key(),
            self.token1_asset_key(),
            self.lp_asset_key(),
        );
        let assets = active_note::get_initial_assets();
        assert!(assets.len() == 1, "Burn note should contain only one asset");
        let asset = assets[0];
        assert!(
            asset.is_fungible(),
            "Burn note contains a non-fungible token"
        );
        assert!(
            Word::from(asset.id).eq(&tokenlp_asset_key),
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
            fees.protocol_fee_share_bps,
        )
        .unwrap();
        quote
    }
    fn swap(&mut self, recipient: AccountId, serial_num: Word, tag: Felt, min_amount_out: u64) {
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
        let (token_in, token_out, reserve_in, reserve_out) = if Word::from(asset.id).eq(&token0_asset_key) {
            (
                token0_asset_key,
                token1_asset_key,
                reserves.reserve0,
                reserves.reserve1,
            )
        } else if Word::from(asset.id).eq(&token1_asset_key) {
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
            fees.protocol_fee_share_bps,
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
    fn get_registry_identity(&self) -> PoolIdentity {
        let fees = self.get_fees();
        PoolIdentity {
            registry_id: self.registry_id.get(),
            token0_key: self.token0_asset_key.get(),
            token1_key: self.token1_asset_key.get(),
            protocol_fee_share_bps: fees.protocol_fee_share_bps,
            lp_fee_bps: fees.lp_fee_bps,
        }
    }

    fn collect(&mut self, serial_num: Word) {
        let mut protocol_pot = self.protocol_pot.get();
        let token0_amount = protocol_pot.amount0();
        let token1_amount = protocol_pot.amount1();
        assert!(
            token0_amount > 0 || token1_amount > 0,
            "Protocol pot is empty"
        );
        let (token0_asset_key, token1_asset_key) =
            (self.token0_asset_key(), self.token1_asset_key());
        // The destination is the registry's, not the pool's: the address can be
        // rotated in one place, and it is read only here, on the collect path.
        let fee_to = self.registry_protocol_fee_to();
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
}
