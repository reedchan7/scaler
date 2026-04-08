//! Detached run management: launch commands in the background (Linux via
//! `systemd-run --no-block`, macOS via double-fork) and query their state
//! later via `scaler status`.

pub mod id;
pub mod state;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;
