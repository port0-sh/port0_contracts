# MidenXYK

A constant-product AMM for the Miden rollup, modeled on Uniswap V2. A single pool holds two tokens. Liquidity providers deposit both tokens and receive LP tokens. Traders swap one token for the other. A configured protocol fee accrues to a fee recipient.

## Concepts

- Pool: a single account that holds token0 and token1.
- LP token: a native fungible asset the pool mints to liquidity providers. It represents their share of the reserves.
- Roles:
  - Liquidity provider sends mint and burn notes.
  - Trader sends swap notes.
  - Admin sends the admin note to configure the pool.
  - Fee recipient sends the collect note.

## How you use it

You interact with the pool by sending notes to it. Each note script runs against the pool account and settles the operation.

1. Create the pool account. Its initial storage holds the token0 and token1 asset keys and the admin account id.
2. The admin sends one admin note to set fees, the fee recipient, the LP token key, and the P2ID note root.
3. Liquidity providers send mint notes to add liquidity and burn notes to remove it.
4. Traders send swap notes.
5. The fee recipient sends collect notes to withdraw protocol fees.

## Notes

The table lists each note, the assets it carries, and the felts it reads from note storage.

| Note | Assets | Note storage (felts, in order) |
|------|--------|-------------------------------|
| mint-note | token0, token1 | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_lp_out (1) |
| burn-note | LP token | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_amount0 (1), min_amount1 (1) |
| swap-note | token0 or token1 | pool id (2), recipient (2), serial (4), tag (1), deadline (1), min_amount_out (1) |
| collect-note | none | serial (4) |
| admin-note | none | pool id (2), protocol_fee_to (2), lp_fee_bps (1), protocol_fee_bps (1), p2id_root (4), LP token key (4) |

Account ids are two felts, prefix then suffix. `deadline` is a block timestamp; the note reverts if the pool executes it after that time. The `min_*` fields bound slippage.

### mint-note

Deposit token0 and token1. The pool mints LP tokens to the recipient. The first mint locks 1000 LP tokens permanently. If the deposited ratio differs from the pool ratio, the pool refunds the excess of one token to the recipient.

### burn-note

Burn LP tokens. The pool returns a proportional share of both reserves to the recipient.

### swap-note

Send one token. The pool pays out the other token, less fees. `min_amount_out` reverts the trade if the output would be smaller.

### collect-note

Withdraws the accumulated protocol fee to the configured fee recipient. Reverts if the fee pot is empty.

### admin-note

Configures the pool. Only the admin account can send it. Sets `lp_fee_bps`, `protocol_fee_bps`, the fee recipient, the LP token key, and the P2ID note root used for output notes.

## Quoting a swap

`quote_swap` is an account procedure you can call by FPI. It returns the amount out for a hypothetical swap without moving assets. It is the read-only version of the swap note.

## Fees

The admin configures two basis-point fees:

- `lp_fee_bps` goes to liquidity providers.
- `protocol_fee_bps` is taken from the input first; the LP fee applies to the remainder.

Their sum must stay below 10000. This differs from Uniswap V2, where the protocol fee is a fraction of the LP fee.

## Limits

- Amounts, reserves, and LP supply are u64 values capped at `MAX_AMOUNT = 2^57`. The cap keeps every u128 intermediate, including the swap numerator, below `u128::MAX`.
- The price oracle (cumulative prices and the last block timestamp) saturates at `u128::MAX` rather than wrapping.

## Repository layout

```
contracts/
  miden-xyk/      pool account component
  mint-note/      add-liquidity note script
  burn-note/      remove-liquidity note script
  swap-note/      swap note script
  collect-note/   protocol-fee collection note script
  admin-note/     pool configuration note script
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
