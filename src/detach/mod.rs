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
