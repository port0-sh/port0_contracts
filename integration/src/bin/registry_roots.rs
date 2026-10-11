//! Prints a contract component's procedure roots.
//!
//! A pool addresses the registry's procedures over foreign procedure
//! invocation, which takes a MAST root, so the roots have to be compile-time
//! constants in the pool contract. The root is a four-felt `Word`; this prints
//! both the hex form and the four limbs in order, so the constant is
//! transcribed without an endianness mistake.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run -p integration --bin registry_roots --release -- [contract-dir]
//! ```

use std::path::Path;

use anyhow::Result;
use integration::helpers::build_project_in_dir;
use miden_mast_package::PackageExport;

fn main() -> Result<()> {
    // Resolved against the crate directory so the binary works whether it is run
    // from the workspace root or from `integration/`. Defaults to the registry;
    // pass a contract directory name to print another contract's roots.
    let contract = std::env::args().nth(1).unwrap_or_else(|| "pool-registry".into());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts").join(contract);
    let package = build_project_in_dir(&dir, true)?;
    let mut exports: Vec<_> = package
        .manifest
        .exports()
        .filter_map(|export| match export {
            PackageExport::Procedure(procedure) => Some(procedure),
            _ => None,
        })
        .collect();
    exports.sort_by(|a, b| a.path.cmp(&b.path));

    for procedure in exports {
        let limbs: Vec<String> = procedure
            .digest
            .as_elements()
            .iter()
            .map(|felt| felt.as_canonical_u64().to_string())
            .collect();
        // `Path` has no `Display`; take the trailing segment of its debug form.
        let path = format!("{:?}", procedure.path);
        let name = path.rsplit("::").next().unwrap_or(&path).trim_matches('"');
        println!("{name}");
        println!("  hex:   {}", procedure.digest.to_hex());
        println!("  limbs: [{}]", limbs.join(", "));
    }
    Ok(())
}
