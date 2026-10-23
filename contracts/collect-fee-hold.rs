// Fee a collect procedure reserves for the transaction's own epilogue charge, in
// the fee asset's base units.
//
// `collect` pays `vault - reserves - hold` before the kernel settles the
// transaction fee, so the fee asset needs a reserve when the transaction is not
// sponsored. The values are measured by
// `cargo test -p integration --release note_fee_benchmark` on the path the
// tooling sends, four asset keys, and the kernel prices a transaction at
// `base_fee * (log2(cycles rounded up to the next power of two) + 1)`.
//
// This file is the single source: both contracts include it, the integration
// benchmark includes it so its assertion compares against the enforced value,
// and `npm run generate` parses it into the SDK so the tooling prints the same
// numbers. Do not copy these values anywhere else; a test or a script that
// carries its own copy cannot fail for the reason it exists.
//
// Measured at verification base fee 7, the 0.17 testnet value.

/// Asset keys one collect note may carry.
///
/// The cap and the holds are coupled: each key raises the transaction's cycle
/// count, and the hold must cover the fee for the largest note the scripts
/// accept. Raising the cap is safe only together with a re-measurement; lowering
/// it makes the tooling's batches of this size invalid, so it moves here with
/// the holds.
#[allow(dead_code)]
const COLLECT_MAX_KEYS: usize = 4;

/// Pool collect, four asset keys including the fee asset: 125,270 cycles,
/// rounded to 2^17, priced at 7 * 18 = 126.
#[allow(dead_code)]
const POOL_COLLECT_FEE_HOLD: u64 = 126;

/// Registry collect, four asset keys including the fee asset: 64,799 cycles,
/// rounded to 2^16, priced at 7 * (16 + 1) = 119.
#[allow(dead_code)]
const REGISTRY_COLLECT_FEE_HOLD: u64 = 119;
