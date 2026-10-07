//! Integration tests for the UniSwap V2 constant-product AMM (`miden_xyk`) contract.
//!
//! These tests exercise the pool end-to-end through MockChain: configuration via the
//! admin note, liquidity provision (mint), swaps, liquidity removal (burn), and protocol
//! fee collection (collect). The pool account issues its own LP token, so the LP token's
//! asset key is derived from the pool account id.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use integration::helpers::build_project_in_dir;
use miden_client::{
    account::{
        component::{InitStorageData, StorageValueName},
        AccountBuilder, AccountComponent, AccountId, AccountType, StorageSlotName,
    },
    asset::{Asset, FungibleAsset},
    auth::AuthSchemeId,
    note::Note,
    transaction::RawOutputNote,
    Felt, Word,
};
use miden_mast_package::Package;
use miden_standards::{note::P2idNote, testing::note::NoteBuilder};
use miden_testing::{AccountState, Auth, MockChain, MockChainBuilder};

/// Storage slot prefix shared by every `miden_xyk` component slot.
///
/// Slot names are `<package_name>::<interface_segment>::<field_name>`; the package name is
/// `miden-xyk` (from `miden-project.toml`) and the interface segment is `miden-xyk` (from the
/// `miden:miden-xyk/miden-xyk@0.1.0` namespace), both sanitized to `miden_xyk`.
const SLOT_PREFIX: &str = "miden_xyk::miden_xyk";

// Fee configuration (basis points), kept well below the 10000 combined cap.
const LP_FEE_BPS: u32 = 30;
const PROTOCOL_FEE_BPS: u32 = 10;

/// Mirrors `contracts/miden-xyk/src/utils/math.rs::MAX_AMOUNT` (the u128-safe clip).
const MAX_AMOUNT: u64 = 1 << 57;

fn basic_auth() -> Auth {
    Auth::BasicAuth {
        auth_scheme: AuthSchemeId::Falcon512Poseidon2,
    }
}

fn slot(field: &str) -> StorageSlotName {
    StorageSlotName::new(format!("{SLOT_PREFIX}::{field}")).expect("valid storage slot name")
}

fn serial_num() -> Word {
    Word::from([Felt::from(1_u8), Felt::from(2_u8), Felt::from(3_u8), Felt::from(4_u8)])
}

/// Deadline far in the future (u32::MAX seconds) so no note ever expires in these tests.
fn deadline() -> Felt {
    Felt::from(u32::MAX)
}

/// The two felts the note scripts pass to `AccountId::new(prefix, suffix)`.
fn account_id_felts(id: AccountId) -> [Felt; 2] {
    [id.prefix().as_felt(), id.suffix()]
}

/// Floor square root, mirroring `contracts/miden-xyk/src/utils/math.rs`.
fn isqrt(mut n: u128) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut bit = 1u128 << ((127 - n.leading_zeros()) & !1);
    let mut root = 0u128;
    while bit != 0 {
        let trial = root + bit;
        root >>= 1;
        if n >= trial {
            n -= trial;
            root += bit;
        }
        bit >>= 2;
    }
    root as u64
}

/// Swap amount-out formula, mirroring `contracts/miden-xyk/src/amm/swap.rs`.
///
/// Returns `(amount_out, protocol_fee)`.
fn amount_out(
    amount_in: u64,
    reserve_in: u64,
    reserve_out: u64,
    lp_fee_bps: u32,
    protocol_fee_bps: u32,
) -> (u64, u64) {
    let fee_denominator: u128 = 10_000;
    let protocol_fee = (protocol_fee_bps as u128 * amount_in as u128 / fee_denominator) as u64;
    let lp_amount = amount_in - protocol_fee;
    let weight = fee_denominator - lp_fee_bps as u128;
    let amount_in_with_fee = lp_amount as u128 * weight;
    let denominator = reserve_in as u128 * fee_denominator + amount_in_with_fee;
    let numerator = amount_in_with_fee * reserve_out as u128;
    let out = (numerator / denominator) as u64;
    (out, protocol_fee)
}

/// Builds every contract package once, out of process.
struct Contracts {
    miden_xyk: Arc<Package>,
    admin_note: Arc<Package>,
    mint_note: Arc<Package>,
    burn_note: Arc<Package>,
    swap_note: Arc<Package>,
    collect_note: Arc<Package>,
}

impl Contracts {
    fn build() -> Result<Self> {
        Ok(Self {
            miden_xyk: Arc::new(build_project_in_dir(Path::new("../contracts/miden-xyk"), true)?),
            admin_note: Arc::new(build_project_in_dir(Path::new("../contracts/admin-note"), true)?),
            mint_note: Arc::new(build_project_in_dir(Path::new("../contracts/mint-note"), true)?),
            burn_note: Arc::new(build_project_in_dir(Path::new("../contracts/burn-note"), true)?),
            swap_note: Arc::new(build_project_in_dir(Path::new("../contracts/swap-note"), true)?),
            collect_note: Arc::new(build_project_in_dir(
                Path::new("../contracts/collect-note"),
                true,
            )?),
        })
    }
}

/// Builds the `InitStorageData` for a fresh pool account.
///
/// `token0`/`token1` asset keys and the `admin` id are seeded at creation; the remaining slots
/// start at their zero values and are later set by the admin `configure` call.
fn pool_init_storage(token0_key: Word, token1_key: Word, admin: AccountId) -> Result<InitStorageData> {
    let mut init = InitStorageData::default();
    init.insert_value(
        StorageValueName::from_slot_name(&slot("tokenlp_asset_key")),
        Word::default(),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("token0_asset_key")),
        token0_key,
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("token1_asset_key")),
        token1_key,
    )?;
    init.insert_value(StorageValueName::from_slot_name(&slot("lp_supply")), Felt::ZERO)?;
    init.insert_value(StorageValueName::from_slot_name(&slot("fees")), Word::default())?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("admin")),
        Word::from([Felt::ZERO, Felt::ZERO, admin.suffix(), admin.prefix().as_felt()]),
    )?;
    init.insert_value(StorageValueName::from_slot_name(&slot("p2id_root")), Word::default())?;
    init.insert_value(StorageValueName::from_slot_name(&slot("protocol_pot")), Word::default())?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("price0_cumulative_last")),
        Word::default(),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("price1_cumulative_last")),
        Word::default(),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("block_timestamp_last")),
        Felt::ZERO,
    )?;
    Ok(init)
}

/// Creates the pool account component and registers it on the mock chain, returning its id.
fn add_pool_account(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    admin: AccountId,
    seed_assets: Vec<Asset>,
) -> Result<AccountId> {
    let init = pool_init_storage(token0_key, token1_key, admin)?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from miden_xyk package")?;

    let account_builder = AccountBuilder::new([4_u8; 32])
        .account_type(AccountType::Public)
        .with_component(component)
        .with_assets(seed_assets);

    let account = builder
        .add_account_from_builder(basic_auth(), account_builder, AccountState::Exists)
        .context("failed to add pool account")?;
    Ok(account.id())
}

/// Builds a note from a compiled package.
fn build_note(
    package: Arc<Package>,
    sender: AccountId,
    storage: Vec<Felt>,
    assets: Vec<Asset>,
) -> Result<Note> {
    let mut rng = rand::rng();
    NoteBuilder::new(sender, &mut rng)
        .package((*package).clone())
        .add_assets(assets)
        .note_storage(storage)
        .context("failed to set note storage")?
        .build()
        .context("failed to build note")
}

/// 14-felt storage layout for the admin note (see `contracts/admin-note/src/lib.rs`).
#[allow(clippy::too_many_arguments)]
fn admin_note_storage(
    pool: AccountId,
    fee_to: AccountId,
    lp_fee_bps: u32,
    protocol_fee_bps: u32,
    p2id_root: Word,
    lp_key: Word,
) -> Vec<Felt> {
    let mut s = Vec::with_capacity(14);
    s.extend(account_id_felts(pool)); // [0], [1]
    s.extend(account_id_felts(fee_to)); // [2], [3]
    s.push(Felt::from(lp_fee_bps)); // [4]
    s.push(Felt::from(protocol_fee_bps)); // [5]
    s.extend([p2id_root[0], p2id_root[1], p2id_root[2], p2id_root[3]]); // [6..10]
    s.extend([lp_key[0], lp_key[1], lp_key[2], lp_key[3]]); // [10..14]
    s
}

/// Common storage for the mint / swap / burn notes: pool id, recipient, serial, tag, deadline.
fn pool_note_prefix(pool: AccountId, recipient: AccountId, serial: Word, tag: Felt) -> Vec<Felt> {
    let mut s = Vec::with_capacity(10);
    s.extend(account_id_felts(pool));
    s.extend(account_id_felts(recipient));
    s.extend([serial[0], serial[1], serial[2], serial[3]]);
    s.push(tag);
    s.push(deadline());
    s
}

/// Executes a transaction in which `pool` consumes `note`, then proves a block.
async fn consume_note(mock_chain: &mut MockChain, pool: AccountId, note: &Note) -> Result<()> {
    let executed = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(note.id())
        .build()?
        .execute()
        .await?;
    mock_chain.add_pending_executed_transaction(&executed)?;
    mock_chain.prove_next_block()?;
    Ok(())
}

/// Returns the pool's committed vault balance for a fungible asset issued by `faucet`.
fn vault_balance(mock_chain: &MockChain, pool: AccountId, faucet: AccountId, amount: u64) -> Result<u64> {
    let account = mock_chain.committed_account(pool)?;
    let asset_id = FungibleAsset::new(faucet, amount)?.id();
    Ok(account.vault().get_balance(asset_id)?.as_u64())
}

/// Reads a scalar `u64` from a value storage slot (stored as `[value, 0, 0, 0]`).
fn read_scalar(mock_chain: &MockChain, pool: AccountId, field: &str) -> Result<u64> {
    let account = mock_chain.committed_account(pool)?;
    let word = account.storage().get_item(&slot(field))?;
    Ok(word[0].as_canonical_u64())
}

/// Reads a full `Word` from a value storage slot.
fn read_word(mock_chain: &MockChain, pool: AccountId, field: &str) -> Result<Word> {
    let account = mock_chain.committed_account(pool)?;
    Ok(account.storage().get_item(&slot(field))?)
}

/// Reconstructs the `u128` stored in the contract's `U128` limb layout (little-endian 32-bit limbs).
fn word_to_u128(w: Word) -> u128 {
    (w[0].as_canonical_u64() as u128)
        | ((w[1].as_canonical_u64() as u128) << 32)
        | ((w[2].as_canonical_u64() as u128) << 64)
        | ((w[3].as_canonical_u64() as u128) << 96)
}

#[tokio::test]
async fn configure_sets_pool_parameters() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    // The LP token is issued by the pool account itself.
    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, fee_to.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let account = mock_chain.committed_account(pool)?;

    // fees word = [lp_fee_bps, protocol_fee_bps, fee_to.prefix, fee_to.suffix]
    let fees = account.storage().get_item(&slot("fees"))?;
    assert_eq!(fees[0].as_canonical_u64(), LP_FEE_BPS as u64, "lp fee bps mismatch");
    assert_eq!(fees[1].as_canonical_u64(), PROTOCOL_FEE_BPS as u64, "protocol fee bps mismatch");
    assert_eq!(fees[2], fee_to.id().prefix().as_felt(), "fee_to prefix mismatch");
    assert_eq!(fees[3], fee_to.id().suffix(), "fee_to suffix mismatch");

    assert_eq!(
        account.storage().get_item(&slot("tokenlp_asset_key"))?,
        lp_key,
        "lp token key mismatch"
    );
    assert_eq!(
        account.storage().get_item(&slot("p2id_root"))?,
        p2id_root,
        "p2id root mismatch"
    );
    Ok(())
}

#[tokio::test]
async fn mint_provides_initial_liquidity() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    let (amount0, amount1) = (1_000_000_u64, 1_000_000_u64);
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
        ],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    // First mint: min_lp_out = 0 keeps the assertion non-binding.
    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO); // min_lp_out
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
        ],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let executed = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(mint_note.id())
        .build()?
        .execute()
        .await?;
    mock_chain.add_pending_executed_transaction(&executed)?;
    mock_chain.prove_next_block()?;

    // Vault holds the deposited tokens.
    assert_eq!(vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?, amount0, "token0 reserve mismatch");
    assert_eq!(vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?, amount1, "token1 reserve mismatch");

    // First mint sets lp_supply to sqrt(x*y); the LP receives sqrt(x*y) - 1000 (min liquidity).
    let expected_lp = isqrt(amount0 as u128 * amount1 as u128);
    assert_eq!(read_scalar(&mock_chain, pool, "lp_supply")?, expected_lp, "lp supply mismatch");

    // The mint produced exactly one output note (LP tokens + any refund) for the LP.
    assert_eq!(executed.output_notes().num_notes(), 1, "expected one LP output note");
    Ok(())
}

#[tokio::test]
async fn swap_updates_reserves_and_protocol_pot() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    // Pre-fund the pool so swap can run without a prior mint transaction.
    let reserve0 = 1_000_000_u64;
    let reserve1 = 1_000_000_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let amount_in = 10_000_u64;
    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, protocol_fee) =
        amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_BPS);

    // The pool adds the full input to its vault, pays out `amount_out`, and books the
    // protocol fee in the protocol pot (subtracted when reserves are derived).
    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        reserve0 + amount_in,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        reserve1 - amount_out,
        "token1 vault mismatch"
    );

    let account = mock_chain.committed_account(pool)?;
    let pot = account.storage().get_item(&slot("protocol_pot"))?;
    assert_eq!(pot[0].as_canonical_u64(), protocol_fee, "protocol pot amount0 mismatch");
    assert_eq!(pot[1].as_canonical_u64(), 0, "protocol pot amount1 mismatch");
    Ok(())
}

#[tokio::test]
async fn burn_returns_liquidity() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let amount0 = 1_000_000_u64;
    let amount1 = 1_000_000_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    // The LP supplies both tokens for the initial mint and holds LP tokens to burn.
    let burn_amount = 100_000_u64;
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
            FungibleAsset::new(pool, burn_amount)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    // Initial mint establishes reserves and lp_supply.
    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO); // min_lp_out
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
        ],
    )?;

    // Burn uses a distinct serial so its output note does not collide with the mint's.
    let burn_serial = Word::from([Felt::from(9_u8), Felt::from(8_u8), Felt::from(7_u8), Felt::from(6_u8)]);
    let mut burn_storage = pool_note_prefix(pool, lp.id(), burn_serial, Felt::ZERO);
    burn_storage.push(Felt::ZERO); // min_amount0
    burn_storage.push(Felt::ZERO); // min_amount1
    let burn_note = build_note(
        contracts.burn_note.clone(),
        lp.id(),
        burn_storage,
        vec![FungibleAsset::new(pool, burn_amount)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(burn_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &mint_note).await?;
    consume_note(&mut mock_chain, pool, &burn_note).await?;

    // After the initial mint, lp_supply = sqrt(amount0 * amount1) and reserves equal the
    // deposited amounts. Burning `burn_amount` LP returns a pro-rata share of both reserves.
    let lp_supply = isqrt(amount0 as u128 * amount1 as u128);
    let out0 = burn_amount as u128 * amount0 as u128 / lp_supply as u128;
    let out1 = burn_amount as u128 * amount1 as u128 / lp_supply as u128;

    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        amount0 - out0 as u64,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        amount1 - out1 as u64,
        "token1 vault mismatch"
    );
    assert_eq!(
        read_scalar(&mock_chain, pool, "lp_supply")?,
        lp_supply - burn_amount,
        "lp supply mismatch"
    );
    Ok(())
}

#[tokio::test]
async fn collect_drains_protocol_pot() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let reserve0 = 1_000_000_u64;
    let reserve1 = 1_000_000_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, fee_to.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let amount_in = 10_000_u64;
    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    let serial = serial_num();
    let collect_note = build_note(
        contracts.collect_note.clone(),
        admin.id(),
        vec![serial[0], serial[1], serial[2], serial[3]],
        vec![],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));
    builder.add_output_note(RawOutputNote::Full(collect_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (_, protocol_fee) = amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_BPS);
    let before = vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?;

    consume_note(&mut mock_chain, pool, &collect_note).await?;

    // The protocol fee is removed from the vault and the pot is zeroed.
    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        before - protocol_fee,
        "token0 vault mismatch after collect"
    );

    let account = mock_chain.committed_account(pool)?;
    let pot = account.storage().get_item(&slot("protocol_pot"))?;
    assert_eq!(pot[0].as_canonical_u64(), 0, "protocol pot amount0 not drained");
    assert_eq!(pot[1].as_canonical_u64(), 0, "protocol pot amount1 not drained");
    Ok(())
}

#[tokio::test]
async fn mint_before_configure_fails() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), 1_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000)?.into(),
        ],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO);
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000)?.into(),
        ],
    )?;

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    let mock_chain = builder.build()?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(mint_note.id())
        .build()?
        .execute()
        .await;

    assert!(result.is_err(), "mint before configure should fail");
    Ok(())
}

#[tokio::test]
async fn configure_rejects_non_admin() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let attacker = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    // The note is sent by `attacker`, not the configured admin.
    let note = build_note(
        contracts.admin_note.clone(),
        attacker.id(),
        admin_note_storage(pool, attacker.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    builder.add_output_note(RawOutputNote::Full(note.clone()));
    let mock_chain = builder.build()?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(note.id())
        .build()?
        .execute()
        .await;

    assert!(result.is_err(), "configure by a non-admin should fail");
    Ok(())
}

#[tokio::test]
async fn swap_large_amount_succeeds() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_u64 << 58, Some(1_u64 << 58))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_u64 << 58, Some(1_u64 << 58))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    // Reserves and swap size at half the clip: the swap numerator
    // (amount_in_with_fee * reserve_out) stays comfortably below u128::MAX.
    let reserve0 = MAX_AMOUNT / 2;
    let reserve1 = MAX_AMOUNT / 2;
    let amount_in = MAX_AMOUNT / 2;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, protocol_fee) =
        amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_BPS);

    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        reserve0 + amount_in,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        reserve1 - amount_out,
        "token1 vault mismatch"
    );

    let account = mock_chain.committed_account(pool)?;
    let pot = account.storage().get_item(&slot("protocol_pot"))?;
    assert_eq!(pot[0].as_canonical_u64(), protocol_fee, "protocol pot amount0 mismatch");
    assert_eq!(pot[1].as_canonical_u64(), 0, "protocol pot amount1 mismatch");
    Ok(())
}

#[tokio::test]
async fn swap_overflowing_reserve_reverts() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_u64 << 58, Some(1_u64 << 58))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_u64 << 58, Some(1_u64 << 58))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT / 2)?.into(),
            FungibleAsset::new(token1_faucet.id(), MAX_AMOUNT / 2)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(swap_note.id())
        .build()?
        .execute()
        .await;

    assert!(
        result.is_err(),
        "swap that pushes a reserve over MAX_AMOUNT should fail"
    );
    Ok(())
}

#[tokio::test]
async fn mint_above_max_amount_reverts() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_u64 << 58, Some(1_u64 << 58))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_u64 << 58, Some(1_u64 << 58))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT + 1)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1)?.into(),
        ],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO); // min_lp_out
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT + 1)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1)?.into(),
        ],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(mint_note.id())
        .build()?
        .execute()
        .await;

    assert!(result.is_err(), "mint above MAX_AMOUNT should fail");
    Ok(())
}

#[tokio::test]
async fn burn_below_minimum_liquidity_fails() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let amount0 = 1_000_000_u64;
    let amount1 = 1_000_000_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    // The LP supplies both tokens and holds one LP token more than the burn cap allows.
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
            FungibleAsset::new(pool, 999_001)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO); // min_lp_out
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
        ],
    )?;

    let burn_serial =
        Word::from([Felt::from(9_u8), Felt::from(8_u8), Felt::from(7_u8), Felt::from(6_u8)]);
    let mut burn_storage = pool_note_prefix(pool, lp.id(), burn_serial, Felt::ZERO);
    burn_storage.push(Felt::ZERO); // min_amount0
    burn_storage.push(Felt::ZERO); // min_amount1
    let burn_note = build_note(
        contracts.burn_note.clone(),
        lp.id(),
        burn_storage,
        vec![FungibleAsset::new(pool, 999_001)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(burn_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &mint_note).await?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(burn_note.id())
        .build()?
        .execute()
        .await;

    assert!(
        result.is_err(),
        "burning into the locked MINIMUM_LIQUIDITY should fail"
    );
    Ok(())
}

#[tokio::test]
async fn second_mint_is_pro_rata_and_refunds_excess() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    // First mint deposits 1e6/1e6; second mint deposits 200_000/100_000 (unequal ratio).
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), 1_200_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_100_000)?.into(),
        ],
    )?;

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut first_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    first_storage.push(Felt::ZERO); // min_lp_out
    let first_mint = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        first_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;

    let second_serial =
        Word::from([Felt::from(5_u8), Felt::from(6_u8), Felt::from(7_u8), Felt::from(8_u8)]);
    let mut second_storage = pool_note_prefix(pool, lp.id(), second_serial, Felt::ZERO);
    second_storage.push(Felt::ZERO); // min_lp_out
    let second_mint = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        second_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 200_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 100_000)?.into(),
        ],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(first_mint.clone()));
    builder.add_output_note(RawOutputNote::Full(second_mint.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &first_mint).await?;

    let executed = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(second_mint.id())
        .build()?
        .execute()
        .await?;
    mock_chain.add_pending_executed_transaction(&executed)?;
    mock_chain.prove_next_block()?;

    // liquidity = min(200_000 * L / 1e6, 100_000 * L / 1e6) = 100_000; 100_000 of each token used,
    // so token0 gets a 100_000 refund and both reserves and lp_supply grow by 100_000.
    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        1_100_000,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        1_100_000,
        "token1 vault mismatch"
    );
    assert_eq!(
        read_scalar(&mock_chain, pool, "lp_supply")?,
        1_100_000,
        "lp supply mismatch"
    );
    assert_eq!(executed.output_notes().num_notes(), 1, "expected one LP output note");
    Ok(())
}

#[tokio::test]
async fn swap_token1_to_token0() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token1_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let reserve0 = 1_000_000_u64;
    let reserve1 = 1_000_000_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let amount_in = 10_000_u64;
    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token1_faucet.id(), amount_in)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, protocol_fee) =
        amount_out(amount_in, reserve1, reserve0, LP_FEE_BPS, PROTOCOL_FEE_BPS);

    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        reserve0 - amount_out,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        reserve1 + amount_in,
        "token1 vault mismatch"
    );

    let account = mock_chain.committed_account(pool)?;
    let pot = account.storage().get_item(&slot("protocol_pot"))?;
    assert_eq!(pot[0].as_canonical_u64(), 0, "protocol pot amount0 mismatch");
    assert_eq!(pot[1].as_canonical_u64(), protocol_fee, "protocol pot amount1 mismatch");
    Ok(())
}

#[tokio::test]
async fn swap_with_slippage_rejected() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::new(999_999)?); // min_amount_out is way above what the pool can pay
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(swap_note.id())
        .build()?
        .execute()
        .await;

    assert!(result.is_err(), "swap below min_amount_out should fail");
    Ok(())
}

#[tokio::test]
async fn swap_without_liquidity_fails() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    // Pool is configured but holds no reserves.
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO);
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(swap_note.id())
        .build()?
        .execute()
        .await;

    assert!(result.is_err(), "swap against an empty pool should fail");
    Ok(())
}

#[tokio::test]
async fn swap_with_protocol_fee_off() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let reserve0 = 1_000_000_u64;
    let reserve1 = 1_000_000_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    // protocol_fee_bps = 0: the full fee goes to LPs and the protocol pot stays empty.
    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, 0, p2id_root, lp_key),
        vec![],
    )?;

    let amount_in = 10_000_u64;
    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO);
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, _) = amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, 0);

    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        reserve0 + amount_in,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        reserve1 - amount_out,
        "token1 vault mismatch"
    );

    let account = mock_chain.committed_account(pool)?;
    let pot = account.storage().get_item(&slot("protocol_pot"))?;
    assert_eq!(pot[0].as_canonical_u64(), 0, "protocol pot should be empty");
    assert_eq!(pot[1].as_canonical_u64(), 0, "protocol pot should be empty");
    Ok(())
}

#[tokio::test]
async fn mint_requires_both_tokens() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 1_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    // The mint note carries only token0, so amount1 is zero and the mint must revert.
    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO);
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![FungibleAsset::new(token0_faucet.id(), 1_000)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;

    let result = mock_chain
        .build_transaction(pool)
        .authenticated_input_note(mint_note.id())
        .build()?
        .execute()
        .await;

    assert!(result.is_err(), "mint with only one token should fail");
    Ok(())
}

#[tokio::test]
async fn oracle_updates_price_cumulative() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool = add_pool_account(
        &mut builder,
        contracts.miden_xyk.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_key = FungibleAsset::new(pool, 1)?.to_id_word();
    let p2id_root = Word::from(P2idNote::script_root());

    let admin_note = build_note(
        contracts.admin_note.clone(),
        admin.id(),
        admin_note_storage(pool, admin.id(), LP_FEE_BPS, PROTOCOL_FEE_BPS, p2id_root, lp_key),
        vec![],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO);
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;

    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO);
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(admin_note.clone()));
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &admin_note).await?;
    consume_note(&mut mock_chain, pool, &mint_note).await?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    // The first mint only records the timestamp (reserves were 0). The swap then accrues
    // `price * elapsed` over one 10-second block step at reserves 1e6/1e6, where price = 2^64.
    let expected = 10u128 << 64;
    let p0 = read_word(&mock_chain, pool, "price0_cumulative_last")?;
    let p1 = read_word(&mock_chain, pool, "price1_cumulative_last")?;
    assert_eq!(word_to_u128(p0), expected, "price0 cumulative mismatch");
    assert_eq!(word_to_u128(p1), expected, "price1 cumulative mismatch");
    assert_eq!(
        read_scalar(&mock_chain, pool, "block_timestamp_last")?,
        1_700_000_020,
        "block timestamp last mismatch"
    );
    Ok(())
}
