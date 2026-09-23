//! Paths and RPC selection shared by CVM, container, and native emulator runs.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Resolve an application data path under the native emulator's filesystem root.
/// Without an emulator root, retain the application's existing relative paths.
pub fn storage_path(path: &str) -> PathBuf {
    match std::env::var_os("EMULATOR_ROOTFS").filter(|value| !value.is_empty()) {
        Some(root) => {
            PathBuf::from(root).join(path.trim_start_matches("./").trim_start_matches('/'))
        }
        None => PathBuf::from(path),
    }
}

/// Use the emulator fork for local session verification, or the configured chain RPC.
pub fn session_registry_rpc_url() -> Result<String> {
    std::env::var("EMULATOR_RPC_URL")
        .or_else(|_| std::env::var("SESSION_REGISTRY_RPC_URL"))
        .context("Set SESSION_REGISTRY_RPC_URL, or use atakit emulator exec for local verification")
}
