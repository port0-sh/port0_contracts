//! Integration tests for the UniSwap V2 constant-product AMM (`port0`) contract.
//!
//! These tests exercise the pool end-to-end through MockChain: pool creation with
//! the registry pointer and fee rates as initial storage, liquidity provision (mint),
//! swaps, liquidity removal (burn), protocol fee collection (collect), and registration.
//! The pool account issues its own LP token, so the LP token's
//! asset key is derived from the pool account id.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result};
use integration::helpers::build_project_in_dir;
use miden_client::{
    account::{
        component::{InitStorageData, StorageValueName},
        StorageMapKey,
        Account, AccountBuilder, AccountComponent, AccountId, AccountType, StorageSlotName,
    },
    asset::{Asset, FungibleAsset},
    auth::AuthSchemeId,
    note::Note,
    transaction::RawOutputNote,
    Felt, Word,
};
use miden_mast_package::Package;
use miden_protocol::asset::{AssetAmount, AssetId, TokenSymbol};
use miden_protocol::block::FeeParameters;
use miden_protocol::note::{NoteScript, NoteScriptRoot};
use miden_protocol::testing::account_id::ACCOUNT_ID_FEE_FAUCET;
use miden_standards::{
    account::auth::{AuthNetworkAccount, SponsorshipPolicy},
    account::faucets::{FungibleFaucet, TokenName},
    account::fees::{BasicConstantFeePolicy, FeePolicyManager},
    account::inspection::CodeInspection,
    note::costs::NoteCost,
    note::P2idNote,
    testing::note::NoteBuilder,
};
use miden_testing::{AccountState, Auth, MockChain, MockChainBuilder, MockChainNote};
use miden_tx::NetworkNotePricer;

/// Storage slot prefix shared by every `port0` component slot.
///
/// Slot names are `<package_name>::<interface_segment>::<field_name>`; the package name is
/// `port0` (from `miden-project.toml`) and the interface segment is `port0` (from the
/// `miden:port0/port0@0.1.0` namespace), both sanitized to `port0`.
const SLOT_PREFIX: &str = "port0::port0";

// Fee configuration (basis points), kept well below the 10000 combined cap.
/// LP fee tiers the registry lists, in basis points.
const LP_FEE_TIERS: [u32; 3] = [5, 30, 100];

const LP_FEE_BPS: u32 = 30;
const PROTOCOL_FEE_SHARE_BPS: u32 = 1200;

/// Mirrors `contracts/port0/src/utils/math.rs::MAX_AMOUNT` (the u128-safe clip).
const MAX_AMOUNT: u64 = 1 << 57;

fn basic_auth() -> Auth {
    Auth::BasicAuth {
        auth_scheme: AuthSchemeId::Falcon512Poseidon2,
    }
}

fn slot(field: &str) -> StorageSlotName {
    StorageSlotName::new(format!("{SLOT_PREFIX}::{field}")).expect("valid storage slot name")
}

/// Storage slot prefix shared by every `pool-registry` component slot.
const REGISTRY_SLOT_PREFIX: &str = "pool_registry::pool_registry";

fn registry_slot(field: &str) -> StorageSlotName {
    StorageSlotName::new(format!("{REGISTRY_SLOT_PREFIX}::{field}")).expect("valid storage slot name")
}

/// Seeds the registry's value slots; its three maps start empty.
fn registry_init_storage(
    protocol_fee_to: AccountId,
    protocol_fee_share_bps: u32,
    accepted: Word,
    expected_auth: [Word; 6],
    register_note_root: Word,
    admin: AccountId,
) -> Result<InitStorageData> {
    let mut init = InitStorageData::default();
    init.insert_map_entry(
        registry_slot("accepted_code_commitments").as_str(),
        accepted,
        Felt::ONE,
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("protocol_fee_to")),
        Word::from([
            Felt::ZERO,
            Felt::ZERO,
            protocol_fee_to.suffix(),
            protocol_fee_to.prefix().as_felt(),
        ]),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("protocol_fee_share_bps")),
        Felt::from(protocol_fee_share_bps),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("p2id_root")),
        Word::from(P2idNote::script_root()),
    )?;
    for tier in LP_FEE_TIERS {
        init.insert_map_entry(
            registry_slot("allowed_lp_fee_bps").as_str(),
            Felt::from(tier),
            Felt::ONE,
        )?;
    }
    // The pool auth roots the registry compares every registration against. A
    // registry seeded with zeros accepts nothing, so a deploy that forgets them
    // fails closed.
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("expected_note_allowlist_root")),
        expected_auth[0],
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("expected_tx_script_allowlist_root")),
        expected_auth[1],
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("expected_fee_schedule_root")),
        expected_auth[2],
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("admin")),
        Word::from([Felt::ZERO, Felt::ZERO, admin.suffix(), admin.prefix().as_felt()]),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("register_note_root")),
        register_note_root,
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("expected_fee_asset_id")),
        expected_auth[3],
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("expected_active_fee_policy_root")),
        expected_auth[4],
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&registry_slot("expected_allowed_fee_policy_roots")),
        expected_auth[5],
    )?;
    Ok(init)
}

/// The code commitment of every pool these helpers build.
///
/// It depends on the component set, not on storage or the seed, so a throwaway
/// build with placeholder ids yields the value the registry must accept.
fn pool_code_commitment(
    builder: &mut MockChainBuilder,
    contracts: &Contracts,
    token0_key: Word,
    token1_key: Word,
    admin: AccountId,
) -> Result<Word> {
    let init = pool_init_storage_with_registry(
        token0_key,
        token1_key,
        admin,
        admin,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (0, 0),
    )?;
    let component = AccountComponent::from_package(contracts.port0.as_ref().clone(), &init)
        .context("failed to build pool component for the commitment")?;
    // Built through the chain builder so it carries the mock's auth component.
    let account_builder = AccountBuilder::new([8_u8; 32])
        .account_type(AccountType::Public)
        .with_component(component)
        .with_component(CodeInspection::from(CodeInspection));
    let account = builder
        .add_account_from_builder(basic_auth(), account_builder, AccountState::Exists)
        .context("failed to build the throwaway pool")?;
    Ok(account.code().commitment())
}

/// Creates the registry account and registers it on the mock chain.
fn add_registry_account(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    protocol_fee_to: AccountId,
    protocol_fee_share_bps: u32,
    accepted: Word,
    expected_auth: [Word; 6],
    register_note_root: Word,
) -> Result<AccountId> {
    add_registry_account_with_assets(
        builder,
        package,
        protocol_fee_to,
        protocol_fee_share_bps,
        accepted,
        expected_auth,
        register_note_root,
        vec![],
    )
}

/// Creates a registry holding `seed_assets`, which its `collect` pays out.
fn add_registry_account_with_assets(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    protocol_fee_to: AccountId,
    protocol_fee_share_bps: u32,
    accepted: Word,
    expected_auth: [Word; 6],
    register_note_root: Word,
    seed_assets: Vec<Asset>,
) -> Result<AccountId> {
    let init = registry_init_storage(
        protocol_fee_to,
        protocol_fee_share_bps,
        accepted,
        expected_auth,
        register_note_root,
        protocol_fee_to,
    )?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from pool-registry package")?;
    let account_builder = AccountBuilder::new([6_u8; 32])
        .account_type(AccountType::Public)
        .with_component(component)
        .with_assets(seed_assets);
    let account = builder
        .add_account_from_builder(basic_auth(), account_builder, AccountState::Exists)
        .context("failed to add registry account")?;
    Ok(account.id())
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

/// Floor square root, mirroring `contracts/port0/src/utils/math.rs`.
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

/// Swap amount-out formula, mirroring `contracts/port0/src/amm/swap.rs`.
///
/// Returns `(amount_out, protocol_fee)`.
fn amount_out(
    amount_in: u64,
    reserve_in: u64,
    reserve_out: u64,
    lp_fee_bps: u32,
    protocol_fee_share_bps: u32,
) -> (u64, u64) {
    // Mirrors `amm::swap::amount_out`: the protocol's share is carved out of the
    // LP fee, and the output does not depend on it.
    let fee_denominator: u128 = 10_000;
    let lp_fee = (lp_fee_bps as u128 * amount_in as u128 / fee_denominator) as u64;
    let protocol_fee = (protocol_fee_share_bps as u128 * lp_fee as u128 / fee_denominator) as u64;
    let weight = fee_denominator - lp_fee_bps as u128;
    let amount_in_with_fee = amount_in as u128 * weight;
    let denominator = reserve_in as u128 * fee_denominator + amount_in_with_fee;
    let numerator = amount_in_with_fee * reserve_out as u128;
    let out = (numerator / denominator) as u64;
    (out, protocol_fee)
}

/// Builds every contract package once, out of process.
struct Contracts {
    network_faucet: Arc<Package>,
    launch_note: Arc<Package>,
    port0: Arc<Package>,
    registry: Arc<Package>,
    register_note: Arc<Package>,
    registry_collect_note: Arc<Package>,
    registry_config_note: Arc<Package>,
    mint_note: Arc<Package>,
    burn_note: Arc<Package>,
    swap_note: Arc<Package>,
    collect_note: Arc<Package>,
}

impl Contracts {
    fn build() -> Result<Self> {
        Ok(Self {
            network_faucet: Arc::new(build_project_in_dir(
                Path::new("../contracts/network-faucet"),
                true,
            )?),
            launch_note: Arc::new(build_project_in_dir(Path::new("../contracts/launch-note"), true)?),
            port0: Arc::new(build_project_in_dir(Path::new("../contracts/port0"), true)?),
            registry: Arc::new(build_project_in_dir(Path::new("../contracts/pool-registry"), true)?),
            register_note: Arc::new(build_project_in_dir(Path::new("../contracts/register-note"), true)?),
            registry_collect_note: Arc::new(build_project_in_dir(
                Path::new("../contracts/registry-collect-note"),
                true,
            )?),
            registry_config_note: Arc::new(build_project_in_dir(
                Path::new("../contracts/registry-config-note"),
                true,
            )?),
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

/// Builds the `InitStorageData` for a pool that points at `registry_id`.
/** The standard pool seed: the payout root is the standard P2ID script. */
fn pool_init_storage_with_registry(
    token0_key: Word,
    token1_key: Word,
    admin: AccountId,
    registry_id: AccountId,
    lp_fee_bps: u32,
    protocol_fee_share_bps: u32,
    initial_reserves: (u64, u64),
) -> Result<InitStorageData> {
    pool_init_storage_full(
        token0_key,
        token1_key,
        admin,
        registry_id,
        lp_fee_bps,
        protocol_fee_share_bps,
        initial_reserves,
        Word::from(P2idNote::script_root()),
    )
}

/// The pool seed with a caller-chosen payout script root.
#[allow(clippy::too_many_arguments)]
fn pool_init_storage_full(
    token0_key: Word,
    token1_key: Word,
    admin: AccountId,
    registry_id: AccountId,
    lp_fee_bps: u32,
    protocol_fee_share_bps: u32,
    initial_reserves: (u64, u64),
    p2id_root: Word,
) -> Result<InitStorageData> {
    let mut init = InitStorageData::default();
    init.insert_value(
        StorageValueName::from_slot_name(&slot("token0_asset_key")),
        token0_key,
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("token1_asset_key")),
        token1_key,
    )?;
    init.insert_value(StorageValueName::from_slot_name(&slot("lp_supply")), Felt::ZERO)?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("reserves")),
        Word::from([
            Felt::new_unchecked(initial_reserves.0),
            Felt::new_unchecked(initial_reserves.1),
            Felt::ZERO,
            Felt::ZERO,
        ]),
    )?;
    // Seeded at creation: the LP token key is not stored, because the contract
    // derives it from the pool account id.
    init.insert_value(
        StorageValueName::from_slot_name(&slot("fees")),
        Word::from([
            Felt::from(lp_fee_bps),
            Felt::from(protocol_fee_share_bps),
            Felt::ZERO,
            Felt::ZERO,
        ]),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(&slot("p2id_root")),
        p2id_root,
    )?;
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
    init.insert_value(
        StorageValueName::from_slot_name(&slot("registry_id")),
        Word::from([
            Felt::ZERO,
            Felt::ZERO,
            registry_id.suffix(),
            registry_id.prefix().as_felt(),
        ]),
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
    add_pool_account_with_registry(
        builder,
        package,
        token0_key,
        token1_key,
        admin,
        admin,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        seed_assets,
    )
}

/// Creates a pool that points at `registry_id`, which its `collect` reads.
#[allow(clippy::too_many_arguments)]
fn add_pool_account_with_registry(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    admin: AccountId,
    registry_id: AccountId,
    lp_fee_bps: u32,
    protocol_fee_share_bps: u32,
    seed_assets: Vec<Asset>,
) -> Result<AccountId> {
    // A pre-funded pool records the token0 and token1 seed amounts as its stored reserves.
    let (reserve0, reserve1) = seed_reserves(token0_key, token1_key, &seed_assets);
    add_pool_account_with_reserves(
        builder,
        package,
        token0_key,
        token1_key,
        admin,
        registry_id,
        lp_fee_bps,
        protocol_fee_share_bps,
        (reserve0, reserve1),
        seed_assets,
    )
}

/// The stored reserves a pre-funded pool starts with: its token0 and token1 seed amounts.
fn seed_reserves(token0_key: Word, token1_key: Word, seed_assets: &[Asset]) -> (u64, u64) {
    let mut reserve0 = 0u64;
    let mut reserve1 = 0u64;
    for asset in seed_assets {
        if let Some(fungible) = asset.as_fungible() {
            if fungible.to_id_word() == token0_key {
                reserve0 = reserve0.checked_add(fungible.amount().as_u64()).unwrap();
            } else if fungible.to_id_word() == token1_key {
                reserve1 = reserve1.checked_add(fungible.amount().as_u64()).unwrap();
            }
        }
    }
    (reserve0, reserve1)
}

/// Creates a pool with explicit stored reserves and extra vault credits.
#[allow(clippy::too_many_arguments)]
fn add_pool_account_with_reserves(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    admin: AccountId,
    registry_id: AccountId,
    lp_fee_bps: u32,
    protocol_fee_share_bps: u32,
    initial_reserves: (u64, u64),
    seed_assets: Vec<Asset>,
) -> Result<AccountId> {
    let init = pool_init_storage_with_registry(
        token0_key,
        token1_key,
        admin,
        registry_id,
        lp_fee_bps,
        protocol_fee_share_bps,
        initial_reserves,
    )?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from port0 package")?;

    let account_builder = AccountBuilder::new([4_u8; 32])
        .account_type(AccountType::Public)
        .with_component(component)
        .with_component(CodeInspection::from(CodeInspection))
        .with_assets(seed_assets);

    let account = builder
        .add_account_from_builder(basic_auth(), account_builder, AccountState::Exists)
        .context("failed to add pool account")?;
    Ok(account.id())
}

/// A network account in its production shape: the given note roots allowlisted
/// and a fee policy, so a benchmark measures the cycles the node charges for
/// consuming a note.
fn build_network_account(
    builder: &mut MockChainBuilder,
    seed: [u8; 32],
    component: AccountComponent,
    seed_assets: Vec<Asset>,
    allowed_roots: BTreeSet<NoteScriptRoot>,
) -> Result<Account> {
    // A constant policy aborts fee estimation for a root without an entry, so
    // schedule a zero fee for every allowlisted root. A benchmark measures the
    // epilogue fee, not the schedule price.
    let mut policy = BasicConstantFeePolicy::new();
    for root in allowed_roots
        .iter()
        .chain(AuthNetworkAccount::default_allowed_note_scripts().iter())
    {
        policy = policy.with_fee(*root, AssetAmount::ZERO);
    }
    let fee_policy_manager = FeePolicyManager::builder()
        .active_fee_policy(policy.into())
        .fee_faucet_id(ACCOUNT_ID_FEE_FAUCET.try_into().expect("valid fee faucet id"))
        .build();

    let account_builder = AccountBuilder::new(seed)
        .account_type(AccountType::Public)
        .with_component(component)
        .with_component(CodeInspection::from(CodeInspection))
        .with_assets(seed_assets);

    let auth = Auth::NetworkAccount {
        allowed_script_roots: allowed_roots,
        allowed_tx_script_roots: BTreeSet::new(),
        fee_policy_manager,
        sponsorship_policy: SponsorshipPolicy::default(),
    };

    builder
        .add_account_from_builder(auth, account_builder, AccountState::Exists)
        .context("failed to add network account")
}

/// A pool account in its production shape (network auth).
#[allow(clippy::too_many_arguments)]
fn add_pool_account_network(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    registry_id: AccountId,
    initial_reserves: (u64, u64),
    seed_assets: Vec<Asset>,
    allowed_roots: BTreeSet<NoteScriptRoot>,
) -> Result<AccountId> {
    add_pool_account_network_with_fees(
        builder,
        package,
        token0_key,
        token1_key,
        registry_id,
        PROTOCOL_FEE_SHARE_BPS,
        initial_reserves,
        seed_assets,
        allowed_roots,
    )
}

/// A pool with the accepted components but a payout script root of its choosing.
fn add_pool_account_network_with_p2id(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    registry_id: AccountId,
    p2id_root: Word,
    allowed_roots: BTreeSet<NoteScriptRoot>,
) -> Result<AccountId> {
    let init = pool_init_storage_full(
        token0_key,
        token1_key,
        registry_id,
        registry_id,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (0, 0),
        p2id_root,
    )?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from port0 package")?;
    let account = build_network_account(builder, [7_u8; 32], component, vec![], allowed_roots)?;
    Ok(account.id())
}

/// The same pool with an explicit protocol fee rate, for the registry's rate
/// check.
#[allow(clippy::too_many_arguments)]
fn add_pool_account_network_with_fees(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    registry_id: AccountId,
    protocol_fee_share_bps: u32,
    initial_reserves: (u64, u64),
    seed_assets: Vec<Asset>,
    allowed_roots: BTreeSet<NoteScriptRoot>,
) -> Result<AccountId> {
    let init = pool_init_storage_with_registry(
        token0_key,
        token1_key,
        registry_id,
        registry_id,
        LP_FEE_BPS,
        protocol_fee_share_bps,
        initial_reserves,
    )?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from port0 package")?;
    let account = build_network_account(builder, [7_u8; 32], component, seed_assets, allowed_roots)?;
    Ok(account.id())
}

/// The code commitment of the network-auth pool, which is what the registry's
/// accepted-commitments map must hold. It depends on the component set, not on
/// storage or the seed.
/// What the registry must accept and require for production pools: the code
/// commitment of the network-auth pool, and that pool's auth roots.
struct PoolNetworkFixture {
    commitment: Word,
    auth_roots: [Word; 6],
}

/// The storage items the pool's auth identity exposes, read from a built
/// account: the two allowlist roots, the fee schedule root, then the fee asset
/// and the two policy roots.
fn pool_auth_roots_of(account: &Account) -> Result<[Word; 6]> {
    let storage = account.storage();
    let mut roots = [Word::default(); 6];
    for (index, name) in POOL_AUTH_SLOTS.iter().enumerate() {
        let slot = StorageSlotName::new(*name).context("invalid auth slot name")?;
        let item = storage
            .get_item(&slot)
            .with_context(|| format!("pool has no {name} storage"))?;
        roots[index] = Word::from(*item);
    }
    Ok(roots)
}

/// The slot names of the pool's auth component, in the order `get_auth_identity`
/// returns them.
const POOL_AUTH_SLOTS: [&str; 6] = [
    "miden::standards::auth::network_account::allowed_note_scripts",
    "miden::standards::auth::network_account::allowed_tx_scripts",
    "miden::standards::fees::policies::basic_constant_fee::fee_schedule",
    "miden::standards::auth::network_account::fee_asset_id",
    "miden::standards::auth::network_account::active_fee_policy_proc_root",
    "miden::standards::auth::network_account::allowed_fee_policy_proc_roots",
];

fn pool_code_commitment_network(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    token0_key: Word,
    token1_key: Word,
    placeholder_registry: AccountId,
    allowed_roots: &BTreeSet<NoteScriptRoot>,
) -> Result<PoolNetworkFixture> {
    let init = pool_init_storage_with_registry(
        token0_key,
        token1_key,
        placeholder_registry,
        placeholder_registry,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (0, 0),
    )?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from port0 package")?;
    let account =
        build_network_account(builder, [8_u8; 32], component, vec![], allowed_roots.clone())?;
    Ok(PoolNetworkFixture {
        commitment: account.code().commitment(),
        auth_roots: pool_auth_roots_of(&account)?,
    })
}

/// The register note's script root, which the registry stores so the admin's fee
/// setter can only move that one entry.
fn register_note_root(contracts: &Contracts) -> Result<Word> {
    Ok(Word::from(NoteScript::from_package(
        contracts.register_note.as_ref(),
    )?.root()))
}

/// Auth roots for a registry that is never asked to register a pool: zeros are
/// the fail-closed value, since no real pool carries a zero allowlist root.
fn zero_auth_roots() -> [Word; 6] {
    [Word::from([Felt::ZERO, Felt::ZERO, Felt::ZERO, Felt::ZERO]); 6]
}

/// The script root of a compiled note package.
fn note_root(package: &Arc<Package>) -> Result<NoteScriptRoot> {
    Ok(NoteScript::from_package(package.as_ref())?.root())
}

/// The note roots a production pool admits (its own four note scripts; the
/// network auth adds the protocol defaults itself).
fn pool_note_roots(contracts: &Contracts) -> Result<BTreeSet<NoteScriptRoot>> {
    Ok([
        note_root(&contracts.mint_note)?,
        note_root(&contracts.burn_note)?,
        note_root(&contracts.swap_note)?,
        note_root(&contracts.collect_note)?,
    ]
    .into_iter()
    .collect())
}

/// The note roots a production registry admits.
fn registry_note_roots(contracts: &Contracts) -> Result<BTreeSet<NoteScriptRoot>> {
    Ok([
        note_root(&contracts.register_note)?,
        note_root(&contracts.registry_collect_note)?,
        note_root(&contracts.registry_config_note)?,
    ]
    .into_iter()
    .collect())
}

/// A registry account in its production shape (network auth).
#[allow(clippy::too_many_arguments)]
fn add_registry_account_network(
    builder: &mut MockChainBuilder,
    package: Arc<Package>,
    protocol_fee_to: AccountId,
    protocol_fee_share_bps: u32,
    accepted: Word,
    expected_auth: [Word; 6],
    register_note_root: Word,
    seed_assets: Vec<Asset>,
    allowed_roots: BTreeSet<NoteScriptRoot>,
) -> Result<AccountId> {
    let init = registry_init_storage(
        protocol_fee_to,
        protocol_fee_share_bps,
        accepted,
        expected_auth,
        register_note_root,
        protocol_fee_to,
    )?;
    let component = AccountComponent::from_package(package.as_ref().clone(), &init)
        .context("failed to build account component from pool-registry package")?;
    let account = build_network_account(builder, [5_u8; 32], component, seed_assets, allowed_roots)?;
    Ok(account.id())
}

/// Executes `account` consuming `note`, proves a block, and returns the total
/// cycles the transaction executed. `foreign` is declared when the script calls
/// it.
async fn bench_consume(
    chain: &mut MockChain,
    account: AccountId,
    note: &Note,
    foreign: Option<AccountId>,
) -> Result<usize> {
    let mut tx = chain.build_transaction(account).authenticated_input_note(note.id());
    if let Some(foreign_id) = foreign {
        tx = tx.foreign_accounts(vec![chain.get_foreign_account_inputs(foreign_id)?]);
    }
    let executed = tx.build()?.execute().await?;
    let cycles = executed.measurements().total_cycles();
    chain.add_pending_executed_transaction(&executed)?;
    chain.prove_next_block()?;
    Ok(cycles)
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

/// Storage for the register note: registry, pool, then the two assets.
fn register_note_storage(
    registry: AccountId,
    pool: AccountId,
    asset_0: AccountId,
    asset_1: AccountId,
) -> Vec<Felt> {
    let mut s = Vec::with_capacity(8);
    s.extend(account_id_felts(registry));
    s.extend(account_id_felts(pool));
    s.extend(account_id_felts(asset_0));
    s.extend(account_id_felts(asset_1));
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

/// Consumes a note with `foreign` declared, which a foreign procedure call needs.
async fn consume_note_with_foreign(
    mock_chain: &mut MockChain,
    account: AccountId,
    note: &Note,
    foreign: AccountId,
) -> Result<()> {
    let foreign_inputs = mock_chain.get_foreign_account_inputs(foreign)?;
    let executed = mock_chain
        .build_transaction(account)
        .authenticated_input_note(note.id())
        .foreign_accounts(vec![foreign_inputs])
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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
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

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;

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

    // The LP asset must be issued by the pool, whose asset key is derived from
    // its own account id. This is what `configure` used to assert, kept end to
    // end now that nothing writes the key.
    let lp_asset = mock_chain
        .committed_notes()
        .values()
        .filter_map(|note| match note {
            MockChainNote::Public(note, _) => Some(note),
            _ => None,
        })
        .flat_map(|note| note.assets().iter())
        .filter_map(|asset| asset.as_fungible())
        .find(|asset| asset.faucet_id() == pool)
        .context("mint produced no asset issued by the pool")?;
    assert_eq!(
        lp_asset.to_id_word(),
        FungibleAsset::new(pool, 1)?.to_id_word(),
        "LP asset key is not the one derived from the pool account id"
    );

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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
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

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, protocol_fee) =
        amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_SHARE_BPS);

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
        contracts.port0.clone(),
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

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(burn_note.clone()));

    let mut mock_chain = builder.build()?;
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
async fn collect_pays_registry_designated_recipient() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let registry_fee_to = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let accepted = pool_code_commitment(&mut builder, &contracts, token0_key, token1_key, admin.id())?;
    let registry = add_registry_account(
        &mut builder,
        contracts.registry.clone(),
        registry_fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        accepted,
        zero_auth_roots(),
        register_note_root(&contracts)?,
    )?;

    let reserve0 = 1_000_000_u64;
    let reserve1 = 1_000_000_u64;
    let pool = add_pool_account_with_registry(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        registry,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;

    let p2id_root = Word::from(P2idNote::script_root());

    // The pool's own recipient is a decoy: collect must not use it.

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
    // Layout: pool id, serial, then one key per asset the collect sweeps.
    let mut collect_storage = account_id_felts(pool).to_vec();
    collect_storage.extend([serial[0], serial[1], serial[2], serial[3]]);
    collect_storage.extend([token0_key[0], token0_key[1], token0_key[2], token0_key[3]]);
    collect_storage.extend([token1_key[0], token1_key[1], token1_key[2], token1_key[3]]);
    let collect_note = build_note(
        contracts.collect_note.clone(),
        admin.id(),
        collect_storage,
        vec![],
    )?;

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));
    builder.add_output_note(RawOutputNote::Full(collect_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (_, protocol_fee) = amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_SHARE_BPS);
    let before = vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?;
    // Only the payout the collect creates is of interest; the chain already
    // holds notes of its own.
    let notes_before: BTreeSet<_> = mock_chain.committed_notes().keys().cloned().collect();

    consume_note_with_foreign(&mut mock_chain, pool, &collect_note, registry).await?;

    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        before - protocol_fee,
        "token0 vault mismatch after collect"
    );

    // The payout note must name the registry's recipient in its inputs.
    let payout = mock_chain
        .committed_notes()
        .iter()
        .filter(|(id, _)| !notes_before.contains(*id))
        .find_map(|(_, note)| match note {
            MockChainNote::Public(note, _)
                if Word::from(note.recipient().script().root()) == p2id_root =>
            {
                Some(note.clone())
            },
            _ => None,
        })
        .context("collect created no P2ID payout note")?;
    let inputs = payout.recipient().storage().items();
    assert_eq!(
        inputs[0],
        registry_fee_to.id().suffix(),
        "payout recipient suffix is not the registry's"
    );
    assert_eq!(
        inputs[1],
        registry_fee_to.id().prefix().as_felt(),
        "payout recipient prefix is not the registry's"
    );

    // The pot is drained by the payout.
    let account = mock_chain.committed_account(pool)?;
    let pot = account.storage().get_item(&slot("protocol_pot"))?;
    assert_eq!(pot[0].as_canonical_u64(), 0, "protocol pot amount0 not drained");
    assert_eq!(pot[1].as_canonical_u64(), 0, "protocol pot amount1 not drained");
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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;



    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, protocol_fee) =
        amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_SHARE_BPS);

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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT / 2)?.into(),
            FungibleAsset::new(token1_faucet.id(), MAX_AMOUNT / 2)?.into(),
        ],
    )?;



    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), MAX_AMOUNT)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;

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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
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

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;

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
        contracts.port0.clone(),
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

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(burn_note.clone()));

    let mut mock_chain = builder.build()?;
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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
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

    builder.add_output_note(RawOutputNote::Full(first_mint.clone()));
    builder.add_output_note(RawOutputNote::Full(second_mint.clone()));

    let mut mock_chain = builder.build()?;
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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
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

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (amount_out, protocol_fee) =
        amount_out(amount_in, reserve1, reserve0, LP_FEE_BPS, PROTOCOL_FEE_SHARE_BPS);

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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;



    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::new(999_999)?); // min_amount_out is way above what the pool can pay
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;

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

    // Pool holds no reserves.
    let pool = add_pool_account(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
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

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;

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
    let pool = add_pool_account_with_registry(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        admin.id(),
        LP_FEE_BPS,
        0,
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1)?.into(),
        ],
    )?;


    // protocol_fee_share_bps = 0: the full fee goes to LPs and the protocol pot stays empty.

    let amount_in = 10_000_u64;
    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO);
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), amount_in)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
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
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
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

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;

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
        contracts.port0.clone(),
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

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
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
        1_700_000_010,
        "block timestamp last mismatch"
    );
    Ok(())
}

/// A pool built for this registry, holding the pair it claims, at the fee rates
/// this registry sanctions, is listed.
#[tokio::test]
async fn register_lists_a_matching_pool() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool_roots = pool_note_roots(&contracts)?;
    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        vec![],
        registry_note_roots(&contracts)?,
    )?;
    let pool = add_pool_account_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        (0, 0),
        vec![],
        pool_roots,
    )?;

    let register_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, pool, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(register_note.clone()));

    let mut mock_chain = builder.build()?;
    // The registry reads the pool by foreign call, so the pool is declared.
    consume_note_with_foreign(&mut mock_chain, registry, &register_note, pool).await?;
    Ok(())
}

/// A pool charging a protocol fee the registry does not sanction is rejected.
#[tokio::test]
async fn register_rejects_a_pool_with_a_foreign_fee_rate() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool_roots = pool_note_roots(&contracts)?;
    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        vec![],
        registry_note_roots(&contracts)?,
    )?;
    // One basis point off the registry's rate; the auth roots still match.
    let pool = add_pool_account_network_with_fees(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        PROTOCOL_FEE_SHARE_BPS + 1,
        (0, 0),
        vec![],
        pool_roots,
    )?;

    let register_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, pool, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(register_note.clone()));

    let mut mock_chain = builder.build()?;
    let result = consume_note_with_foreign(&mut mock_chain, registry, &register_note, pool).await;
    assert!(
        result.is_err(),
        "registration must fail when the pool's protocol fee differs"
    );
    Ok(())
}

// The collect reserves the contracts enforce, from the shared source. Including
// the same file the contracts include is what makes the assertion below able to
// fail when a contract is short.
include!("../../contracts/collect-fee-hold.rs");

/// The kernel's transaction fee for a cycle count, in base units at
/// `verification_base_fee` 7.
///
/// The kernel rounds the cycle count up to the next power of two and charges
/// `base_fee * (log2 + 1)`, so floor-log2 understates the fee by one bucket
/// whenever the count is not an exact power of two.
fn kernel_fee(cycles: u32) -> u64 {
    let rounded = (cycles as u64).next_power_of_two();
    let log2 = 64 - rounded.leading_zeros();
    7 * u64::from(log2)
}

/// The registry's scheduled price for a note script root, read from its auth
/// component's fee schedule.
fn registry_fee_schedule(
    chain: &MockChain,
    registry: AccountId,
    script: &Arc<Package>,
) -> Result<u64> {
    let account = chain.committed_account(registry)?;
    let storage = account.storage();
    let slot = StorageSlotName::new(
        "miden::standards::fees::policies::basic_constant_fee::fee_schedule",
    )?;
    let root = Word::from(NoteScript::from_package(script.as_ref())?.root());
    let entry = storage
        .get_map_item(&slot, StorageMapKey::new(root))
        .context("the register root has no schedule entry")?;
    Ok(entry[0].as_canonical_u64())
}

/// The admin can raise the register fee, and the registry's own schedule is what
/// changes: the fee is a schedule entry, not a separate copy.
#[tokio::test]
async fn registry_admin_sets_the_register_fee() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let fee_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "USDCX", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let registry_roots = registry_note_roots(&contracts)?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        Word::default(),
        zero_auth_roots(),
        register_note_root(&contracts)?,
        vec![FungibleAsset::new(fee_faucet.id(), 10_000)?.into()],
        registry_roots,
    )?;

    let new_fee = 1_000_000_u64;
    // The register note's root is what the fee schedule is keyed by, and it moves
    // whenever the registry's interface changes, so it comes from the build.
    let register_root = register_note_root(&contracts)?;
    // `add_registry_account_network` makes the protocol fee recipient the admin,
    // so that is the account whose config note the registry accepts.
    let config_note = build_note(
        contracts.registry_config_note.clone(),
        fee_to.id(),
        vec![
            registry.prefix().as_felt(),
            registry.suffix(),
            Felt::new_unchecked(new_fee),
            register_root[0],
            register_root[1],
            register_root[2],
            register_root[3],
        ],
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(config_note.clone()));

    let mut chain = builder.build()?;
    consume_note(&mut chain, registry, &config_note).await?;

    assert_eq!(
        registry_fee_schedule(&chain, registry, &contracts.register_note)?,
        new_fee,
        "the register fee must be the value the admin set"
    );
    Ok(())
}

/// A config note from anyone but the admin is rejected, so the price cannot be
/// changed by a passer-by.
#[tokio::test]
async fn registry_rejects_a_foreign_fee_change() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let fee_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "USDCX", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let stranger = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        Word::default(),
        zero_auth_roots(),
        register_note_root(&contracts)?,
        vec![FungibleAsset::new(fee_faucet.id(), 10_000)?.into()],
        registry_note_roots(&contracts)?,
    )?;

    let register_root = register_note_root(&contracts)?;
    let config_note = build_note(
        contracts.registry_config_note.clone(),
        stranger.id(),
        vec![
            registry.prefix().as_felt(),
            registry.suffix(),
            Felt::new_unchecked(42),
            register_root[0],
            register_root[1],
            register_root[2],
            register_root[3],
        ],
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(config_note.clone()));

    let mut chain = builder.build()?;
    let result = consume_note(&mut chain, registry, &config_note).await;
    assert!(
        result.is_err(),
        "a config note from a non-admin must be rejected"
    );
    Ok(())
}

/// A pool that would charge its note fees in an asset other than the chain's
/// fee asset is rejected. The fee policy runs in the pool's context during fee
/// estimation, so an unpinned value leaves a listed pool that no note can be
/// consumed against.
#[tokio::test]
async fn register_rejects_a_pool_with_a_foreign_fee_asset() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool_roots = pool_note_roots(&contracts)?;
    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        vec![],
        registry_note_roots(&contracts)?,
    )?;

    // The pool carries the accepted components but charges in ALPHA instead of
    // the chain fee asset.
    let init = pool_init_storage_full(
        token0_key,
        token1_key,
        registry,
        registry,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (0, 0),
        Word::from(P2idNote::script_root()),
    )?;
    let component = AccountComponent::from_package(contracts.port0.as_ref().clone(), &init)
        .context("failed to build account component from port0 package")?;
    let mut policy = BasicConstantFeePolicy::new();
    for root in pool_roots
        .iter()
        .chain(AuthNetworkAccount::default_allowed_note_scripts().iter())
    {
        policy = policy.with_fee(*root, AssetAmount::ZERO);
    }
    let fee_policy_manager = FeePolicyManager::builder()
        .active_fee_policy(policy.into())
        .fee_faucet_id(token0_faucet.id())
        .build();
    let account_builder = AccountBuilder::new([7_u8; 32])
        .account_type(AccountType::Public)
        .with_component(component)
        .with_component(CodeInspection::from(CodeInspection));
    let auth = Auth::NetworkAccount {
        allowed_script_roots: pool_roots,
        allowed_tx_script_roots: BTreeSet::new(),
        fee_policy_manager,
        sponsorship_policy: SponsorshipPolicy::default(),
    };
    let pool = builder
        .add_account_from_builder(auth, account_builder, AccountState::Exists)
        .context("failed to add the hostile pool")?
        .id();

    let register_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, pool, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(register_note.clone()));

    let mut mock_chain = builder.build()?;
    let result = consume_note_with_foreign(&mut mock_chain, registry, &register_note, pool).await;
    assert!(
        result.is_err(),
        "registration must fail when the pool's fee asset is not the chain's"
    );
    Ok(())
}

/// A network faucet launches its whole supply once, and only from its owner.
///
/// The faucet is a network account, so creating it needs no allowlist entry; the
/// owner's launch note mints the maximum supply and pays it out in a P2ID note.
#[tokio::test]
async fn network_faucet_launches_the_whole_supply_once() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder().verification_base_fee(7);

    let owner = builder.add_existing_wallet(basic_auth())?;
    let stranger = builder.add_existing_wallet(basic_auth())?;
    let recipient = builder.add_existing_wallet(basic_auth())?;
    let supply = 1_000_000_u64;
    let launch_root = note_root(&contracts.launch_note)?;

    // The faucet account: the network-faucet component, the standards' fungible
    // faucet component for the metadata, and the network auth.
    let faucet = FungibleFaucet::builder()
        .name(TokenName::new("Launch Token")?)
        .symbol(TokenSymbol::new("LNCH")?)
        .decimals(4)
        .max_supply(AssetAmount::new(supply).context("supply")?)
        .build()?;
    let standard = AccountComponent::new(
        FungibleFaucet::code().clone(),
        faucet.into_storage_slots(),
        FungibleFaucet::component_metadata(),
    )
    .context("standard faucet component")?;
    let mut init = InitStorageData::default();
    init.insert_value(
        StorageValueName::from_slot_name(
            &StorageSlotName::new("network_faucet::network_faucet::owner")?,
        ),
        Word::from([Felt::ZERO, Felt::ZERO, owner.id().suffix(), owner.id().prefix().as_felt()]),
    )?;
    init.insert_value(
        StorageValueName::from_slot_name(
            &StorageSlotName::new("network_faucet::network_faucet::launched")?,
        ),
        Word::from([Felt::ZERO, Felt::ZERO, Felt::ZERO, Felt::ZERO]),
    )?;
    let ours = AccountComponent::from_package(contracts.network_faucet.as_ref().clone(), &init)
        .context("network faucet component")?;

    let mut policy = BasicConstantFeePolicy::new();
    for root in [launch_root]
        .into_iter()
        .chain(AuthNetworkAccount::default_allowed_note_scripts().iter().copied())
    {
        policy = policy.with_fee(root, AssetAmount::ZERO);
    }
    let fee_faucet: AccountId = ACCOUNT_ID_FEE_FAUCET.try_into().expect("fee faucet id");
    let fee_policy_manager = FeePolicyManager::builder()
        .active_fee_policy(policy.into())
        .fee_faucet_id(fee_faucet)
        .build();
    let account_builder = AccountBuilder::new([11_u8; 32])
        .account_type(AccountType::Public)
        .with_component(ours)
        .with_component(standard)
        // The faucet pays its launch transaction's fee from its own balance, which
        // in production arrives in the funding note that creates the account.
        .with_assets([FungibleAsset::new(fee_faucet, 10_000)?.into()]);
    let auth = Auth::NetworkAccount {
        allowed_script_roots: [launch_root].into_iter().collect(),
        allowed_tx_script_roots: BTreeSet::new(),
        fee_policy_manager,
        sponsorship_policy: SponsorshipPolicy::default(),
    };
    let faucet_id = builder
        .add_account_from_builder(auth, account_builder, AccountState::Exists)
        .context("failed to build the network faucet")?
        .id();

    let launch_note = |sender: AccountId| -> Result<Note> {
        let mut storage = account_id_felts(faucet_id).to_vec();
        storage.extend(account_id_felts(recipient.id()));
        storage.extend([Felt::from(1_u8), Felt::from(2_u8), Felt::from(3_u8), Felt::from(4_u8)]);
        storage.push(Felt::ZERO); // output note tag
        build_note(contracts.launch_note.clone(), sender, storage, vec![])
    };

    // A launch note from anyone but the owner is rejected.
    let hostile = launch_note(stranger.id())?;
    builder.add_output_note(RawOutputNote::Full(hostile.clone()));
    let owner_note = launch_note(owner.id())?;
    builder.add_output_note(RawOutputNote::Full(owner_note.clone()));

    let mut chain = builder.build()?;
    assert!(
        consume_note(&mut chain, faucet_id, &hostile).await.is_err(),
        "a launch note from a non-owner must be rejected"
    );
    consume_note(&mut chain, faucet_id, &owner_note).await?;

    // The whole supply left the faucet in one P2ID note to the recipient.
    let paying = chain
        .committed_notes()
        .values()
        .filter_map(|note| match note {
            MockChainNote::Public(note, _) => Some(note.clone()),
            _ => None,
        })
        .filter_map(|note| {
            let asset = note
                .assets()
                .iter()
                .find(|asset| asset.id().faucet_id() == faucet_id)
                .cloned()?;
            Some((note, asset))
        })
        .max_by_key(|(_, asset)| asset.unwrap_fungible().amount().as_u64())
        .context("no payout note carried the token")?;
    assert_eq!(
        paying.1.unwrap_fungible().amount().as_u64(),
        supply,
        "the launch must pay the whole supply"
    );

    // A second launch is rejected: the flag is set.
    let again = {
        let mut storage = account_id_felts(faucet_id).to_vec();
        storage.extend(account_id_felts(recipient.id()));
        storage.extend([Felt::from(5_u8), Felt::from(6_u8), Felt::from(7_u8), Felt::from(8_u8)]);
        storage.push(Felt::ZERO);
        build_note(contracts.launch_note.clone(), owner.id(), storage, vec![])?
    };
    assert!(
        consume_note(&mut chain, faucet_id, &again).await.is_err(),
        "a token cannot launch twice"
    );
    Ok(())
}

/// A pool whose payout notes would carry a script of the creator's choosing is
/// rejected. The pool's `p2id_root` is storage, so the code commitment does not
/// pin it, and a hostile root would run an attacker script when a recipient
/// consumes a swap or burn payout.
#[tokio::test]
async fn register_rejects_a_pool_with_a_foreign_payout_script() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool_roots = pool_note_roots(&contracts)?;
    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        vec![],
        registry_note_roots(&contracts)?,
    )?;

    // A payout script that is not the standard P2ID.
    let hostile_root = Word::from(NoteScript::from_package(contracts.swap_note.as_ref())?.root());
    let pool = add_pool_account_network_with_p2id(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        hostile_root,
        pool_roots,
    )?;

    let register_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, pool, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(register_note.clone()));

    let mut mock_chain = builder.build()?;
    let result = consume_note_with_foreign(&mut mock_chain, registry, &register_note, pool).await;
    assert!(
        result.is_err(),
        "registration must fail when the pool's payout script root is not the P2ID root"
    );
    Ok(())
}

/// A pool that carries the accepted code but an allowlist of its own is
/// rejected. The code commitment pins code, not storage, and a note the pool
/// admits could drain its vault through the wallet's `move_asset_to_note`.
#[tokio::test]
async fn register_rejects_a_pool_with_a_foreign_allowlist() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool_roots = pool_note_roots(&contracts)?;
    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        vec![],
        registry_note_roots(&contracts)?,
    )?;

    // The same code commitment, one extra admitted note script.
    let mut hostile_roots = pool_roots;
    hostile_roots.insert(note_root(&contracts.register_note)?);
    let pool = add_pool_account_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        (0, 0),
        vec![],
        hostile_roots,
    )?;

    let register_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, pool, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(register_note.clone()));

    let mut mock_chain = builder.build()?;
    let result = consume_note_with_foreign(&mut mock_chain, registry, &register_note, pool).await;
    assert!(
        result.is_err(),
        "registration must fail when the pool's allowlist differs from the registry's"
    );
    Ok(())
}

/// A second pool cannot claim a pair that is already listed, which is what stops
/// anyone filing a competing pool under an existing one.
#[tokio::test]
async fn register_rejects_a_second_pool_for_the_same_pair() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let pool_roots = pool_note_roots(&contracts)?;
    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        vec![],
        registry_note_roots(&contracts)?,
    )?;

    // Two distinct pools, same pair.
    let first = add_pool_account_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        (0, 0),
        vec![],
        pool_roots.clone(),
    )?;
    let second = add_pool_account_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        (0, 0),
        vec![],
        pool_roots,
    )?;

    let first_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, first, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    let second_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, second, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(first_note.clone()));
    builder.add_output_note(RawOutputNote::Full(second_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note_with_foreign(&mut mock_chain, registry, &first_note, first).await?;
    let duplicate = consume_note_with_foreign(&mut mock_chain, registry, &second_note, second).await;
    assert!(
        duplicate.is_err(),
        "a second pool for a listed pair must be rejected"
    );
    Ok(())
}

/// Register fees accumulate in the registry's vault, and `collect` pays them to
/// the protocol account. Like a pool's collect it is permissionless, so the test
/// calls it without any authority and still sees the fees reach the protocol.
#[tokio::test]
async fn registry_collect_pays_the_protocol() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let fee_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "USDCX", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let protocol = builder.add_existing_wallet(basic_auth())?;

    let fee_key = FungibleAsset::new(fee_faucet.id(), 1)?.to_id_word();
    let collected = 1_337_u64;

    let registry = add_registry_account_with_assets(
        &mut builder,
        contracts.registry.clone(),
        protocol.id(),
        PROTOCOL_FEE_SHARE_BPS,
        Word::default(),
        zero_auth_roots(),
        register_note_root(&contracts)?,
        vec![FungibleAsset::new(fee_faucet.id(), collected)?.into()],
    )?;

    let serial = serial_num();
    // Layout: registry id, serial, then one key per asset the collect sweeps.
    let mut storage = account_id_felts(registry).to_vec();
    storage.extend([serial[0], serial[1], serial[2], serial[3]]);
    storage.extend([fee_key[0], fee_key[1], fee_key[2], fee_key[3]]);
    let collect_note = build_note(
        contracts.registry_collect_note.clone(),
        admin.id(),
        storage,
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(collect_note.clone()));

    let mut mock_chain = builder.build()?;
    let notes_before: BTreeSet<_> = mock_chain.committed_notes().keys().cloned().collect();
    consume_note(&mut mock_chain, registry, &collect_note).await?;

    assert_eq!(
        vault_balance(&mock_chain, registry, fee_faucet.id(), 0)?,
        0,
        "registry vault not drained"
    );

    let p2id_root = Word::from(P2idNote::script_root());
    let payout = mock_chain
        .committed_notes()
        .iter()
        .filter(|(id, _)| !notes_before.contains(*id))
        .find_map(|(_, note)| match note {
            MockChainNote::Public(note, _)
                if Word::from(note.recipient().script().root()) == p2id_root =>
            {
                Some(note.clone())
            }
            _ => None,
        })
        .context("collect created no P2ID payout note")?;
    let inputs = payout.recipient().storage().items();
    assert_eq!(
        inputs[0],
        protocol.id().suffix(),
        "payout recipient suffix is not the protocol account"
    );
    assert_eq!(
        inputs[1],
        protocol.id().prefix().as_felt(),
        "payout recipient prefix is not the protocol account"
    );
    Ok(())
}

// ==========================================================================================
// STORED RESERVES
//
// Reserves are the contract's own accounting; the vault is the custody backing it. A credit
// the contract did not create is never a reserve, and a collect sweeps it to the protocol.
// ==========================================================================================

/// A credit the contract did not create is not a reserve.
///
/// The funding surplus of the deploy flow is emulated by seeding the pool vault with 21 base
/// units of token1 before the first mint. The mint succeeds against the stored reserves, and
/// the stray credit stays in the vault as a sweepable balance.
#[tokio::test]
async fn stray_credit_does_not_enter_reserves() -> Result<()> {
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
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    // The 21 base units a 133-base-unit funding note leaves after the 112-base-unit fee.
    let pool = add_pool_account_with_reserves(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        admin.id(),
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (0, 0),
        vec![FungibleAsset::new(token1_faucet.id(), 21)?.into()],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO); // min_lp_out
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &mint_note).await?;

    let reserves = read_word(&mock_chain, pool, "reserves")?;
    assert_eq!(reserves[0].as_canonical_u64(), 1_000_000, "stored reserve0 mismatch");
    assert_eq!(reserves[1].as_canonical_u64(), 1_000_000, "stored reserve1 mismatch");
    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        1_000_000,
        "token0 vault mismatch"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        1_000_021,
        "the stray credit was not retained"
    );
    assert_eq!(read_scalar(&mock_chain, pool, "lp_supply")?, 1_000_000, "lp supply mismatch");
    Ok(())
}

/// A stray credit does not shift the next operation's quote.
///
/// With 50,000 base units of token1 credited outside an operation, the swap is still priced
/// against the stored reserve. The vault delta shows the clean quote; the credit is only
/// spendable through a collect.
#[tokio::test]
async fn stray_credit_does_not_shift_swap_output() -> Result<()> {
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
    // Large enough that a vault-derived quote would visibly shift; the stored reserves do not.
    let stray = 50_000_u64;
    let pool = add_pool_account_with_reserves(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        admin.id(),
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (reserve0, reserve1),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1 + stray)?.into(),
        ],
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
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    let (clean_out, protocol_fee) =
        amount_out(amount_in, reserve0, reserve1, LP_FEE_BPS, PROTOCOL_FEE_SHARE_BPS);

    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        reserve1 + stray - clean_out,
        "the swap was priced against the stray credit"
    );
    let reserves = read_word(&mock_chain, pool, "reserves")?;
    assert_eq!(
        reserves[0].as_canonical_u64(),
        reserve0 + amount_in - protocol_fee,
        "stored reserve0 mismatch"
    );
    assert_eq!(
        reserves[1].as_canonical_u64(),
        reserve1 - clean_out,
        "stored reserve1 mismatch"
    );
    Ok(())
}

/// The oracle accrues at the stored reserves, not at a stray credit.
#[tokio::test]
async fn oracle_ignores_stray_credit() -> Result<()> {
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
    let stray = 21_u64;
    let pool = add_pool_account_with_reserves(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        admin.id(),
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (reserve0, reserve1),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1 + stray)?.into(),
        ],
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
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &swap_note).await?;

    // The first block's timestamp is 1_700_000_000, and the pool's `block_timestamp_last`
    // starts at zero, so the whole span accrues at the stored reserves.
    let elapsed = 1_700_000_000_u128;
    let clean_price0 = ((reserve1 as u128) << 64) / reserve0 as u128;
    let p0 = word_to_u128(read_word(&mock_chain, pool, "price0_cumulative_last")?);
    assert_eq!(
        p0,
        clean_price0 * elapsed,
        "the oracle accrued at the stray credit"
    );
    Ok(())
}

/// A full LP exit leaves the locked minimum's residue, and the stored reserves track it.
///
/// The burn check keeps `MINIMUM_LIQUIDITY` in the pool, so the reserves after a full exit
/// are nonzero and the pool remains usable.
#[tokio::test]
async fn full_exit_leaves_usable_residue() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    // Odd amounts force a nonzero rounding residue on the burn.
    let amount0 = 1_000_000_u64;
    let amount1 = 1_000_001_u64;
    let pool = add_pool_account(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        vec![],
    )?;

    let lp_supply = isqrt(amount0 as u128 * amount1 as u128);
    let user_lp = lp_supply - 1000;
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
            FungibleAsset::new(pool, user_lp)?.into(),
        ],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO);
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), amount0)?.into(),
            FungibleAsset::new(token1_faucet.id(), amount1)?.into(),
        ],
    )?;

    let burn_serial = Word::from([Felt::from(9_u8), Felt::from(8_u8), Felt::from(7_u8), Felt::from(6_u8)]);
    let mut burn_storage = pool_note_prefix(pool, lp.id(), burn_serial, Felt::ZERO);
    burn_storage.push(Felt::ZERO);
    burn_storage.push(Felt::ZERO);
    let burn_note = build_note(
        contracts.burn_note.clone(),
        lp.id(),
        burn_storage,
        vec![FungibleAsset::new(pool, user_lp)?.into()],
    )?;

    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));
    builder.add_output_note(RawOutputNote::Full(burn_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note(&mut mock_chain, pool, &mint_note).await?;
    consume_note(&mut mock_chain, pool, &burn_note).await?;

    let remaining0 = vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?;
    let remaining1 = vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?;
    assert_eq!(read_scalar(&mock_chain, pool, "lp_supply")?, 1000, "lp supply is not the locked minimum");
    assert!(remaining0 > 0 && remaining1 > 0, "a reserve was drained to zero");
    assert!(
        remaining0 < amount0 && remaining1 < amount1,
        "the exit did not pay out a proportional share"
    );
    let reserves = read_word(&mock_chain, pool, "reserves")?;
    assert_eq!(reserves[0].as_canonical_u64(), remaining0, "stored reserve0 != vault");
    assert_eq!(reserves[1].as_canonical_u64(), remaining1, "stored reserve1 != vault");
    Ok(())
}

/// The code commitment covers code, not storage.
///
/// Two pools that differ only in their registry pointer (a storage value) have the same code
/// commitment. Adding a fee-schedule entry to the network auth therefore does not move the
/// value the registry accepts.
#[tokio::test]
async fn code_commitment_is_storage_independent() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 1_000_000, Some(1_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 1_000_000, Some(1_000_000))?;
    let admin_a = builder.add_existing_wallet(basic_auth())?;
    let admin_b = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let commitment_a =
        pool_code_commitment(&mut builder, &contracts, token0_key, token1_key, admin_a.id())?;
    let commitment_b =
        pool_code_commitment(&mut builder, &contracts, token0_key, token1_key, admin_b.id())?;
    assert_eq!(
        commitment_a, commitment_b,
        "the code commitment moved with a storage value"
    );
    Ok(())
}

/// Collect pays the protocol pot, any credit outside the reserves, and third assets, while
/// never touching the recorded reserves.
#[tokio::test]
async fn collect_sweeps_stray_credits_and_third_assets() -> Result<()> {
    let contracts = Contracts::build()?;
    let mut builder = MockChain::builder();

    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let token2_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "GAMMA", 2_000_000, Some(2_000_000))?;
    let admin = builder.add_existing_wallet(basic_auth())?;
    let registry_fee_to = builder.add_existing_wallet(basic_auth())?;

    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();
    let token2_key = FungibleAsset::new(token2_faucet.id(), 1)?.to_id_word();

    let accepted = pool_code_commitment(&mut builder, &contracts, token0_key, token1_key, admin.id())?;
    let registry = add_registry_account(
        &mut builder,
        contracts.registry.clone(),
        registry_fee_to.id(),
        PROTOCOL_FEE_SHARE_BPS,
        accepted,
        zero_auth_roots(),
        register_note_root(&contracts)?,
    )?;

    // The vault holds the reserves plus a credit in each of the three assets.
    let reserve0 = 1_000_000_u64;
    let reserve1 = 1_000_000_u64;
    let stray0 = 1_234_u64;
    let stray1 = 5_678_u64;
    let third = 5_000_u64;
    let pool = add_pool_account_with_reserves(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        registry,
        LP_FEE_BPS,
        PROTOCOL_FEE_SHARE_BPS,
        (reserve0, reserve1),
        vec![
            FungibleAsset::new(token0_faucet.id(), reserve0 + stray0)?.into(),
            FungibleAsset::new(token1_faucet.id(), reserve1 + stray1)?.into(),
            FungibleAsset::new(token2_faucet.id(), third)?.into(),
        ],
    )?;

    let serial = serial_num();
    let mut collect_storage = account_id_felts(pool).to_vec();
    collect_storage.extend([serial[0], serial[1], serial[2], serial[3]]);
    for key in [token0_key, token1_key, token2_key] {
        collect_storage.extend([key[0], key[1], key[2], key[3]]);
    }
    let collect_note = build_note(
        contracts.collect_note.clone(),
        admin.id(),
        collect_storage,
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(collect_note.clone()));

    let mut mock_chain = builder.build()?;
    consume_note_with_foreign(&mut mock_chain, pool, &collect_note, registry).await?;

    assert_eq!(
        vault_balance(&mock_chain, pool, token0_faucet.id(), 0)?,
        reserve0,
        "collect touched the token0 reserve"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token1_faucet.id(), 0)?,
        reserve1,
        "collect touched the token1 reserve"
    );
    assert_eq!(
        vault_balance(&mock_chain, pool, token2_faucet.id(), 0)?,
        0,
        "the third asset was not swept"
    );

    // The swept amounts left as P2ID notes to the registry's protocol account.
    let p2id_root = Word::from(P2idNote::script_root());
    let mut swept = Vec::new();
    for note in mock_chain.committed_notes().values() {
        if let MockChainNote::Public(note, _) = note {
            if Word::from(note.recipient().script().root()) == p2id_root {
                let inputs = note.recipient().storage().items();
                if inputs[0] == registry_fee_to.id().suffix()
                    && inputs[1] == registry_fee_to.id().prefix().as_felt()
                {
                    for asset in note.assets().iter() {
                        if let Some(fungible) = asset.as_fungible() {
                            swept.push((fungible.faucet_id(), fungible.amount().as_u64()));
                        }
                    }
                }
            }
        }
    }
    assert!(swept.contains(&(token0_faucet.id(), stray0)), "stray token0 not swept: {swept:?}");
    assert!(swept.contains(&(token1_faucet.id(), stray1)), "stray token1 not swept: {swept:?}");
    assert!(swept.contains(&(token2_faucet.id(), third)), "third asset not swept: {swept:?}");
    Ok(())
}

/// Measures the cycles of the canonical network-account transaction consuming
/// each port0 note root on a fee-charging chain, then prints the fee the node
/// charges and the schedule price the `NetworkNotePricer` derives.
#[tokio::test]
async fn note_fee_benchmark() -> Result<()> {
    let contracts = Contracts::build()?;
    let mint_root = note_root(&contracts.mint_note)?;
    let burn_root = note_root(&contracts.burn_note)?;
    let swap_root = note_root(&contracts.swap_note)?;
    let collect_root = note_root(&contracts.collect_note)?;
    let register_root = note_root(&contracts.register_note)?;
    let registry_collect_root = note_root(&contracts.registry_collect_note)?;
    let pool_roots: BTreeSet<NoteScriptRoot> =
        [mint_root, burn_root, swap_root, collect_root].into_iter().collect();
    let registry_roots: BTreeSet<NoteScriptRoot> =
        [register_root, registry_collect_root].into_iter().collect();

    let fee_faucet_id: AccountId = ACCOUNT_ID_FEE_FAUCET.try_into().expect("valid fee faucet id");
    let fee_key = FungibleAsset::new(fee_faucet_id, 1)?.to_id_word();

    let mut measured: Vec<(&'static str, NoteScriptRoot, usize)> = Vec::new();

    // Pool chain: mint, swap, burn, collect. The registry is a network account
    // too, because the pool's collect reads its protocol_fee_to by foreign call.
    let mut builder = MockChain::builder().verification_base_fee(7);
    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let mut extra_faucets = Vec::new();
    for symbol in ["GAMMA", "DELTA", "EPSILON", "ZETA", "ETA", "THETA", "IOTA"] {
        extra_faucets.push(builder.add_existing_basic_faucet(
            basic_auth(),
            symbol,
            1_000_000,
            Some(1_000_000),
        )?);
    }
    let admin = builder.add_existing_wallet(basic_auth())?;
    let trader = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;
    let trader1 = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [FungibleAsset::new(token1_faucet.id(), 10_000)?.into()],
    )?;
    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        admin.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        zero_auth_roots(),
        register_note_root(&contracts)?,
        vec![],
        registry_roots.clone(),
    )?;
    let mut pool_seed: Vec<Asset> = vec![FungibleAsset::new(fee_faucet_id, 1_000_000)?.into()];
    for faucet in extra_faucets.iter().take(6) {
        pool_seed.push(FungibleAsset::new(faucet.id(), 10)?.into());
    }
    let pool = add_pool_account_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        (0, 0),
        pool_seed,
        pool_roots.clone(),
    )?;
    let lp = builder.add_existing_wallet_with_assets(
        basic_auth(),
        [
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(pool, 100_000)?.into(),
        ],
    )?;

    let mut mint_storage = pool_note_prefix(pool, lp.id(), serial_num(), Felt::ZERO);
    mint_storage.push(Felt::ZERO); // min_lp_out
    let mint_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 1_000_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 1_000_000)?.into(),
        ],
    )?;
    builder.add_output_note(RawOutputNote::Full(mint_note.clone()));

    let second_serial =
        Word::from([Felt::from(3_u8), Felt::from(1_u8), Felt::from(4_u8), Felt::from(1_u8)]);
    let mut mint2_storage = pool_note_prefix(pool, lp.id(), second_serial, Felt::ZERO);
    mint2_storage.push(Felt::ZERO); // min_lp_out
    let mint2_note = build_note(
        contracts.mint_note.clone(),
        lp.id(),
        mint2_storage,
        vec![
            FungibleAsset::new(token0_faucet.id(), 10_000)?.into(),
            FungibleAsset::new(token1_faucet.id(), 10_000)?.into(),
        ],
    )?;
    builder.add_output_note(RawOutputNote::Full(mint2_note.clone()));

    let mut swap_storage = pool_note_prefix(pool, trader.id(), serial_num(), Felt::ZERO);
    swap_storage.push(Felt::ZERO); // min_amount_out
    let swap_note = build_note(
        contracts.swap_note.clone(),
        trader.id(),
        swap_storage,
        vec![FungibleAsset::new(token0_faucet.id(), 10_000)?.into()],
    )?;
    builder.add_output_note(RawOutputNote::Full(swap_note.clone()));

    let mut swap2_storage = pool_note_prefix(pool, trader1.id(), second_serial, Felt::ZERO);
    swap2_storage.push(Felt::ZERO); // min_amount_out
    let swap2_note = build_note(
        contracts.swap_note.clone(),
        trader1.id(),
        swap2_storage,
        vec![FungibleAsset::new(token1_faucet.id(), 10_000)?.into()],
    )?;
    builder.add_output_note(RawOutputNote::Full(swap2_note.clone()));

    let burn_serial =
        Word::from([Felt::from(9_u8), Felt::from(8_u8), Felt::from(7_u8), Felt::from(6_u8)]);
    let mut burn_storage = pool_note_prefix(pool, lp.id(), burn_serial, Felt::ZERO);
    burn_storage.push(Felt::ZERO); // min_amount0
    burn_storage.push(Felt::ZERO); // min_amount1
    let burn_note = build_note(
        contracts.burn_note.clone(),
        lp.id(),
        burn_storage,
        vec![FungibleAsset::new(pool, 100_000)?.into()],
    )?;
    builder.add_output_note(RawOutputNote::Full(burn_note.clone()));

    let serial = serial_num();
    let mut collect_storage = account_id_felts(pool).to_vec();
    collect_storage.extend([serial[0], serial[1], serial[2], serial[3]]);
    let mut collect_keys = vec![token0_key, token1_key];
    for faucet in extra_faucets.iter().take(2) {
        collect_keys.push(FungibleAsset::new(faucet.id(), 1)?.to_id_word());
    }
    for key in collect_keys {
        collect_storage.extend([key[0], key[1], key[2], key[3]]);
    }
    let collect_note =
        build_note(contracts.collect_note.clone(), admin.id(), collect_storage, vec![])?;
    builder.add_output_note(RawOutputNote::Full(collect_note.clone()));

    let mut chain = builder.build()?;
    measured.push(("pool mint", mint_root, bench_consume(&mut chain, pool, &mint_note, None).await?));
    measured.push(("pool mint 2", mint_root, bench_consume(&mut chain, pool, &mint2_note, None).await?));
    measured.push(("pool swap", swap_root, bench_consume(&mut chain, pool, &swap_note, None).await?));
    measured.push(("pool swap 2", swap_root, bench_consume(&mut chain, pool, &swap2_note, None).await?));
    measured.push(("pool burn", burn_root, bench_consume(&mut chain, pool, &burn_note, None).await?));
    measured.push((
        "pool collect",
        collect_root,
        bench_consume(&mut chain, pool, &collect_note, Some(registry)).await?,
    ));

    // Registry chain: register, registry collect. The register note's script reads
    // the pool by foreign call.
    let mut builder = MockChain::builder().verification_base_fee(7);
    let token0_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "ALPHA", 2_000_000, Some(2_000_000))?;
    let token1_faucet =
        builder.add_existing_basic_faucet(basic_auth(), "BETA", 2_000_000, Some(2_000_000))?;
    let mut extra_faucets = Vec::new();
    for symbol in ["GAMMA", "DELTA", "EPSILON", "ZETA", "ETA", "THETA", "IOTA"] {
        extra_faucets.push(builder.add_existing_basic_faucet(
            basic_auth(),
            symbol,
            1_000_000,
            Some(1_000_000),
        )?);
    }
    let admin = builder.add_existing_wallet(basic_auth())?;
    let token0_key = FungibleAsset::new(token0_faucet.id(), 1)?.to_id_word();
    let token1_key = FungibleAsset::new(token1_faucet.id(), 1)?.to_id_word();

    let fixture = pool_code_commitment_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        admin.id(),
        &pool_roots,
    )?;
    let mut registry_seed: Vec<Asset> = vec![FungibleAsset::new(fee_faucet_id, 100_000)?.into()];
    for faucet in &extra_faucets {
        registry_seed.push(FungibleAsset::new(faucet.id(), 10)?.into());
    }
    let registry = add_registry_account_network(
        &mut builder,
        contracts.registry.clone(),
        admin.id(),
        PROTOCOL_FEE_SHARE_BPS,
        fixture.commitment,
        fixture.auth_roots,
        register_note_root(&contracts)?,
        registry_seed,
        registry_roots.clone(),
    )?;
    let pool = add_pool_account_network(
        &mut builder,
        contracts.port0.clone(),
        token0_key,
        token1_key,
        registry,
        (0, 0),
        vec![],
        pool_roots.clone(),
    )?;

    let register_note = build_note(
        contracts.register_note.clone(),
        admin.id(),
        register_note_storage(registry, pool, token0_faucet.id(), token1_faucet.id()),
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(register_note.clone()));

    let serial = serial_num();
    let mut registry_collect_storage = account_id_felts(registry).to_vec();
    registry_collect_storage.extend([serial[0], serial[1], serial[2], serial[3]]);
    registry_collect_storage.extend([fee_key[0], fee_key[1], fee_key[2], fee_key[3]]);
    // Four keys including the fee asset: the shape the sweep tooling sends, so
    // the registry's hold is measured on the path it will actually take.
    for faucet in extra_faucets.iter().take(3) {
        let key = FungibleAsset::new(faucet.id(), 1)?.to_id_word();
        registry_collect_storage.extend([key[0], key[1], key[2], key[3]]);
    }
    let registry_collect_note = build_note(
        contracts.registry_collect_note.clone(),
        admin.id(),
        registry_collect_storage,
        vec![],
    )?;
    builder.add_output_note(RawOutputNote::Full(registry_collect_note.clone()));

    let mut chain = builder.build()?;
    measured.push((
        "registry register",
        register_root,
        bench_consume(&mut chain, registry, &register_note, Some(pool)).await?,
    ));
    measured.push((
        "registry collect",
        registry_collect_root,
        bench_consume(&mut chain, registry, &registry_collect_note, None).await?,
    ));

    // The schedule to install: fee from the kernel formula, price from the
    // pricer's default one-verification-cycle margin.
    // Per root, the maximum across the measured paths: the schedule must cover
    // the most expensive path.
    let mut max_cycles: BTreeMap<NoteScriptRoot, usize> = BTreeMap::new();
    for (_, root, cycles) in &measured {
        max_cycles
            .entry(*root)
            .and_modify(|current| *current = (*current).max(*cycles))
            .or_insert(*cycles);
    }

    let pricer = {
        let mut pricer_builder = NetworkNotePricer::builder()
            .fee_parameters(FeeParameters::new(7))
            .fee_asset_id(AssetId::new_fungible(fee_faucet_id));
        for (root, cycles) in &max_cycles {
            pricer_builder =
                pricer_builder.note_cost(*root, NoteCost::new(*cycles as u32, Vec::new()));
        }
        pricer_builder.build()
    };

    for (name, root, cycles) in &measured {
        let fee = kernel_fee(*cycles as u32);
        // `collect` reserves this fee out of the fee asset before paying out, so the
        // reservation must cover each measured collect path. The constants are
        // `COLLECT_FEE_HOLD` in `contracts/port0` (119) and `contracts/pool-registry` (112).
        let hold = if name.starts_with("pool collect") {
            POOL_COLLECT_FEE_HOLD
        } else if name.starts_with("registry collect") {
            REGISTRY_COLLECT_FEE_HOLD
        } else {
            0
        };
        if hold > 0 {
            assert!(
                fee <= hold,
                "{name}: measured fee {fee} exceeds the contract's collect hold {hold}"
            );
        }
        println!("{name} root={} cycles={cycles} fee={fee}", Word::from(*root).to_hex());
    }
    println!("schedule (max path per root):");
    for (root, cycles) in &max_cycles {
        let fee = kernel_fee(*cycles as u32);
        let price = pricer.price(*root)?;
        println!(
            "  root={} cycles={cycles} fee={fee} price={}",
            Word::from(*root).to_hex(),
            price.as_u64(),
        );
    }
    Ok(())
}
