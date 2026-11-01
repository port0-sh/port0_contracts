# port0.sh contracts

Constant-product AMM contracts for the port0.sh exchange on Miden: the pool
account component, the registry account component, the network faucet component,
and the note scripts that operate them.

The consumer interface, the design, the fee model, and the security model are
documented in the book at https://docs.port0.sh/contracts/overview.html.
`ARCHITECTURE.md` in this repository carries the internal design.

## Components

| Contract | Kind | Purpose |
| --- | --- | --- |
| `port0` | account component | Pool: reserves, LP supply, fee accounting, oracle, refunds |
| `pool-registry` | account component | Pool set, accepted code commitments, fee policy, register fee |
| `network-faucet` | account component | Fixed-supply token faucet: immutable owner, one launch |
| `mint-note` | note script | Add liquidity |
| `burn-note` | note script | Remove liquidity |
| `swap-note` | note script | Trade |
| `collect-note` | note script | Move a pool's protocol fees to the protocol account |
| `register-note` | note script | Register a pool |
| `registry-collect-note` | note script | Move the registry's balance to the protocol account |
| `registry-config-note` | note script | Change the register fee, admin only |
| `launch-note` | note script | Mint a network faucet's whole supply, once |

## Build

Each contract is a standalone crate built with cargo-miden:

```sh
cargo miden build --manifest-path contracts/port0/Cargo.toml --release
```

## Test

```sh
cargo test -p integration --release
```

Tests run against MockChain, which compiles the contracts and executes the note
flows without a network.

## Repository layout

```
contracts/
  port0/                    pool account component
  pool-registry/            registry account component
  network-faucet/           fixed-supply token faucet component
  mint-note/                add-liquidity note script
  burn-note/                remove-liquidity note script
  swap-note/                swap note script
  collect-note/             pool protocol-fee collection note script
  register-note/            pool registration note script
  registry-collect-note/    registry fee collection note script
  registry-config-note/     register fee configuration note script
  launch-note/              network faucet launch note script
  collect-fee-hold.rs       shared collect fee reserve, included by both accounts
  token-config-slot.rs      shared faucet token config slot id
integration/                MockChain tests and deployment binaries
```

## Source

Radicle is the canonical source. GitHub is the mirror.

| Source | Location |
| --- | --- |
| Radicle | https://radicle.network/nodes/iris.radicle.network/rad%3Az2byaq213P9X1TgSh9N1b4dCG73VE |
| GitHub | https://github.com/port0-sh/port0_contracts |

## License

MIT. See `LICENSE`.
