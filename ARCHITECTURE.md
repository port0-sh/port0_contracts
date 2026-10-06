# Architecture

This document describes the internal design of MidenXYK. `README.md` describes the consumer-facing note interface.

## Overview

MidenXYK is a constant-product automated market maker. A single account holds two token reserves. Liquidity providers deposit both tokens and receive LP tokens. Traders exchange one token for the other. A fee recipient collects a protocol fee.

## Components

Six contracts compose the system.

- `miden-xyk`: the pool account component. Holds the reserves, the LP supply, and the fee accounting.
- `mint-note`: note script for adding liquidity.
- `burn-note`: note script for removing liquidity.
- `swap-note`: note script for trading.
- `collect-note`: note script for withdrawing protocol fees.
- `admin-note`: note script for pool configuration.

The pool component exposes `mint`, `burn`, `swap`, `quote_swap`, `collect`, `configure`, and the state getters.

## Storage

The pool component declares 11 value slots.

| Slot | Type | Encoding |
|------|------|----------|
| `tokenlp_asset_key` | Word | LP token asset key |
| `token0_asset_key` | Word | token0 asset key |
| `token1_asset_key` | Word | token1 asset key |
| `lp_supply` | Felt | `[supply, 0, 0, 0]` |
| `fees` | Word | `[lp_fee_bps, protocol_fee_bps, fee_to.prefix, fee_to.suffix]` |
| `admin` | Word | `[0, 0, suffix, prefix]` of the admin account id |
| `p2id_root` | Word | P2ID script root used for output notes |
| `protocol_pot` | Word | `[amount0, amount1, 0, 0]` |
| `price0_cumulative_last` | Word | u128, four 32-bit limbs, least significant first |
| `price1_cumulative_last` | Word | u128, four 32-bit limbs, least significant first |
| `block_timestamp_last` | Felt | `[timestamp, 0, 0, 0]` |

`token0_asset_key`, `token1_asset_key`, and `admin` are set at account creation. `configure` sets the remaining slots.

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

The pool applies `protocol_fee_bps` to the input amount first, then applies `lp_fee_bps` to the remainder.

```
lp_amount = amount_in - amount_in * protocol_fee_bps / 10000
amount_in_with_fee = lp_amount * (10000 - lp_fee_bps)
amount_out = amount_in_with_fee * reserve_out / (reserve_in * 10000 + amount_in_with_fee)
```

The full input amount enters the vault. The output leaves the vault. The protocol fee is recorded in `protocol_pot`.

## quote_swap

`quote_swap` runs the swap computation without modifying state. It returns the amount out, the resulting reserves, and the protocol fee.

## Collect

`collect` moves the protocol pot contents to `protocol_fee_to` and resets the pot. An empty pot reverts.

## Configure

`configure` sets `lp_fee_bps`, `protocol_fee_bps`, `protocol_fee_to`, `p2id_root`, and `tokenlp_asset_key`. The note sender must equal the stored admin account id. The fee sum must stay below 10000.

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
