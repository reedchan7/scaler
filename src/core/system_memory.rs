//! Cross-platform total-memory probe for the host.
//!
//! Used by `core::summary::format_memory` so that when the user did NOT
//! pass `--mem`, the peak memory line can still be shown as a percent of
//! something meaningful (the host's physical RAM) instead of a bare
//! number. The probe is best-effort: if we can't read the total, we
//! return `None` and the summary falls back to showing the peak alone.
//!
//! The result is cached for the life of the process because this value
//! does not change between `scaler run` invocations — and even a short
//! command currently triggers exactly one `sample_process_tree` that
//! would otherwise re-probe.

use std::sync::OnceLock;

/// Returns the host's total physical memory in bytes, cached after the
/// first successful probe.
pub fn total_memory_bytes() -> Option<u64> {
    static CACHE: OnceLock<Option<u64>> = OnceLock::new();
    *CACHE.get_or_init(query_total_memory)
}

#[cfg(target_os = "linux")]
fn query_total_memory() -> Option<u64> {
    // /proc/meminfo has a line like `MemTotal:       16338204 kB`. Parse
    // the first number, convert kibibytes to bytes.
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return kib.checked_mul(1024);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn query_total_memory() -> Option<u64> {
    // `sysctl -n hw.memsize` prints the physical memory in bytes on a
    // single line, e.g. `17179869184`. We shell out instead of calling
    // libc::sysctlbyname so we don't need a new dependency or unsafe
    // block for a value we only read once at the end of a run.
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn query_total_memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_memory_bytes_returns_a_reasonable_value_on_this_host() {
        // On macOS and Linux dev machines this probe should succeed and
        // return at least 256 MiB (we really hope so) and less than a
        // petabyte (ditto).
        let Some(bytes) = total_memory_bytes() else {
            // Skip silently on platforms where we haven't implemented a
            // probe — the test still serves to exercise the code path.
            return;
        };
        assert!(
            bytes >= 256 * 1024 * 1024,
            "total memory too small: {bytes}"
        );
        assert!(
            bytes < 1024 * 1024 * 1024 * 1024 * 1024,
            "total memory too big: {bytes}",
        );
    }
}
