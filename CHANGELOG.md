# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-10-31

### Added

- The refund path. A spot note, or a limit note past its deadline, that cannot
  fill is consumed and the note's assets are paid back to the sender as one P2ID
  note: `reject` and `refund_to_sender` in the pool, with the reserves unmoved.
- The refund commitment in the `swap-note`, `mint-note`, and `burn-note` storage:
  the refund tag, the reclaimer, and the reclaim height. The swap and mint notes
  grow to 15 felts, the burn note to 16.
- `get_lifecycle` on the pool, and the registry's zero check over its fourteen
  felts, so a pool created with reserves, a pot, or a seeded oracle is rejected
  at registration.
- `write_lp_supply`, which records the circulating LP supply in the LP faucet's
  token config on mint and burn.
- Tests: the seeded-oracle rejection, the base-fee invariant sweep over 1, 7, 14
  and 30, and the ten-thousand-operation acceptance run, ignored by default.

### Changed

- The collect holds are 126 for the pool and the registry collect: the scheduled
  price, not the kernel fee.
- The registry's register path reads and checks the pool's lifecycle fields.

### Fixed

- The benchmark's collect-hold comment names the constants at 126 and 126.
- The comments that narrated earlier behaviour or earlier values.

### Removed

- `U128::as_word`, unused.

## [0.3.4] - 2026-10-30

Initial release.

### Added

- Account component `port0`: the constant-product pool. Two asset keys, a stored
  LP supply, the protocol pot, the LP fee rate and protocol fee share, the
  cumulative-price oracle, the last block timestamp, and the P2ID payout root.
- Account component `pool-registry`: accepted pool code commitments, the pool
  set, the asset-pair index, the protocol fee destination and share, the allowed
  LP fee tiers, and the admin register fee.
- Account component `network-faucet`: a fixed-supply faucet whose owner is
  immutable and whose whole supply is minted once, by the owner.
- Note scripts `mint-note`, `burn-note`, `swap-note`, `collect-note`,
  `register-note`, `registry-collect-note`, `registry-config-note`, and
  `launch-note`.
- The pool account's four components: `port0`, the standard basic fungible
  faucet that names the LP token, `CodeInspection`, and the network auth with the
  note-script allowlist, the fee schedule, and the identity roots.
- Registration checks on chain: the pool code commitment, the identity, the six
  auth storage roots, the P2ID payout root, the fee asset, the fee rates, and a
  duplicate pair.
- Fee model: LP fee tiers 5, 30 and 100 basis points, the protocol's share carved
  out of the LP fee, permissionless collects paid to the registry's fee
  destination, and the collect fee hold shared through `collect-fee-hold.rs`.
- The price oracle: cumulative prices and the block timestamp, written on every
  reserve change.
- A MockChain integration suite over the pool actions, the registry, the network
  faucet, and the launch path.
- `README.md` and `ARCHITECTURE.md`.
