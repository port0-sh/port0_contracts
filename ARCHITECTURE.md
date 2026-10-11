# Architecture

This document describes the internal design of MidenXYK. `README.md` describes the consumer-facing note interface.

## Overview

MidenXYK is a constant-product automated market maker. A single account holds two token reserves. Liquidity providers deposit both tokens and receive LP tokens. Traders exchange one token for the other. A fee recipient collects a protocol fee.

## Components

Eight contracts compose the system.

- `miden-xyk`: the pool account component. Holds the reserves, the LP supply, and the fee accounting.
- `pool-registry`: the registry account component. Holds the accepted pool code commitments, the registered pools, and the ordered asset pair index.
- `mint-note`: note script for adding liquidity.
- `burn-note`: note script for removing liquidity.
- `swap-note`: note script for trading.
- `collect-note`: note script for withdrawing protocol fees.
- `register-note`: note script for registering a pool.

The pool component exposes `mint`, `burn`, `swap`, `quote_swap`, `collect`, `get_registry_identity`, and the state getters. There is no `configure`: fees and the P2ID root are written at account creation and never change.

## Registry

The registry account holds three storage maps:

| Slot | Key | Value |
|------|-----|-------|
| `accepted_code_commitments` | pool code commitment | `1` when accepted |
| `pools` | pool id key `[prefix, suffix, 0, 0]` | pool code commitment |
| `assets_to_pool` | ordered asset pair `[lo.prefix, lo.suffix, hi.prefix, hi.suffix]` | pool id key |

`register` rejects equal assets, an already-registered pool id, and an already-registered asset pair. It then calls the pool's `CodeInspection::get_code_commitment` by foreign procedure invocation, requires the returned commitment in `accepted_code_commitments`, and writes both mappings. The pool installs the standard `CodeInspection` component, which carries no storage.

The accepted commitment covers the pool's full component set: the `miden-xyk` component, `CodeInspection`, the network-account auth, and the fee-policy components. It therefore changes when any of those change, and the registry must be seeded with the value the deployed pool produces.

## Storage

The pool component declares 11 value slots.

| Slot | Type | Encoding |
|------|------|----------|
| `token0_asset_key` | Word | token0 asset key |
| `token1_asset_key` | Word | token1 asset key |
| `lp_supply` | Felt | `[supply, 0, 0, 0]` |
| `fees` | Word | `[lp_fee_bps, protocol_fee_share_bps, fee_to.prefix, fee_to.suffix]` |
| `admin` | Word | `[0, 0, suffix, prefix]` of the admin account id |
| `p2id_root` | Word | P2ID script root used for output notes |
| `protocol_pot` | Word | `[amount0, amount1, 0, 0]` |
| `price0_cumulative_last` | Word | u128, four 32-bit limbs, least significant first |
| `price1_cumulative_last` | Word | u128, four 32-bit limbs, least significant first |
| `block_timestamp_last` | Felt | `[timestamp, 0, 0, 0]` |

Every slot is written at account creation except the LP token key, which the contract derives from the pool's own account id, and the protocol pot, which accrues as swaps happen.

## Reserves

The pool does not store reserves. Reserves equal the vault balances of token0 and token1 minus the protocol pot.

## Execution model

Notes invoke pool procedures through the component interface. Pool procedures run in account context and may create output notes and move assets. Note scripts run in note context and may not perform these actions directly.

## Mint

On the first deposit, the pool mints `sqrt(x * y)` LP tokens and retains `MINIMUM_LIQUIDITY = 1000` for itself. The provider receives `sqrt(x * y) - 1000`.

On later deposits, the pool mints the smaller of `amount0 * lp_supply / reserve0` and `amount1 * lp_supply / reserve1`. The amount of each token used rounds up in the pool's favor; the remainder returns to the provider.

## Burn

For a burn of `amount` LP tokens, the pool returns `amount * reserve0 / lp_supply` of token0 and `amount * reserve1 / lp_supply` of token1, then reduces `lp_supply`. A burn that would leave less than `MINIMUM_LIQUIDITY` reverts.

## Swap

The pool applies `protocol_fee_share_bps` to the input amount first, then applies `lp_fee_bps` to the remainder.

```
lp_amount = amount_in - amount_in * protocol_fee_share_bps / 10000
amount_in_with_fee = lp_amount * (10000 - lp_fee_bps)
amount_out = amount_in_with_fee * reserve_out / (reserve_in * 10000 + amount_in_with_fee)
```

The full input amount enters the vault. The output leaves the vault. The protocol fee is recorded in `protocol_pot`.

## quote_swap

`quote_swap` runs the swap computation without modifying state. It returns the amount out, the resulting reserves, and the protocol fee.

## Collect

`collect` moves the protocol pot contents to the protocol account the registry designates and resets the pot. An empty pot reverts.

A swap computes `lp_fee = amount_in * lp_fee_bps / 10000` and
`protocol_fee = lp_fee * protocol_fee_share_bps / 10000`. The pool's reserve
grows by `amount_in - protocol_fee`, the pot by `protocol_fee`, and the output is
computed from the full input with the LP fee weight, so the swapper's cost is the
LP fee tier and nothing more.

## Configuration

A pool has no configuration procedure. `lp_fee_bps`, `protocol_fee_share_bps`, `p2id_root`, and the registry pointer are written at account creation, the fee sum is checked there, and nothing changes them afterwards. The LP token key is derived from the pool's account id. The registry's `register` procedure rejects a pool whose fee rate or token pair disagrees with the registry, and a pool whose registry pointer is not that registry.

## Oracle

The oracle updates at the start of each mint, burn, and swap. It computes

```
p0 = (reserve1 << 64) / reserve0
p1 = (reserve0 << 64) / reserve1
```

and adds `p0 * elapsed` and `p1 * elapsed` to the two cumulative values, where `elapsed` is the block timestamp delta. The accumulators saturate at `u128::MAX`. A backward timestamp yields an `elapsed` of zero.

## Asset model

The LP token is a fungible asset issued by the pool account. No separate faucet account exists.

## Numeric bounds

Amounts, reserves, and `lp_supply` are u64 values bounded by `MAX_AMOUNT = 2^57`.

The binding constraint is the swap numerator, `amount_in_with_fee * reserve_out`. `amount_in_with_fee` is at most `amount_in * 10000`, so the worst case is `MAX_AMOUNT^2 * 10000`. This value must stay below `2^128`, which requires `MAX_AMOUNT < 2^64 / 100`. `2^57` satisfies the bound with margin.

Mint and burn products stay below `2^126`.

## Failure conditions

Each procedure reverts under these conditions.

Mint:

- pool has no LP token key
- deposit missing one token
- deposited amount above `MAX_AMOUNT`
- first deposit mints at most `MINIMUM_LIQUIDITY`
- later deposit mints zero liquidity
- liquidity out below `min_lp_out`

Burn:

- note carries multiple assets or a non-LP asset
- LP amount zero
- pool empty
- amount above `MAX_AMOUNT`
- burn leaves less than `MINIMUM_LIQUIDITY`
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

- protocol pot empty

Configure:

- sender differs from admin
- empty LP token key
- key collides with an existing token key
- fee sum 10000 or more

## Indexing

An indexer identifies pools by the account code root. It selects accounts whose code root equals the `miden-xyk` component root.

The Rust SDK (miden 0.14) has no event API. An indexer reconstructs activity from account vault deltas, input and output notes, and oracle deltas.

Public pool accounts expose full storage, so an indexer reads reserves, `lp_supply`, fees, and cumulative prices from the slots listed above.
