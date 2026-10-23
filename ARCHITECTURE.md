# Architecture

This document describes the internal design of port0.sh. `README.md` describes
the consumer-facing note interface.

## Overview

port0.sh is a constant-product automated market maker. A single account holds
two token reserves. Liquidity providers deposit both tokens and receive LP
tokens. Traders exchange one token for the other. A protocol fee accrues to the
protocol account the registry designates.

## Components

Nine contracts compose the system.

- `port0`: the pool account component. Holds the reserves accounting, the
  LP supply, the fee rates, the protocol pot, and the price oracle.
- `pool-registry`: the registry account component. Holds the accepted pool code
  commitments, the registered pools, the ordered asset pair index, the protocol
  fee policy, and the pool storage roots it pins at registration.
- `mint-note`: note script for adding liquidity.
- `burn-note`: note script for removing liquidity.
- `swap-note`: note script for trading.
- `collect-note`: note script for withdrawing a pool's protocol fees.
- `register-note`: note script for registering a pool.
- `registry-collect-note`: note script for withdrawing the registry's register
  fees.
- `registry-config-note`: note script for changing the register fee, admin only.

The pool component exposes `token0_asset_key`, `token1_asset_key`, `lp_supply`,
`get_reserves`, `get_fees`, `mint`, `burn`, `swap`, `quote_swap`, `collect`,
`get_registry_identity`, `get_auth_identity`, and `get_fee_identity` as account
procedures. A pool's fee rates, P2ID root, registry pointer, and token keys are
part of its initial storage and hold for its lifetime.
`get_registry_identity` returns the registry pointer, the two token keys, the two
fee rates, and the payout root as sixteen felts, and is the procedure the
registry calls over foreign procedure invocation at registration.
`get_auth_identity` and `get_fee_identity` return the six storage roots the
registry pins.

## Registry

The registry account declares fifteen storage slots.

| Slot | Type | Key | Value |
|------|------|-----|-------|
| `accepted_code_commitments` | map | pool code commitment | `1` when accepted |
| `pools` | map | pool id key `[prefix, suffix, 0, 0]` | pool code commitment |
| `assets_to_pool` | map | ordered asset pair `[lo.prefix, lo.suffix, hi.prefix, hi.suffix]` | pool id key |
| `protocol_fee_to` | value | account id word `[0, 0, suffix, prefix]` | protocol fee destination |
| `protocol_fee_share_bps` | value | `[bps, 0, 0, 0]` | protocol fee rate a pool must carry |
| `allowed_lp_fee_bps` | map | LP fee tier in bps | `1` when a pool may charge it |
| `p2id_root` | value | P2ID script root | script root of the payout note, and the root every pool must carry |
| `expected_note_allowlist_root` | value | root | note allowlist root every pool must carry |
| `expected_tx_script_allowlist_root` | value | root | transaction-script allowlist root every pool must carry |
| `expected_fee_schedule_root` | value | root | fee schedule root every pool must carry |
| `expected_fee_asset_id` | value | asset id word | fee asset every pool must charge in |
| `expected_active_fee_policy_root` | value | root | active fee policy root every pool must carry |
| `expected_allowed_fee_policy_roots` | value | root | allowed fee policy roots every pool must carry |
| `admin` | value | account id word `[0, 0, suffix, prefix]` | account allowed to change the register fee |
| `register_note_root` | value | script root | the only script root whose fee the admin may change |

`register` rejects equal assets, an already-registered pool id, and an
already-registered asset pair. It then calls the pool's
`CodeInspection::get_code_commitment`, `get_registry_identity`,
`get_auth_identity`, and `get_fee_identity` by foreign procedure invocation and
asserts:

- the returned commitment is in `accepted_code_commitments`;
- the pool's registry pointer equals the executing registry;
- the pool's token keys match the claimed pair in either order;
- the pool's protocol fee rate equals the registry's
  `protocol_fee_share_bps`;
- the pool's LP fee is an entry in `allowed_lp_fee_bps`;
- the pool's payout root equals the registry's `p2id_root`;
- the pool's note allowlist, transaction-script allowlist, and fee schedule
  roots equal the values the registry was created with;
- the pool's fee asset, active fee policy root, and allowed fee policy roots
  equal the values the registry was created with.

The storage checks exist because a code commitment pins code and not storage: a
pool could carry the accepted component set with an allowlist of its own, and an
admitted note could move the pool's assets out through the wallet's
`move_asset_to_note`.

## Register fee

The registry has one admin, fixed at creation, with one power.
`set_register_fee(new_fee, note_root)` rewrites the register note's entry in the
registry's own fee schedule. It requires the note sender to be the admin, the
new fee to be positive, and the script root to already carry a scheduled entry,
so only the register note can be repriced. The call is a `registry-config` note.
Registration is permissionless, so the fee is the only lever against a flood of
pools.

It then writes `pools` and `assets_to_pool`. The pool installs the standard
`CodeInspection` component, which carries no storage. The accepted commitment
covers the pool's full component set: the `port0` component,
`CodeInspection`, the network-account auth, and the fee-policy components. It
therefore changes when any of those change, and the registry must be seeded with
the value the deployed pool produces.

`pool_for_assets` returns `[found, prefix, suffix, 0]` and never a zero account
id as if it were a pool. `collect(asset_key, serial_num)` moves the registry's
whole balance of `asset_key` to `protocol_fee_to` as a P2ID note; it is
permissionless and reverts when the registry holds none of the asset or when
`p2id_root` is unset.

## Storage

The pool component declares eleven value slots.

| Slot | Type | Encoding |
|------|------|----------|
| `token0_asset_key` | Word | token0 asset key |
| `token1_asset_key` | Word | token1 asset key |
| `lp_supply` | Felt | `[supply, 0, 0, 0]` |
| `reserves` | Word | `[reserve0, reserve1, 0, 0]` |
| `fees` | Word | `[lp_fee_bps, protocol_fee_share_bps, 0, 0]` |
| `p2id_root` | Word | P2ID script root used for output notes |
| `protocol_pot` | Word | `[amount0, amount1, 0, 0]` |
| `price0_cumulative_last` | Word | u128, four 32-bit limbs, least significant first |
| `price1_cumulative_last` | Word | u128, four 32-bit limbs, least significant first |
| `block_timestamp_last` | Felt | `[timestamp, 0, 0, 0]` |
| `registry_id` | Word | `[0, 0, suffix, prefix]` of the registry account id |

Every slot except `reserves`, `protocol_pot`, and the oracle accumulators is
written at account creation. The LP token key is not a slot: the contract
derives it from the pool's account id as `[0, 0, suffix | 0x11, prefix]`, because
the key embeds the account id and the account id is derived from the initial
storage commitment, so seeding it would be circular. `reserves` starts at zero
and is written by `mint`, `burn`, and `swap`; `protocol_pot` accrues as swaps
happen.

## Reserves

Reserves are stored accounting: the `reserves` word is authoritative, and the
vault is the custody backing it. `get_reserves` returns the stored word and
asserts the vault covers `reserves + protocol_pot` for both tokens on every
read. A credit the contract did not create (a funding surplus, a stray P2ID, a
sponsorship remainder) is never a reserve and never enters a quote.

## Execution model

Notes invoke pool procedures through the component interface. Pool procedures
run in account context and may create output notes and move assets. Note scripts
run in note context and may not perform these actions directly.

## Mint

On the first deposit the pool computes `root = isqrt(reserve0_after *
reserve1_after)` and requires `root > MINIMUM_LIQUIDITY`. It mints `root` LP
tokens and retains `MINIMUM_LIQUIDITY = 1000`; the provider receives `root -
1000`.

On later deposits the pool mints the smaller of `amount0 * lp_supply / reserve0`
and `amount1 * lp_supply / reserve1`. The amount of each token used rounds up in
the pool's favor with `div_ceil`; the remainder returns to the provider.

## Burn

For a burn of `amount` LP tokens, the pool returns `amount * reserve0 /
lp_supply` of token0 and `amount * reserve1 / lp_supply` of token1, then reduces
`lp_supply`. The burn reverts when `amount > lp_supply - MINIMUM_LIQUIDITY` and
when either payout rounds to zero.

## Swap

The pool computes the LP fee on the full input, carves the protocol's share out
of that fee, and computes the output from the full input with the LP fee weight.
The output does not depend on the protocol's share.

```
lp_fee             = amount_in * lp_fee_bps / 10000
protocol_fee       = lp_fee * protocol_fee_share_bps / 10000
amount_in_with_fee = amount_in * (10000 - lp_fee_bps)
amount_out         = amount_in_with_fee * reserve_out /
                     (reserve_in * 10000 + amount_in_with_fee)
```

The full input enters the vault. The output leaves the vault. The reserve grows
by `amount_in - protocol_fee`; the pool keeps the LP fee minus the protocol's
share. The protocol fee is recorded in `protocol_pot`.

## quote_swap

`quote_swap` runs the swap computation without modifying state. It returns the
amount out, the resulting reserves, and the protocol fee.

## Collect

`collect(asset_key, serial_num)` pays everything the vault holds above the
recorded reserves for one asset key to the registry's protocol account, as a
P2ID note. For token0 and token1 that is the protocol pot plus any credit
outside the reserves, and the pot is reset because it was paid. For any other
key it is the whole balance, so a third asset sent to the pool is recoverable.
The call is permissionless and the destination is not stored in the pool: it is
read from the registry over foreign procedure invocation on the collect path, so
it is rotated in one place.

When the key is the chain fee asset, `collect` leaves a measured reserve in the
vault for the transaction's own epilogue charge. The reserve is
`POOL_COLLECT_FEE_HOLD` (126 base units) and `REGISTRY_COLLECT_FEE_HOLD` (119)
for the registry, measured on the four-key path and stored in
`contracts/collect-fee-hold.rs`, which both contracts include. The reserve
applies only when the transaction carries no FEE_SPONSORSHIP note: a sponsored
collect reserves nothing and sweeps the whole balance. A collect note carries
`[pool_id, serial, key...]`, calls `collect` once per key, and carries at most
four keys.

## Configuration

A pool's configuration is its initial storage: `lp_fee_bps`,
`protocol_fee_share_bps`, `p2id_root`, `registry_id`, and the two token asset
keys. The LP token key is derived from the pool's account id. No pool parameter
is settable after creation.

The registry is the authority on which pools are listed and on the fee policy
they must carry. Its `register` procedure rejects a pool whose token pair, fee
share, LP fee tier, or registry pointer disagrees with the registry.

## Oracle

The oracle updates at the start of each mint, burn, and swap. It computes

```
p0 = (reserve1 << 64) / reserve0
p1 = (reserve0 << 64) / reserve1
```

and adds `p0 * elapsed` and `p1 * elapsed` to the two cumulative values, where
`elapsed` is the block timestamp delta. The accumulators saturate at
`u128::MAX`. A backward timestamp yields an `elapsed` of zero. A pool with a
zero reserve returns before updating the accumulators.

## Asset model

The LP token is a fungible asset issued by the pool account. No separate faucet
account exists.

## Numeric bounds

Amounts, reserves, and `lp_supply` are u64 values bounded by `MAX_AMOUNT = 2^57`.

The binding constraint is the swap numerator, `amount_in_with_fee * reserve_out`.
`amount_in_with_fee` is at most `amount_in * 10000`, so the worst case is
`MAX_AMOUNT^2 * 10000`. This value must stay below `2^128`, which requires
`MAX_AMOUNT < 2^64 / 100`. `2^57` satisfies the bound with margin.

Mint and burn products stay below `2^126`.

## Failure conditions

Each procedure reverts under these conditions.

Mint:

- deposit missing one token
- deposit plus reserve above `MAX_AMOUNT`
- first deposit mints at most `MINIMUM_LIQUIDITY`
- later deposit mints zero liquidity
- liquidity out below `min_lp_out`

Burn:

- note carries multiple assets or a non-LP asset
- LP amount zero
- pool empty
- amount, reserve, or LP supply above `MAX_AMOUNT`
- burn leaves less than `MINIMUM_LIQUIDITY`
- either payout rounds to zero
- payout below its minimum

Swap:

- note carries multiple assets or an unknown asset
- input zero
- pool empty
- input or reserve above `MAX_AMOUNT`
- output zero or at least `reserve_out`
- resulting reserve above `MAX_AMOUNT`
- output below `min_amount_out`

Collect:

- a key that pays nothing is a no-op, and a sweep in which every key pays
  nothing moves no value. The consumed note and the paid fee are the
  transaction's effect.

Registry `collect`:

- `p2id_root` unset
- asset key empty

A key that pays nothing is a no-op, as in a pool collect.

Registry `set_register_fee`:

- note sender differs from `admin`
- new fee is zero
- script root differs from `register_note_root`
- the script root has no schedule entry with its set marker

## Indexing

An indexer reads the registry's `pools` map to obtain the pool set. It then
reads each public pool's account storage, vault, and network-account fee slots
to obtain reserves, `lp_supply`, fees, the oracle accumulators, the fee asset,
and the per-note fee schedule. Reserves are the pool's stored `reserves` word;
the vault is checked to cover the reserves plus the protocol pot.

The node returns at most 1000 entries per storage map in one account read. A
`pools` map above that limit must be paged with `SyncAccountStorageMaps` or
sharded; an indexer that reads it inline stops seeing new pools at 1001.
