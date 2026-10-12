# MidenXYK

A constant-product automated market maker for the Miden rollup. A pool holds two
fungible assets and enforces `x * y = k` between their reserves. Liquidity
providers deposit both assets and receive LP tokens. Traders swap one asset for
the other. A protocol fee accrues to the registry's protocol account.
`ARCHITECTURE.md` describes the internal design.

## Components

| Contract | Kind | Purpose |
|----------|------|---------|
| `miden-xyk` | account component | Pool: reserves, LP supply, fee accounting, oracle |
| `pool-registry` | account component | Accepted pool code commitments, pool set, asset pair index, protocol fee policy |
| `mint-note` | note script | Add liquidity |
| `burn-note` | note script | Remove liquidity |
| `swap-note` | note script | Trade |
| `collect-note` | note script | Move a pool's accrued protocol fees to the protocol account |
| `register-note` | note script | Register a pool in the registry |
| `registry-collect-note` | note script | Move the registry's accumulated register fees to the protocol account |

A pool is a public network account. An operation is a network note sent to the
pool; the node's transaction builder consumes it and runs the pool procedure.
Registration is a network note sent to the registry.

## Concepts

- Pool: a single account that holds token0 and token1. Its fee rates, P2ID
  root, and registry pointer are initial storage and hold for its lifetime.
- Registry: a public network account that holds the accepted pool code
  commitments, the registered pools, the ordered asset pair index, and the
  protocol fee policy.
- LP token: a native fungible asset issued by the pool account. The asset key
  is derived from the pool's account id at run time, not stored. An amount `a`
  of LP tokens entitles the holder to the fraction `a / lp_supply` of the
  reserves.
- Roles:
  - Pool creator emits the register note to record the pool in the registry.
  - Liquidity provider emits mint and burn notes.
  - Trader emits swap notes.
  - Fee recipient emits collect notes against a pool, or registry-collect notes
    against the registry. Both are permissionless.

## Usage

1. Create the pool account. Its initial storage holds the token0 and token1
   asset keys, the two fee rates, the P2ID note root, and the registry pointer.
   The LP token key is derived from the pool's account id and is not stored.
2. Fund the pool with a note, then consume that note as the pool's first
   transaction. A network account requires every transaction to have an effect,
   so an empty scriptless deploy is rejected.
3. The creator emits the register note to record the pool in the registry.
4. Liquidity providers emit mint notes to add liquidity and burn notes to
   remove it.
5. Traders emit swap notes.
6. The fee recipient emits collect notes to withdraw a pool's protocol fees.
   Anyone may emit a registry-collect note to move the registry's accumulated
   register fees to the protocol account.

## Notes

The table lists each note, the assets it carries, and the felts it reads from
note storage.

| Note | Assets | Note storage (felts, in order) |
|------|--------|-------------------------------|
| mint-note | token0, token1 | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_lp_out (1) |
| burn-note | LP token | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_amount0 (1), min_amount1 (1) |
| swap-note | token0 or token1 | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_amount_out (1) |
| collect-note | none | serial (4) |
| register-note | none | registry id (2), pool id (2), asset0 (2), asset1 (2) |
| registry-collect-note | none | registry id (2), asset key (4), serial (4) |

Account ids are two felts, prefix then suffix. `deadline` is a block timestamp;
the note reverts if the pool executes it after that time. The `min_*` fields
bound slippage.

### mint-note

Deposit token0 and token1. The pool mints LP tokens to the recipient. On the
first mint the pool mints `sqrt(x * y)` LP tokens and retains
`MINIMUM_LIQUIDITY = 1000` permanently; the provider receives the remainder. If
the deposited ratio differs from the pool ratio, the pool refunds the excess of
one token to the recipient.

### burn-note

Burn LP tokens. The pool returns a proportional share of both reserves to the
recipient. The burn reverts when `amount_to_burn > lp_supply -
MINIMUM_LIQUIDITY` and when either payout rounds to zero.

### swap-note

Send one token. The pool pays out the other token, less fees. `min_amount_out`
reverts the trade if the output would be smaller.

### collect-note

Moves the pool's accrued protocol fee to the protocol account the registry
designates. Permissionless: the destination is fixed in the registry, so a
caller can only flush fees toward it. Reverts if the fee pot is empty.

### register-note

Registers the pool in the registry. The note targets the registry account and
carries the registry id, the pool id, and the two token faucet ids in note
storage. The registry's `register` procedure calls the pool's
`CodeInspection::get_code_commitment` and `get_registry_identity` by foreign
procedure invocation. It rejects a commitment that is not accepted, a pool id
or asset pair that is already registered, a pool that points at a different
registry, a pool whose token keys do not match the claimed pair, a pool whose
protocol fee rate differs from the registry's, and a pool whose LP fee is not
an allowed tier.

### registry-collect-note

Moves the registry's whole balance of one asset to the protocol account. The
note carries the registry id, the four-felt asset key, and a serial. Like a
pool collect, it is permissionless and the destination is fixed in storage.

## Quoting a swap

`quote_swap` is an account procedure callable by foreign procedure invocation.
It returns the amount out, the resulting reserves, and the protocol fee for a
given input without moving assets.

## Fees

A pool is created with two basis-point values, fixed for its lifetime:

| Value | Meaning |
|-------|---------|
| `lp_fee_bps` | The fee a swapper pays. |
| `protocol_fee_share_bps` | The protocol's share of that fee, in basis points of it. `1200` means 12% of the LP fee. |

The share is carved out of the LP fee, not added to the swapper's cost. A swap
computes `lp_fee = amount_in * lp_fee_bps / 10000` and
`protocol_fee = lp_fee * protocol_fee_share_bps / 10000`; the pool's reserve
grows by `amount_in - protocol_fee`, the pot grows by `protocol_fee`, and the
output is computed from the full input with the LP fee weight. The swapper
therefore pays the LP fee tier and nothing more. Uniswap V2 uses a share of
1667 (1/6); PancakeSwap uses 1200 (3/25). `lp_fee_bps` must be below 10000.

## Limits

- Amounts, reserves, and LP supply are u64 values capped at
  `MAX_AMOUNT = 2^57`. The cap keeps every u128 intermediate, including the
  swap numerator, below `u128::MAX`.
- The price oracle (cumulative prices and the last block timestamp) updates on
  mint, burn, and swap, and saturates the accumulators at `u128::MAX`.

## Repository layout

```
contracts/
  miden-xyk/                pool account component
  pool-registry/            registry account component
  mint-note/                add-liquidity note script
  burn-note/                remove-liquidity note script
  swap-note/                swap note script
  collect-note/             pool protocol-fee collection note script
  register-note/            pool registration note script
  registry-collect-note/    registry fee collection note script
integration/                MockChain integration tests and deployment binaries
```

## Build

Each contract is a standalone crate built with cargo-miden:

```
cargo miden build --manifest-path contracts/miden-xyk/Cargo.toml --release
```

## Test

```
cargo test -p integration --release
```

Tests run against MockChain, an in-process chain that compiles the contracts and
executes the note flows without a network.

## License

MIT. See `LICENSE`.
