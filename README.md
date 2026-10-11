# MidenXYK

A constant-product AMM for the Miden rollup, modeled on Uniswap V2. A single pool holds two tokens. Liquidity providers deposit both tokens and receive LP tokens. Traders swap one token for the other. A protocol fee accrues to the registry's protocol account. `ARCHITECTURE.md` describes the internal design.

## Concepts

- Pool: a single account that holds token0 and token1.
- Registry: a public network account that holds the accepted pool code commitments, the registered pools, and an ordered asset pair index. A pool is recorded once, by a register note.
- LP token: a native fungible asset the pool mints on deposit and burns on withdrawal. An amount `a` of LP tokens entitles the holder to the fraction `a / lp_supply` of the reserves.
- Roles:
  - Pool creator sends the register note to record the pool in the registry.
  - Liquidity provider sends mint and burn notes.
  - Trader sends swap notes.
    - Fee recipient sends the collect note.

## Usage

All operations are notes sent to the pool account. Each note script runs against the pool and invokes one pool procedure.

1. Create the pool account. Its initial storage holds the token0 and token1 asset keys, the two fee rates, the P2ID note root, and the registry it reads its protocol fee destination from. The LP token key is derived from the pool's account id and is not stored.
2. The creator sends the register note to record the pool in the registry.
4. Liquidity providers send mint notes to add liquidity and burn notes to remove it.
5. Traders send swap notes.
6. The fee recipient sends collect notes to withdraw protocol fees.

## Notes

The table lists each note, the assets it carries, and the felts it reads from note storage.

| Note | Assets | Note storage (felts, in order) |
|------|--------|-------------------------------|
| mint-note | token0, token1 | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_lp_out (1) |
| burn-note | LP token | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_amount0 (1), min_amount1 (1) |
| swap-note | token0 or token1 | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_amount_out (1) |
| collect-note | none | serial (4) |
| register-note | none | registry id (2), pool id (2), asset0 (2), asset1 (2) |

Account ids are two felts, prefix then suffix. `deadline` is a block timestamp; the note reverts if the pool executes it after that time. The `min_*` fields bound slippage.

### mint-note

Deposit token0 and token1. The pool mints LP tokens to the recipient. The first mint locks 1000 LP tokens permanently. If the deposited ratio differs from the pool ratio, the pool refunds the excess of one token to the recipient.

### burn-note

Burn LP tokens. The pool returns a proportional share of both reserves to the recipient.

### swap-note

Send one token. The pool pays out the other token, less fees. `min_amount_out` reverts the trade if the output would be smaller.

### collect-note

Withdraws the accumulated protocol fee to the protocol account the registry designates. Permissionless: the destination is fixed in the registry, so a caller can only flush fees toward it. Reverts if the fee pot is empty.

### register-note

Registers the pool in the registry. The note targets the registry account and carries the registry id, the pool id, and the two token faucet ids in note storage. The registry's `register` procedure calls the pool's `CodeInspection::get_code_commitment` by foreign procedure invocation and rejects a commitment that is not in the accepted set, a pool id that is already registered, and an asset pair that is already registered.

## Quoting a swap

`quote_swap` is an account procedure callable by FPI. It returns the amount out, the resulting reserves, and the protocol fee for a given input without moving assets.

## Fees

The pool is created with two basis-point values, fixed for its lifetime:

- `lp_fee_bps` is the fee a swapper pays.
- `protocol_fee_share_bps` is the protocol's share of that fee, in basis points of it: `1200` means the protocol takes 12% of the LP fee.

The share is carved out of the LP fee, not added to the swapper's cost. A swap
pays the LP fee tier and the output does not depend on the share; the LP keeps
the fee minus the share, and the share accrues to the protocol pot. This matches
Uniswap V2, where the protocol's cut is a fraction of the LP fee.
`lp_fee_bps` must stay below 10000.

## Limits

- Amounts, reserves, and LP supply are u64 values capped at `MAX_AMOUNT = 2^57`. The cap keeps every u128 intermediate, including the swap numerator, below `u128::MAX`.
- The price oracle (cumulative prices and the last block timestamp) saturates at `u128::MAX`.

## Repository layout

```
contracts/
  miden-xyk/      pool account component
  pool-registry/  registry account component
  mint-note/      add-liquidity note script
  burn-note/      remove-liquidity note script
  swap-note/      swap note script
  collect-note/   protocol-fee collection note script
  register-note/  pool registration note script
integration/      MockChain integration tests
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

Tests run against MockChain, an in-process chain that compiles the contracts and executes the note flows without a network.

## License

MIT. See `LICENSE`.
