//! Detached run management: launch commands in the background (Linux via
//! `systemd-run --no-block`, macOS via double-fork) and query their state
//! later via `scaler status`.

pub mod id;
pub mod state;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

use anyhow::Result;

use crate::core::LaunchPlan;
use crate::detach::id::RunId;
use crate::detach::state::StateRoot;

/// Launch the command in the background and return the new [`RunId`].
///
/// The caller is responsible for printing the id to stdout. Platform
/// dispatch: Linux uses `systemd-run --no-block` (Task 6); macOS uses
/// double-fork (Task 9).
pub fn launch(plan: &LaunchPlan) -> Result<RunId> {
    let root = StateRoot::from_env()?;
    platform_launch(plan, &root)
}

#[cfg(target_os = "linux")]
fn platform_launch(plan: &LaunchPlan, root: &StateRoot) -> Result<RunId> {
    linux::launch(plan, root)
}

#[cfg(target_os = "macos")]
fn platform_launch(_plan: &LaunchPlan, _root: &StateRoot) -> Result<RunId> {
    anyhow::bail!("macOS detach not yet implemented (Task 9)")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_launch(_plan: &LaunchPlan, _root: &StateRoot) -> Result<RunId> {
    anyhow::bail!("--detach is not supported on this platform")
}

pub fn query_one(root: &StateRoot, id: &RunId) -> Result<crate::cli::status::RunView> {
    platform_query_one(root, id)
}

pub fn query_all(root: &StateRoot) -> Result<Vec<crate::cli::status::RunView>> {
    platform_query_all(root)
}

#[cfg(target_os = "linux")]
fn platform_query_one(root: &StateRoot, id: &RunId) -> Result<crate::cli::status::RunView> {
    linux::query_one(root, id)
}

#[cfg(target_os = "linux")]
fn platform_query_all(root: &StateRoot) -> Result<Vec<crate::cli::status::RunView>> {
    linux::query_all(root)
}

#[cfg(target_os = "macos")]
fn platform_query_one(_root: &StateRoot, _id: &RunId) -> Result<crate::cli::status::RunView> {
    anyhow::bail!("macOS status query not yet implemented (Task 9)")
}

#[cfg(target_os = "macos")]
fn platform_query_all(_root: &StateRoot) -> Result<Vec<crate::cli::status::RunView>> {
    anyhow::bail!("macOS status query not yet implemented (Task 9)")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_query_one(_root: &StateRoot, _id: &RunId) -> Result<crate::cli::status::RunView> {
    anyhow::bail!("scaler status is not supported on this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_query_all(_root: &StateRoot) -> Result<Vec<crate::cli::status::RunView>> {
    anyhow::bail!("scaler status is not supported on this platform")
}
