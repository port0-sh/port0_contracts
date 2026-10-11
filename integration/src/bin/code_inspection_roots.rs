//! Prints the `CodeInspection` procedure roots for the pinned `miden-standards`
//! version.
//!
//! The registry contract needs the `get_code_commitment` root as a compile-time
//! constant to address the foreign procedure it calls over FPI. The root is a
//! four-felt `Word`; this binary prints both the hex form and the four limbs in
//! order so the constant is transcribed without an endianness mistake.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run -p integration --bin code_inspection_roots --release
//! ```

use miden_protocol::Word;
use miden_standards::account::inspection::CodeInspection;

fn print_root(name: &str, root: &Word) {
    let limbs: Vec<String> = root
        .as_elements()
        .iter()
        .map(|felt| format!("{}", felt.as_canonical_u64()))
        .collect();
    println!("{name}");
    println!("  hex:   {}", root.to_hex());
    println!("  limbs: [{}]", limbs.join(", "));
}

fn main() {
    print_root(
        "get_code_commitment",
        CodeInspection::get_code_commitment_root().mast_root(),
    );
    print_root(
        "has_procedure",
        CodeInspection::has_procedure_root().mast_root(),
    );
    print_root(
        "get_num_procedures",
        CodeInspection::get_num_procedures_root().mast_root(),
    );
    print_root(
        "get_procedure_root",
        CodeInspection::get_procedure_root_root().mast_root(),
    );
}
