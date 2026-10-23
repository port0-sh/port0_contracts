//! Prints the script roots of the standard notes the contracts compare against.
//!
//! A pool's collect procedure decides whether to reserve the transaction fee by
//! looking for a fee-sponsorship note among the transaction's input notes, so
//! that note's script root is a compile-time constant. The registry compares a
//! pool's payout script root against its own P2ID root, which this prints too.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run -p integration --bin standards_roots --release
//! ```

use anyhow::Result;
use miden_protocol::Word;
use miden_standards::note::{FeeSponsorshipNote, P2idNote};

fn main() -> Result<()> {
    for (name, root) in [
        ("fee-sponsorship", Word::from(FeeSponsorshipNote::script_root())),
        ("p2id", Word::from(P2idNote::script_root())),
    ] {
        let limbs: Vec<String> = root
            .as_elements()
            .iter()
            .map(|felt| felt.as_canonical_u64().to_string())
            .collect();
        println!("{name}");
        println!("  hex:   {}", root.to_hex());
        println!("  limbs: [{}]", limbs.join(", "));
    }
    Ok(())
}
