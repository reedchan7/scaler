//! Cross-platform host logical-core probe.
//!
//! Used by `core::summary::format_cpu_stats` so that when the user did NOT
//! pass `--cpu`, the cpu peak line still has a meaningful denominator
//! (the host's logical core count). The probe is best-effort: if we can't
//! read the count, we return `None` and the summary falls back to showing
//! the cores alone.
//!
//! Cached for the life of the process because this value does not change
//! between `scaler run` invocations.

use std::sync::OnceLock;

/// Returns the host's logical core count, cached after the first probe.
pub fn host_logical_cores() -> Option<u32> {
    static CACHE: OnceLock<Option<u32>> = OnceLock::new();
    *CACHE.get_or_init(query_host_logical_cores)
}

fn query_host_logical_cores() -> Option<u32> {
    std::thread::available_parallelism()
        .ok()
        .map(|n| u32::try_from(n.get()).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_logical_cores_returns_a_reasonable_value_on_this_host() {
        // On Linux and macOS dev boxes this probe should always succeed
        // and return between 1 and 4096 logical cores.
        let Some(cores) = host_logical_cores() else {
            // Skip silently on platforms where the probe fails — the test
            // still exercises the code path.
            return;
        };
        assert!(cores >= 1, "must report at least 1 core");
        assert!(cores < 4096, "more than 4096 cores is suspicious: {cores}");
    }
}
