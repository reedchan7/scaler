use std::process::ExitStatus;
use std::time::{Duration, SystemTime};

use crate::core::{RunOutcome, SummarySample};

/// Total visual width of the summary card (in monospace columns). The
/// longest body row currently fits comfortably inside 38 inner columns,
/// which leaves two columns for the left `│` and right `│` borders.
const SUMMARY_WIDTH: usize = 40;
/// Inner column count between the `│` borders.
const INNER_WIDTH: usize = SUMMARY_WIDTH - 2;
/// Width of the label column inside the card body. The longest label is
/// `elapsed` (7 chars); values line up at column 11 (2-space indent +
/// 7-char label + 2-space gap).
const LABEL_WIDTH: usize = 7;
const INDENT: &str = "  ";
const HEADER_LABEL: &str = "scaler summary";

pub fn render(outcome: &RunOutcome) -> String {
    let mut lines = vec![render_header()];

    push_row(&mut lines, "exit", &format_exit_status(outcome.exit_status));
    push_row(&mut lines, "elapsed", &format_duration(outcome.elapsed));

    if let Some(peak) = outcome.peak_memory {
        push_row(&mut lines, "memory", &format!("max {}", format_bytes(peak)));
    }

    if let Some(stats) = cpu_stats(&outcome.samples) {
        push_row(
            &mut lines,
            "cpu",
            &format!(
                "avg {}, max {}",
                format_cores(stats.avg),
                format_cores(stats.max)
            ),
        );
    }

    lines.push(render_footer());
    lines.join("\n")
}

fn push_row(lines: &mut Vec<String>, label: &str, value: &str) {
    // Compose the content column (indent + label + value), then pad it
    // out to INNER_WIDTH so every row's trailing `│` lines up vertically.
    let content = format!("{INDENT}{label:<LABEL_WIDTH$}  {value}");
    lines.push(format!("│{content:<INNER_WIDTH$}│"));
}

fn render_header() -> String {
    // ┌─────────── scaler summary ───────────┐
    //
    // Label sits centered on the top rule between the two corner glyphs.
    //   - 1 column each for `┌` and `┐`
    //   - L columns for ` scaler summary ` (with surrounding spaces)
    //   - the remaining columns are split between left and right rules,
    //     with the right side getting the extra column when the split is odd
    let label = format!(" {HEADER_LABEL} ");
    let label_cols = label.chars().count();
    let total_rule_cols = INNER_WIDTH.saturating_sub(label_cols);
    let left_rule_cols = total_rule_cols / 2;
    let right_rule_cols = total_rule_cols - left_rule_cols;
    format!(
        "┌{}{label}{}┐",
        "─".repeat(left_rule_cols),
        "─".repeat(right_rule_cols),
    )
}

fn render_footer() -> String {
    // └──────────────────────────────────────┘
    format!("└{}┘", "─".repeat(INNER_WIDTH))
}

fn format_exit_status(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return code.to_string();
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }

    status.to_string()
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

pub fn format_duration(duration: Duration) -> String {
    format!("{:.3}s", duration.as_secs_f64())
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
    fn format_exit_status_prints_numeric_code_for_normal_exit() {
        let status = RunOutcome::fixture_for_test().exit_status;
        assert_eq!(format_exit_status(status), "0");
    }

    #[cfg(unix)]
    #[test]
    fn format_exit_status_reports_signal_number_when_terminated() {
        let status = exit_status_from_signal(9);
        assert_eq!(format_exit_status(status), "signal 9");
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
        let header = render_header();
        // Always opens with `┌` and closes with `┐`.
        assert!(header.starts_with('┌'));
        assert!(header.ends_with('┐'));
        // The label sits in the middle, surrounded by rule columns.
        assert!(header.contains(" scaler summary "));
        // Padded to exactly SUMMARY_WIDTH columns.
        assert_eq!(header.chars().count(), SUMMARY_WIDTH);
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
        let footer = render_footer();
        assert!(footer.starts_with('└'));
        assert!(footer.ends_with('┘'));
        assert_eq!(footer.chars().count(), SUMMARY_WIDTH);
        // Every column between the two corners is a horizontal rule.
        let inner: String = footer.chars().skip(1).take(INNER_WIDTH).collect();
        assert!(inner.chars().all(|c| c == '─'));
    }

    #[test]
    fn render_body_rows_fill_to_inner_width_with_vertical_borders() {
        let rendered = render(&RunOutcome::fixture_for_test());
        for line in rendered.lines() {
            // Every row — header, body, footer — is exactly SUMMARY_WIDTH
            // monospace columns wide.
            assert_eq!(
                line.chars().count(),
                SUMMARY_WIDTH,
                "line has wrong width: {line:?}",
            );
        }
        // At least one body row opens with │ and closes with │.
        assert!(
            rendered
                .lines()
                .any(|line| line.starts_with('│') && line.ends_with('│')),
            "expected at least one │...│ row",
        );
    }

    #[test]
    fn render_lays_out_aligned_label_block_with_cores_and_max() {
        let rendered = render(&RunOutcome::fixture_for_test());
        // Header + footer frame the body with all four corners.
        assert!(rendered.starts_with('┌'));
        assert!(rendered.contains(" scaler summary "));
        assert!(rendered.contains('┐'));
        assert!(rendered.ends_with('┘'));
        // Body rows: 2-space indent + 7-char label + 2 spaces + value,
        // wrapped in │...│.
        assert!(rendered.contains("  exit     0"));
        assert!(rendered.contains("  elapsed  3.000s"));
        assert!(rendered.contains("  memory   max 4.0 MiB"));
        // fixture has cpu_percent samples 12.5 and 25.0
        // -> avg 18.75 % = 0.19c, max 25.0 % = 0.25c
        assert!(rendered.contains("  cpu      avg 0.19c, max 0.25c"));
        // Old labels should not leak in.
        assert!(!rendered.contains("samples"));
        assert!(!rendered.contains("bytes"));
        assert!(!rendered.contains("runtime"));
        assert!(!rendered.contains('%'));
    }

    #[test]
    fn render_omits_cpu_row_when_no_samples() {
        let mut outcome = RunOutcome::fixture_for_test();
        outcome.samples.clear();
        let rendered = render(&outcome);
        assert!(rendered.contains("  exit     0"));
        assert!(rendered.contains("  memory   max 4.0 MiB"));
        // No "  cpu " row when there are no samples to average over.
        assert!(!rendered.contains("  cpu "));
        // Footer is still emitted so the card is properly closed.
        let last = rendered.lines().last().unwrap();
        assert!(last.starts_with('└') && last.ends_with('┘'));
    }

    #[test]
    fn render_omits_memory_row_when_peak_unknown() {
        let mut outcome = RunOutcome::fixture_for_test();
        outcome.peak_memory = None;
        let rendered = render(&outcome);
        assert!(rendered.contains("  exit     0"));
        assert!(!rendered.contains("  memory "));
        let last = rendered.lines().last().unwrap();
        assert!(last.starts_with('└') && last.ends_with('┘'));
    }
}
