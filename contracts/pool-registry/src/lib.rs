//! Pool registry account component.
//!
//! Holds the canonical set of XYK pools. A pool is registered by a register
//! note consumed by this account. Registration verifies the pool's code
//! commitment by foreign procedure invocation against the pool's
//! `CodeInspection::get_code_commitment` procedure, then records the pool and
//! its ordered asset pair.
//!
//! Storage:
//! - `accepted_code_commitments`: pool code commitment -> 1 if accepted.
//! - `pools`: pool id key -> pool code commitment.
//! - `assets_to_pool`: ordered asset pair -> pool id key.
//!
//! Keys are four-felt words. A pool id key is `[prefix, suffix, 0, 0]`; an
//! ordered asset pair is `[lo.prefix, lo.suffix, hi.prefix, hi.suffix]` with the
//! pair ordered by `(prefix, suffix)` as canonical integers.

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

use miden::*;

/// Root of the `get_code_commitment` procedure of the `CodeInspection` account
/// component (`miden-standards` 0.17.1).
///
/// Computed with `cargo run -p integration --bin code_inspection_roots` and
/// verified against the web-sdk compile of the same MASM. The root changes only
/// with the standards version.
fn get_code_commitment_root() -> Word {
    Word::new([
        Felt::new_unchecked(12042007455331814214),
        Felt::new_unchecked(11245817440613954134),
        Felt::new_unchecked(5438815964629222865),
        Felt::new_unchecked(15669238713621113888),
    ])
}

/// Root of the pool's `get_registry_identity` procedure.
///
/// Printed by `cargo run -p integration --bin registry_roots --release -- port0`.
/// The registry reads a pool's registry pointer, token keys, and fee rates
/// through it, because the code commitment pins code and these are storage.
///
/// The pool's storage interface names its slots, so this root moves with the
/// pool's package name. Re-print it after any change to `contracts/port0`.
fn get_registry_identity_root() -> Word {
    Word::new([
        Felt::new_unchecked(1389409443190994318),
        Felt::new_unchecked(4887066864456630200),
        Felt::new_unchecked(7355368621886472789),
        Felt::new_unchecked(9481342496745395225),
    ])
}

#[component_storage]
struct PoolRegistryStorage {
    #[storage(description = "accepted pool code commitment -> 1")]
    accepted_code_commitments: StorageMap<Word, Felt>,
    #[storage(description = "pool id key -> pool code commitment")]
    pools: StorageMap<Word, Word>,
    #[storage(description = "ordered asset pair -> pool id key")]
    assets_to_pool: StorageMap<Word, Word>,
    #[storage(description = "Account that receives the pools' protocol fees")]
    protocol_fee_to: StorageValue<AccountId>,
    #[storage(description = "Protocol fee in bps that a pool must be created with")]
    protocol_fee_share_bps: StorageValue<Felt>,
    #[storage(description = "LP fee tier in bps -> 1 if a pool may charge it")]
    allowed_lp_fee_bps: StorageMap<Felt, Felt>,
    #[storage(description = "Script root of the note that pays register fees out")]
    p2id_root: StorageValue<Word>,
}

#[component]
trait PoolRegistry {
    #[account_procedure]
    fn register(&mut self, pool_id: AccountId, asset_0: AccountId, asset_1: AccountId);

    #[account_procedure]
    fn pool_code_commitment(&self, pool_id: AccountId) -> Word;

    /// `[found, prefix, suffix, 0]`: `found` is `1` and the rest is the pool id,
    /// or all four felts are zero when the pair has no pool.
    #[account_procedure]
    fn pool_for_assets(&self, asset_0: AccountId, asset_1: AccountId) -> Word;

    #[account_procedure]
    fn is_pair_registered(&self, asset_0: AccountId, asset_1: AccountId) -> bool;

    /// Reads the account that receives every registered pool's protocol fees.
    ///
    /// A pool reads this by foreign procedure invocation when it collects, so
    /// the protocol's revenue address is set in one place rather than stored in
    /// every pool. The procedure takes no inputs and returns the id as four
    /// felts: `[prefix, suffix, 0, 0]`.
    #[account_procedure]
    fn protocol_fee_to(&self) -> Word;

    /// Reads the protocol fee in basis points a pool must be created with.
    #[account_procedure]
    fn protocol_fee_share_bps(&self) -> u64;

    /// Moves the registry's whole balance of `asset_key` to the protocol account.
    ///
    /// Permissionless, like a pool's collect: the destination is the protocol
    /// account held in storage, so a caller can only flush accumulated register
    /// fees toward it, never redirect them.
    #[account_procedure]
    fn collect(&mut self, asset_key: Word, serial_num: Word);
}

#[component]
impl PoolRegistry for PoolRegistryStorage {
    fn register(&mut self, pool_id: AccountId, asset_0: AccountId, asset_1: AccountId) {
        assert!(
            !assets_equal(asset_0, asset_1),
            "Both assets are the same"
        );

        let pool_key = pool_id_key(pool_id);
        assert!(
            self.pools.get(pool_key) == zero_word(),
            "Pool id already registered"
        );

        let (asset_lo, asset_hi) = order_assets(asset_0, asset_1);
        let pair_key = asset_pair_key(asset_lo, asset_hi);
        assert!(
            !self.is_pair_registered(asset_0, asset_1),
            "Pool for these assets already registered"
        );

        let commitment = pool_code_commitment_of(pool_id);
        assert!(
            self.accepted_code_commitments.get(commitment) == Felt::ONE,
            "Pool code commitment not accepted"
        );

        // The commitment pins code; these are storage, so they are read from the
        // pool. The registry is the only place the protocol's fee destination
        // lives, so a pool that points elsewhere must not be listed.
        let identity = tx::execute_foreign_procedure(
            pool_id,
            get_registry_identity_root(),
            tx::ForeignProcedureInputs::new([]),
        );
        let pool_registry = AccountId::new(identity.get(0), identity.get(1));
        let pool_token0 = Word::new([
            identity.get(2),
            identity.get(3),
            identity.get(4),
            identity.get(5),
        ]);
        let pool_token1 = Word::new([
            identity.get(6),
            identity.get(7),
            identity.get(8),
            identity.get(9),
        ]);
        let pool_protocol_fee = identity.get(10).as_canonical_u64();
        let pool_lp_fee = identity.get(11).as_canonical_u64();

        assert!(
            pool_registry.eq(&active_account::get_id()),
            "Pool is registered to a different registry"
        );
        // Either order is accepted here; the registry orders the pair itself.
        let key0 = asset_key_of(asset_0);
        let key1 = asset_key_of(asset_1);
        let matched = (pool_token0.eq(&key0) && pool_token1.eq(&key1))
            || (pool_token0.eq(&key1) && pool_token1.eq(&key0));
        assert!(matched, "Pool tokens do not match the claimed pair");
        assert!(
            pool_protocol_fee == self.protocol_fee_share_bps.get().as_canonical_u64(),
            "Pool protocol fee rate differs from the registry's"
        );
        assert!(
            self.allowed_lp_fee_bps.get(Felt::new_unchecked(pool_lp_fee)) == Felt::ONE,
            "Pool LP fee is not an allowed tier"
        );

        self.pools.set(pool_key, commitment);
        self.assets_to_pool.set(pair_key, pool_key);
    }

    fn pool_code_commitment(&self, pool_id: AccountId) -> Word {
        self.pools.get(pool_id_key(pool_id))
    }

    fn pool_for_assets(&self, asset_0: AccountId, asset_1: AccountId) -> Word {
        let (asset_lo, asset_hi) = order_assets(asset_0, asset_1);
        let stored = self.assets_to_pool.get(asset_pair_key(asset_lo, asset_hi));
        if stored == zero_word() {
            // No pool for this pair. A zero account id is not a pool id, so the
            // result is flagged rather than returned as one.
            return zero_word();
        }
        let pool = account_id_from_key(stored);
        Word::new([Felt::ONE, pool.prefix, pool.suffix, Felt::ZERO])
    }

    fn is_pair_registered(&self, asset_0: AccountId, asset_1: AccountId) -> bool {
        let (asset_lo, asset_hi) = order_assets(asset_0, asset_1);
        self.assets_to_pool.get(asset_pair_key(asset_lo, asset_hi)) != zero_word()
    }

    fn protocol_fee_to(&self) -> Word {
        let id = self.protocol_fee_to.get();
        Word::new([id.prefix, id.suffix, Felt::ZERO, Felt::ZERO])
    }

    fn protocol_fee_share_bps(&self) -> u64 {
        self.protocol_fee_share_bps.get().as_canonical_u64()
    }

    fn collect(&mut self, asset_key: Word, serial_num: Word) {
        let p2id_root = self.p2id_root.get();
        assert!(
            p2id_root != zero_word(),
            "Registry P2ID root is not set, so fees cannot be paid out"
        );
        assert!(asset_key != zero_word(), "Asset key must not be empty");

        let amount = active_account::get_asset(AssetId::from(asset_key))[0].as_canonical_u64();
        assert!(amount > 0, "Registry holds none of that asset");

        let fee_to = self.protocol_fee_to.get();
        let recipient = note::build_recipient(
            serial_num,
            p2id_root,
            alloc::vec![fee_to.suffix, fee_to.prefix, Felt::ZERO, Felt::ZERO],
        );
        let note_idx = output_note::create(
            Tag::from(account_target_tag(fee_to)),
            NoteType::from(Felt::new_unchecked(1)),
            recipient,
        );
        let asset = compose_asset(asset_key, amount);
        native_account::remove_asset(asset);
        output_note::add_asset(asset, note_idx);
    }
}

/// Reads the pool's code commitment by foreign procedure invocation against
/// the pool's `CodeInspection` component.
///
/// The `get_code_commitment` procedure takes no inputs and returns one word.
/// The executing transaction must be able to resolve the pool as a foreign
/// account; the node's network transaction builder fetches it from the chain
/// on demand.
fn pool_code_commitment_of(pool_id: AccountId) -> Word {
    let outputs = tx::execute_foreign_procedure(
        pool_id,
        get_code_commitment_root(),
        tx::ForeignProcedureInputs::new([]),
    );
    Word::new([
        outputs.get(0),
        outputs.get(1),
        outputs.get(2),
        outputs.get(3),
    ])
}

fn zero_word() -> Word {
    Word::new([Felt::ZERO, Felt::ZERO, Felt::ZERO, Felt::ZERO])
}

fn assets_equal(a: AccountId, b: AccountId) -> bool {
    a.prefix == b.prefix && a.suffix == b.suffix
}

fn order_assets(a: AccountId, b: AccountId) -> (AccountId, AccountId) {
    let a_key = (a.prefix.as_canonical_u64(), a.suffix.as_canonical_u64());
    let b_key = (b.prefix.as_canonical_u64(), b.suffix.as_canonical_u64());
    if a_key <= b_key {
        (a, b)
    } else {
        (b, a)
    }
}

/// Composes a fungible asset from its key and its whole amount.
fn compose_asset(asset_key: Word, amount: u64) -> Asset {
    Asset::new(
        asset_key,
        Word::new([
            Felt::new_unchecked(amount),
            Felt::ZERO,
            Felt::ZERO,
            Felt::ZERO,
        ]),
    )
}

/// Note tag addressed to `id`, mirroring `port0`'s `account_target_tag`.
fn account_target_tag(id: AccountId) -> Felt {
    const TAG_LEN: u32 = 14;
    let high = (id.prefix.as_canonical_u64() >> 32) as u32;
    Felt::from(high & (u32::MAX << (32 - TAG_LEN)))
}

/// The asset key of a fungible asset issued by `faucet`: `[0, 0, suffix | 0x11, prefix]`.
fn asset_key_of(faucet: AccountId) -> Word {
    Word::new([
        Felt::ZERO,
        Felt::ZERO,
        Felt::new_unchecked(faucet.suffix.as_canonical_u64() | 0x11),
        faucet.prefix,
    ])
}

fn pool_id_key(pool_id: AccountId) -> Word {
    Word::new([pool_id.prefix, pool_id.suffix, Felt::ZERO, Felt::ZERO])
}

fn account_id_from_key(key: Word) -> AccountId {
    let [prefix, suffix, _, _] = key.into_elements();
    AccountId::new(prefix, suffix)
}

fn asset_pair_key(asset_lo: AccountId, asset_hi: AccountId) -> Word {
    Word::new([
        asset_lo.prefix,
        asset_lo.suffix,
        asset_hi.prefix,
        asset_hi.suffix,
    ])
}
