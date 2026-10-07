//! Common helper functions for tests.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use miden_mast_package::Package;

/// Builds a Miden project in the specified directory.
///
/// # Arguments
/// * `dir` - Path to the directory containing the Cargo.toml
/// * `release` - Whether to build in release mode
///
/// # Returns
/// The compiled `Package`
///
/// # Errors
/// Returns an error if compilation fails or if the output is not in the expected format
pub fn build_project_in_dir(dir: &Path, release: bool) -> Result<Package> {
    let profile = if release { "--release" } else { "--debug" };
    let profile_name = if release { "release" } else { "debug" };
    let manifest_path = dir.join("Cargo.toml");
    let artifact_path = dir.join("target").join("miden").join(profile_name).join("out.masp");

    let args = vec![
        profile.to_string(),
        "-o".to_string(),
        artifact_path.display().to_string(),
        "--manifest-path".to_string(),
        manifest_path.display().to_string(),
    ];

    let status = miden_build(args).context("Failed to compile project")?;

    if !status.success() {
        bail!("Failed to compile project package. See output for details.");
    }

    let package_bytes = std::fs::read(&artifact_path)
        .context(format!("Failed to read compiled package from {}", artifact_path.display()))?;

    Package::read_from_bytes_trusted(&package_bytes).context("Failed to deserialize package from bytes")
}

fn miden_build(args: impl IntoIterator<Item = String>) -> anyhow::Result<std::process::ExitStatus> {
    let mut cmd = match std::env::var_os("MIDENUP_HOME") {
        Some(_) => std::process::Command::new("miden"),
        None => match std::env::var_os("CARGO_MIDEN") {
            Some(cargo_miden) => {
                // The `cargo-miden` binary expects the `miden` subcommand token,
                // the same as when cargo invokes it as `cargo miden`.
                let mut cmd = std::process::Command::new(cargo_miden);
                cmd.arg("miden");
                cmd
            }
            None => {
                let mut cmd = std::process::Command::new("cargo");
                cmd.arg("miden");
                cmd
            }
        },
    };
    cmd.arg("build").args(args);

    let mut child = cmd.spawn().map_err(|err| anyhow!("Failed to spawn build command: {err}"))?;

    child.wait().map_err(|err| anyhow!("Build command failed: {err}"))
}
