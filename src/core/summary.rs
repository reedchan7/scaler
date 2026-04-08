use std::process::ExitStatus;
use std::time::{Duration, SystemTime};

use crate::core::{CapabilityLevel, CapabilityReport, RunOutcome, SummarySample};

/// Minimum inner column count for the summary card. Short commands (e.g.
/// `scaler -- echo hello`) have tiny body rows; without a floor the box
/// would shrink to about 15 columns and look cramped.
const MIN_INNER_WIDTH: usize = 38;
/// Width of the label column inside the card body. The longest label is
/// `elapsed` (7 chars); values line up at column 11 (2-space indent +
/// 7-char label + 2-space gap).
const LABEL_WIDTH: usize = 7;
/// Label column width for the capability context block. Sized to fit
/// `interactive` (11 chars). Separate from `LABEL_WIDTH` so changes to
/// the context block don't ripple into the body row test substrings.
const CONTEXT_LABEL_WIDTH: usize = 11;
const INDENT: &str = "  ";
const HEADER_LABEL: &str = "scaler summary";

pub fn render(outcome: &RunOutcome) -> String {
    // Build every body row as raw content first (no borders, no padding)
    // so we can measure the widest one before deciding the card width.
    let mut body: Vec<String> = Vec::with_capacity(4);
    body.push(build_row("exit", &format_exit_status(outcome.exit_status)));
    body.push(build_row("elapsed", &format_duration(outcome.elapsed)));
    if let Some(peak) = outcome.peak_memory {
        body.push(build_row(
            "memory",
            &format_memory(peak, outcome.mem_limit_bytes, outcome.system_memory_bytes),
        ));
    }
    if let Some(stats) = cpu_stats(&outcome.samples) {
        body.push(build_row(
            "cpu",
            &format_cpu_stats(
                stats,
                outcome.cpu_limit_centi_cores,
                outcome.host_logical_cores,
            ),
        ));
    }

    // Build context rows (capabilities + warnings) the same way so the
    // divider + context block + card all share one inner_width.
    let context_rows = build_context_rows(&outcome.capabilities, &outcome.warnings);

    // Inner width = max of (longest body row, longest context row,
    // header label + breathing room, MIN_INNER_WIDTH).
    let longest_body = body
        .iter()
        .chain(context_rows.iter())
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0);
    let header_label_cols = format!(" {HEADER_LABEL} ").chars().count();
    let header_breathing = header_label_cols + 6; // 3 rule chars on each side
    let inner_width = longest_body.max(header_breathing).max(MIN_INNER_WIDTH);

    let mut lines: Vec<String> = Vec::with_capacity(body.len() + context_rows.len() + 4);
    let outer_width = inner_width + 2;
    if !context_rows.is_empty() {
        // Divider matches the card's OUTER width (inner_width + 2 corner
        // columns) so it looks like a continuous frame above the card.
        // Context rows are also padded to outer_width so every line in
        // the rendered output occupies the same horizontal extent.
        let label = "── scaler ";
        let rule_cols = outer_width.saturating_sub(label.chars().count());
        lines.push(format!("{label}{}", "─".repeat(rule_cols)));
        for row in context_rows {
            lines.push(format!("{row:<outer_width$}"));
        }
    }
    lines.push(render_header(inner_width));
    for row in body {
        lines.push(format!("│{row:<inner_width$}│"));
    }
    lines.push(render_footer(inner_width));
    lines.join("\n")
}

/// Build the rows that make up the context block. Returns an empty vec
/// when everything is enforced and there are no warnings, so the caller
/// can suppress the divider entirely on the happy path.
fn build_context_rows(capabilities: &CapabilityReport, warnings: &[String]) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let any_degraded = !is_enforced(capabilities.backend_state)
        || !is_enforced(capabilities.cpu)
        || !is_enforced(capabilities.memory)
        || !is_enforced(capabilities.interactive);

    if !any_degraded && warnings.is_empty() {
        return rows;
    }

    // The `backend` row's value is a backend name (e.g. `macos_taskpolicy`),
    // so it needs the level tag suffix to convey enforcement state. The
    // other three facet rows have the level itself as their value
    // (`enforced` / `best-effort` / `unavailable`), so a suffix would be
    // redundant — we render them tag-free.
    rows.push(build_backend_row(
        capabilities.backend.as_str(),
        capabilities.backend_state,
    ));
    rows.push(build_context_row("cpu", capability_value(capabilities.cpu)));
    rows.push(build_context_row(
        "memory",
        capability_value(capabilities.memory),
    ));
    rows.push(build_context_row(
        "interactive",
        capability_value(capabilities.interactive),
    ));
    for warning in warnings {
        // Warning rows sit at indent level only (no label column padding)
        // so they stand out from the labeled facet rows above.
        rows.push(format!("{INDENT}warning: {warning}"));
    }
    rows
}

fn build_context_row(label: &str, value: &str) -> String {
    format!("{INDENT}{label:<CONTEXT_LABEL_WIDTH$}  {value}")
}

fn is_enforced(level: CapabilityLevel) -> bool {
    matches!(level, CapabilityLevel::Enforced)
}

fn capability_value(level: CapabilityLevel) -> &'static str {
    level.as_str()
}

fn build_backend_row(value: &str, level: CapabilityLevel) -> String {
    let suffix = format!("  [{}]", level.as_str());
    format!(
        "{INDENT}{label:<CONTEXT_LABEL_WIDTH$}  {value}{suffix}",
        label = "backend"
    )
}

fn build_row(label: &str, value: &str) -> String {
    format!("{INDENT}{label:<LABEL_WIDTH$}  {value}")
}

fn render_header(inner_width: usize) -> String {
    // ┌─────────── scaler summary ───────────┐
    //
    // Label centered on the top rule between the two corner glyphs. The
    // right side gets the extra column when the rule-space is odd.
    let label = format!(" {HEADER_LABEL} ");
    let label_cols = label.chars().count();
    let total_rule_cols = inner_width.saturating_sub(label_cols);
    let left_rule_cols = total_rule_cols / 2;
    let right_rule_cols = total_rule_cols - left_rule_cols;
    format!(
        "┌{}{label}{}┐",
        "─".repeat(left_rule_cols),
        "─".repeat(right_rule_cols),
    )
}

fn render_footer(inner_width: usize) -> String {
    // └──────────────────────────────────────┘
    format!("└{}┘", "─".repeat(inner_width))
}

/// Memory row value. The peak is always shown in human units. The
/// parenthesized percent uses the tightest relevant denominator:
///
///   1. If the user passed `--mem N`, percent is relative to N.
///      `max 26.4 MiB (10.3% of 256.0 MiB)`
///   2. Otherwise, if we know the host's physical memory, percent is
///      relative to that.
///      `max 26.4 MiB (0.4% of 16.0 GiB)`
///   3. If neither is available, we fall back to the peak alone.
///      `max 26.4 MiB`
///
/// Zero denominators are treated as "unknown" so a defensive
/// `Some(0)` never causes a divide-by-zero.
fn format_memory(peak: u64, limit: Option<u64>, system_total: Option<u64>) -> String {
    let head = format!("max {}", format_bytes(peak));
    if let Some(limit_bytes) = limit.filter(|&n| n > 0) {
        let percent = (peak as f64 / limit_bytes as f64) * 100.0;
        return format!("{head} ({percent:.1}% of {})", format_bytes(limit_bytes));
    }
    if let Some(total_bytes) = system_total.filter(|&n| n > 0) {
        let percent = (peak as f64 / total_bytes as f64) * 100.0;
        return format!("{head} ({percent:.1}% of {})", format_bytes(total_bytes));
    }
    head
}

/// `avg <Nc>, max <Nc> (<P %> of <denom>)` — cores first, then a tight
/// percent of either the user's `--cpu` budget (if set) or the host's
/// logical core count (if probed). Falls back to the bare `avg/max` form
/// when neither denominator is available, mirroring `format_memory`.
fn format_cpu_stats(
    stats: CpuStats,
    cpu_limit_centi_cores: Option<u32>,
    host_logical_cores: Option<u32>,
) -> String {
    let avg_cores = format_cores(stats.avg);
    let max_cores = format_cores(stats.max);
    let max_in_cores = (stats.max / 100.0).max(0.0);
    let head = format!("avg {avg_cores}, max {max_cores}");

    if let Some(limit_centi) = cpu_limit_centi_cores.filter(|&n| n > 0) {
        let limit_cores = limit_centi as f32 / 100.0;
        let percent = (max_in_cores / limit_cores) * 100.0;
        return format!("{head} ({percent:.1}% of {limit_cores:.2}c)");
    }
    if let Some(host) = host_logical_cores.filter(|&n| n > 0) {
        let percent = (max_in_cores / host as f32) * 100.0;
        return format!("{head} ({percent:.1}% of {host}c)");
    }
    head
}

fn format_exit_status(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        if code == 0 {
            return "0 (success)".to_string();
        }
        // Shell convention: codes 129..=159 mean "terminated by signal N"
        // where N = code - 128. Decode the well-known signals; everything
        // else falls back to a generic `(failure, signal N)` label.
        if (129..=159).contains(&code) {
            let signal = code - 128;
            if let Some(name) = signal_name(signal) {
                let verb = signal_verb(signal);
                return format!("{code} ({verb} by {name})");
            }
            return format!("{code} (failure, signal {signal})");
        }
        return format!("{code} (failure)");
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            if let Some(name) = signal_name(signal) {
                return format!("signal {signal} ({name})");
            }
            return format!("signal {signal}");
        }
    }

    status.to_string()
}

/// Maps the 8 signal numbers most likely to terminate a wrapped command
/// to their canonical names. We deliberately do not enumerate the entire
/// `signal(7)` table — anything outside this set falls back to a numeric
/// label so the user still has *something* to grep for.
fn signal_name(signal: i32) -> Option<&'static str> {
    match signal {
        1 => Some("SIGHUP"),
        2 => Some("SIGINT"),
        3 => Some("SIGQUIT"),
        6 => Some("SIGABRT"),
        9 => Some("SIGKILL"),
        11 => Some("SIGSEGV"),
        13 => Some("SIGPIPE"),
        15 => Some("SIGTERM"),
        _ => None,
    }
}

/// English verb for each signal in the `signal_name` table. Used so the
/// rendered exit row reads like `137 (killed by SIGKILL)` instead of the
/// awkward `137 (terminated by SIGKILL)`.
fn signal_verb(signal: i32) -> &'static str {
    match signal {
        1 => "hangup",
        2 => "interrupted",
        3 => "quit",
        6 => "aborted",
        9 => "killed",
        11 => "segfaulted",
        13 => "pipe-broken",
        15 => "terminated",
        _ => "terminated",
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Human-readable duration formatter with unit changes that match how
/// people talk about command runtimes:
///
///   < 1 s    → `29ms`       (integer milliseconds)
///   < 1 min  → `3.260s`     (3-decimal seconds, unchanged from 0.2.x)
///   < 1 hour → `2m 15.4s`   (minutes + one-decimal seconds)
///   ≥ 1 hour → `1h 23m 45s` (hours + minutes + integer seconds)
pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs_f64();
    if total_seconds < 1.0 {
        return format!("{}ms", duration.as_millis());
    }
    if total_seconds < 60.0 {
        return format!("{total_seconds:.3}s");
    }
    if total_seconds < 3600.0 {
        let minutes = (total_seconds / 60.0).floor() as u64;
        let seconds = total_seconds - (minutes * 60) as f64;
        return format!("{minutes}m {seconds:.1}s");
    }
    let hours = (total_seconds / 3600.0).floor() as u64;
    let after_hours = total_seconds - (hours * 3600) as f64;
    let minutes = (after_hours / 60.0).floor() as u64;
    let seconds = after_hours - (minutes * 60) as f64;
    format!("{hours}h {minutes}m {seconds:.0}s")
}

/// Convert a `ps`-style cpu percentage to logical cores. 100 % == 1 core,
/// so the formatter divides by 100 and prints two decimals with a `c`
/// suffix to mirror the `--cpu Nc` flag the user typed.
fn format_cores(percent: f32) -> String {
    let cores = (percent / 100.0).max(0.0);
    format!("{cores:.2}c")
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CpuStats {
    avg: f32,
    max: f32,
}

fn cpu_stats(samples: &[SummarySample]) -> Option<CpuStats> {
    if samples.is_empty() {
        return None;
    }
    let sum: f32 = samples.iter().map(|sample| sample.cpu_percent).sum();
    let avg = sum / samples.len() as f32;
    let max = samples
        .iter()
        .map(|sample| sample.cpu_percent)
        .fold(f32::NEG_INFINITY, f32::max);
    Some(CpuStats { avg, max })
}

impl RunOutcome {
    pub fn fixture_for_test() -> Self {
        Self {
            exit_status: success_exit_status(),
            elapsed: Duration::from_secs(3),
            peak_memory: Some(4_194_304),
            mem_limit_bytes: None,
            system_memory_bytes: None,
            samples: vec![
                SummarySample {
                    captured_at: SystemTime::UNIX_EPOCH,
                    cpu_percent: 12.5,
                    memory_bytes: 2_097_152,
                },
                SummarySample {
                    captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                    cpu_percent: 25.0,
                    memory_bytes: 4_194_304,
                },
            ],
            cpu_limit_centi_cores: None,
            host_logical_cores: None,
            capabilities: crate::core::CapabilityReport::fully_enforced_for_test(),
            warnings: Vec::new(),
            total_cpu_nanos: None,
        }
    }
}

#[cfg(unix)]
fn success_exit_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;

    ExitStatus::from_raw(0)
}

#[cfg(windows)]
fn success_exit_status() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;

    ExitStatus::from_raw(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn exit_status_from_signal(signal: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(signal)
    }

    #[test]
    fn format_bytes_scales_to_human_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(format_bytes(1_572_864), "1.5 MiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }

    #[test]
    fn format_memory_with_neither_limit_nor_host_shows_peak_only() {
        assert_eq!(format_memory(4_194_304, None, None), "max 4.0 MiB");
        assert_eq!(format_memory(512, None, None), "max 512 B");
        assert_eq!(format_memory(1_610_612_736, None, None), "max 1.5 GiB");
    }

    #[test]
    fn format_memory_with_limit_shows_percent_of_limit() {
        // Peak 26 MiB of a 256 MiB budget -> about 10.2 %.
        assert_eq!(
            format_memory(
                26 * 1024 * 1024,
                Some(256 * 1024 * 1024),
                // Host total should be ignored once a --mem limit is
                // present, so pick a value that would give a wildly
                // different percentage if it were used by mistake.
                Some(16 * 1024 * 1024 * 1024),
            ),
            "max 26.0 MiB (10.2% of 256.0 MiB)"
        );
        // Peak exactly equal to the limit.
        assert_eq!(
            format_memory(64 * 1024 * 1024, Some(64 * 1024 * 1024), None),
            "max 64.0 MiB (100.0% of 64.0 MiB)"
        );
        // Over-limit: sampler can catch the process after it overshot
        // MemoryMax but before the cgroup killed it.
        assert_eq!(
            format_memory(70 * 1024 * 1024, Some(64 * 1024 * 1024), None),
            "max 70.0 MiB (109.4% of 64.0 MiB)"
        );
    }

    #[test]
    fn format_memory_without_limit_falls_back_to_percent_of_host() {
        // 26 MiB of a 16 GiB host -> ~0.16 %.
        assert_eq!(
            format_memory(26 * 1024 * 1024, None, Some(16 * 1024 * 1024 * 1024)),
            "max 26.0 MiB (0.2% of 16.0 GiB)"
        );
        // A process using a third of the host memory.
        assert_eq!(
            format_memory(2 * 1024 * 1024 * 1024, None, Some(6 * 1024 * 1024 * 1024),),
            "max 2.0 GiB (33.3% of 6.0 GiB)"
        );
    }

    #[test]
    fn format_memory_treats_zero_denominators_as_unknown() {
        // `clap` would reject --mem 0 at parse time, but the formatter
        // should still not divide by zero if we ever get here by accident.
        assert_eq!(format_memory(4_194_304, Some(0), None), "max 4.0 MiB");
        // Same for a broken host probe that returns Some(0).
        assert_eq!(format_memory(4_194_304, None, Some(0)), "max 4.0 MiB");
    }

    #[test]
    fn format_cpu_stats_with_no_denominator_shows_avg_and_max_only() {
        assert_eq!(
            format_cpu_stats(CpuStats { avg: 0.0, max: 0.0 }, None, None),
            "avg 0.00c, max 0.00c"
        );
        assert_eq!(
            format_cpu_stats(
                CpuStats {
                    avg: 18.75,
                    max: 25.0,
                },
                None,
                None,
            ),
            "avg 0.19c, max 0.25c"
        );
    }

    #[test]
    fn format_cpu_stats_with_host_cores_appends_percent_of_host() {
        // 25 % of one core = 0.25 cores. On an 8-core host that's 3.1 %.
        assert_eq!(
            format_cpu_stats(
                CpuStats {
                    avg: 18.75,
                    max: 25.0,
                },
                None,
                Some(8),
            ),
            "avg 0.19c, max 0.25c (3.1% of 8c)"
        );
    }

    #[test]
    fn format_cpu_stats_with_cpu_limit_uses_limit_as_denominator() {
        // 0.45 cores against a 0.50c limit = 90 %. Host cores should be
        // ignored when the user passed a limit.
        assert_eq!(
            format_cpu_stats(
                CpuStats {
                    avg: 30.0,
                    max: 45.0,
                },
                Some(50), // 0.50c in centi-cores
                Some(8),
            ),
            "avg 0.30c, max 0.45c (90.0% of 0.50c)"
        );
    }

    #[test]
    fn format_cpu_stats_treats_zero_denominators_as_unknown() {
        assert_eq!(
            format_cpu_stats(
                CpuStats {
                    avg: 5.0,
                    max: 10.0
                },
                Some(0),
                None
            ),
            "avg 0.05c, max 0.10c"
        );
        assert_eq!(
            format_cpu_stats(
                CpuStats {
                    avg: 5.0,
                    max: 10.0
                },
                None,
                Some(0)
            ),
            "avg 0.05c, max 0.10c"
        );
    }

    #[test]
    fn format_duration_sub_second_shows_integer_milliseconds() {
        assert_eq!(format_duration(Duration::from_millis(0)), "0ms");
        assert_eq!(format_duration(Duration::from_millis(29)), "29ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn format_duration_seconds_range_uses_three_decimals() {
        assert_eq!(format_duration(Duration::from_millis(1000)), "1.000s");
        assert_eq!(format_duration(Duration::from_millis(3260)), "3.260s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59.000s");
    }

    #[test]
    fn format_duration_minutes_range_uses_m_and_one_decimal() {
        assert_eq!(format_duration(Duration::from_secs(60)), "1m 0.0s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2m 5.0s");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59m 59.0s");
    }

    #[test]
    fn format_duration_hours_range_uses_hms_with_integer_seconds() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h 0m 0s");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1h 2m 5s");
        assert_eq!(format_duration(Duration::from_secs(7385)), "2h 3m 5s");
    }

    #[test]
    fn format_cores_converts_percentage_to_two_decimal_cores() {
        assert_eq!(format_cores(0.0), "0.00c");
        assert_eq!(format_cores(5.6), "0.06c");
        assert_eq!(format_cores(45.0), "0.45c");
        assert_eq!(format_cores(100.0), "1.00c");
        assert_eq!(format_cores(250.0), "2.50c");
    }

    #[test]
    fn format_cores_clamps_negative_values_to_zero() {
        assert_eq!(format_cores(-1.0), "0.00c");
    }

    #[test]
    fn format_exit_status_zero_reads_as_success() {
        let status = RunOutcome::fixture_for_test().exit_status;
        assert_eq!(format_exit_status(status), "0 (success)");
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_status_nonzero_reads_as_failure() {
        use std::os::unix::process::ExitStatusExt;
        // Code 1 (raw 256 because exit() shifts left by 8 on unix wait status).
        let status = ExitStatus::from_raw(1 << 8);
        assert_eq!(format_exit_status(status), "1 (failure)");
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_status_decodes_known_signal_codes() {
        use std::os::unix::process::ExitStatusExt;
        // Code 128 + 2 (SIGINT) — process exited via the shell convention.
        let status = ExitStatus::from_raw((128 + 2) << 8);
        assert_eq!(format_exit_status(status), "130 (interrupted by SIGINT)");

        let status = ExitStatus::from_raw((128 + 9) << 8);
        assert_eq!(format_exit_status(status), "137 (killed by SIGKILL)");

        let status = ExitStatus::from_raw((128 + 15) << 8);
        assert_eq!(format_exit_status(status), "143 (terminated by SIGTERM)");
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_status_unknown_signal_code_falls_back_to_failure() {
        use std::os::unix::process::ExitStatusExt;
        // Code 128 + 4 (SIGILL) — not in our 8-signal table.
        let status = ExitStatus::from_raw((128 + 4) << 8);
        assert_eq!(format_exit_status(status), "132 (failure, signal 4)");
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_status_reports_signal_number_with_name_when_terminated() {
        let status = exit_status_from_signal(9);
        assert_eq!(format_exit_status(status), "signal 9 (SIGKILL)");
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_status_reports_unknown_signal_number_without_name() {
        let status = exit_status_from_signal(31);
        assert_eq!(format_exit_status(status), "signal 31");
    }

    #[test]
    fn cpu_stats_returns_avg_and_max() {
        let samples = vec![
            SummarySample {
                captured_at: SystemTime::UNIX_EPOCH,
                cpu_percent: 12.5,
                memory_bytes: 0,
            },
            SummarySample {
                captured_at: SystemTime::UNIX_EPOCH,
                cpu_percent: 25.0,
                memory_bytes: 0,
            },
            SummarySample {
                captured_at: SystemTime::UNIX_EPOCH,
                cpu_percent: 7.5,
                memory_bytes: 0,
            },
        ];
        let stats = cpu_stats(&samples).unwrap();
        assert!((stats.avg - 15.0).abs() < 1e-3);
        assert!((stats.max - 25.0).abs() < 1e-3);
    }

    #[test]
    fn cpu_stats_returns_none_for_empty_samples() {
        assert!(cpu_stats(&[]).is_none());
    }

    #[test]
    fn render_header_centers_label_between_corners() {
        let inner_width = 38;
        let header = render_header(inner_width);
        // Always opens with `┌` and closes with `┐`.
        assert!(header.starts_with('┌'));
        assert!(header.ends_with('┐'));
        // The label sits in the middle, surrounded by rule columns.
        assert!(header.contains(" scaler summary "));
        // Padded to exactly inner_width + 2 (for the corner glyphs).
        assert_eq!(header.chars().count(), inner_width + 2);
        // Sanity-check that the label is roughly centered: the gap between
        // the left corner and the label should be within one column of the
        // gap between the label and the right corner.
        let label_start = header.find(" scaler summary ").unwrap();
        let left_gap = header[..label_start].chars().count(); // includes ┌
        let label_end = label_start + " scaler summary ".len();
        let right_gap = header[label_end..].chars().count(); // includes ┐
        let diff = (left_gap as isize - right_gap as isize).abs();
        assert!(
            diff <= 1,
            "label is not centered: left={left_gap}, right={right_gap}",
        );
    }

    #[test]
    fn render_footer_is_corners_around_full_rule() {
        let inner_width = 38;
        let footer = render_footer(inner_width);
        assert!(footer.starts_with('└'));
        assert!(footer.ends_with('┘'));
        assert_eq!(footer.chars().count(), inner_width + 2);
        // Every column between the two corners is a horizontal rule.
        let inner: String = footer.chars().skip(1).take(inner_width).collect();
        assert!(inner.chars().all(|c| c == '─'));
    }
}
