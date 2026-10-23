//! Prints the storage slot ids of the pool's auth component.
//!
//! A pool exposes its own auth identity (the two note allowlist roots and the
//! fee schedule root) to the registry over foreign procedure invocation. The
//! contract addresses those slots by id, which is a hash of the slot name, so
//! the ids are compile-time constants in the pool contract. This prints the id
//! as `(suffix, prefix)` felts, the order `StorageSlotId::new` takes, so the
//! constants are transcribed without an endianness mistake.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run -p integration --bin auth_slot_ids --release
//! ```

use anyhow::{Context, Result};
use miden_protocol::account::StorageSlotName;

/// Slot names owned by `miden-standards` 0.17.1: the network auth allowlists
/// and the constant-fee policy's schedule.
const SLOTS: [&str; 7] = [
    "miden::standards::auth::network_account::allowed_note_scripts",
    "miden::standards::auth::network_account::allowed_tx_scripts",
    "miden::standards::fees::policies::basic_constant_fee::fee_schedule",
    "miden::standards::auth::network_account::fee_asset_id",
    "miden::standards::auth::network_account::active_fee_policy_proc_root",
    "miden::standards::auth::network_account::allowed_fee_policy_proc_roots",
    "miden::standards::faucets::fungible::token_config",
];

fn main() -> Result<()> {
    for name in SLOTS {
        let id = StorageSlotName::new(name)
            .with_context(|| format!("invalid slot name {name}"))?
            .id();
        println!("{name}");
        println!(
            "  suffix={} prefix={}",
            id.suffix().as_canonical_u64(),
            id.prefix().as_canonical_u64()
        );
    }
    Ok(())
}
