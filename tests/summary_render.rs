//! Integration tests for `core::summary::render`. These exercise the
//! public render entry point through `RunOutcome` so that summary.rs can
//! stay focused on its production code; the unit tests in summary.rs
//! still cover the private formatters (`format_*`, `cpu_stats`,
//! `render_header`, `render_footer`).

use std::time::Duration;

use scaler::core::{
    BackendKind, CapabilityLevel, CapabilityReport, Platform, RunOutcome, summary::render,
};

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
    // fixture has no mem_limit, so memory shows peak only (no parens).
    assert!(rendered.contains("  memory   max 4.0 MiB"));
    assert!(!rendered.contains("  memory   max 4.0 MiB ("));
    // fixture has cpu_percent samples 12.5 and 25.0
    // -> avg 18.75 % = 0.19c, max 25.0 % = 0.25c
    // Fixture has no cpu_limit and no host_logical_cores, so cpu row
    // shows just `avg <c>, max <c>` with no parenthesized denominator.
    assert!(rendered.contains("  cpu      avg 0.19c, max 0.25c"));
    // Old labels should not leak in.
    assert!(!rendered.contains("samples"));
    assert!(!rendered.contains("bytes"));
    assert!(!rendered.contains("runtime"));
}

#[test]
fn render_card_grows_to_fit_the_longest_body_row() {
    // MIN_INNER_WIDTH = 38 in src/core/summary.rs (internal layout policy).
    // Hardcoded here so summary.rs can keep MIN_INNER_WIDTH private.
    const MIN_INNER_WIDTH: usize = 38;

    // Set host_logical_cores so the cpu row gets a "% of Nc" suffix,
    // making it the widest row at roughly 45 columns — longer than
    // the MIN_INNER_WIDTH floor of 38.
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.host_logical_cores = Some(8);
    let rendered = render(&outcome);
    let width = rendered.lines().next().unwrap().chars().count();
    assert!(
        width > MIN_INNER_WIDTH + 2,
        "card did not grow to fit body: {width} columns",
    );
}

#[test]
fn render_card_falls_back_to_min_width_for_tiny_body() {
    // MIN_INNER_WIDTH = 38 in src/core/summary.rs (internal layout policy).
    // Hardcoded here so summary.rs can keep MIN_INNER_WIDTH private.
    const MIN_INNER_WIDTH: usize = 38;

    // Build a fixture with just exit + elapsed (no memory, no cpu).
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.peak_memory = None;
    outcome.samples.clear();
    outcome.elapsed = Duration::from_millis(12);
    let rendered = render(&outcome);
    let width = rendered.lines().next().unwrap().chars().count();
    assert_eq!(width, MIN_INNER_WIDTH + 2);
}

#[test]
fn render_body_rows_all_share_the_same_width() {
    let rendered = render(&RunOutcome::fixture_for_test());
    let mut widths = rendered.lines().map(|line| line.chars().count());
    let first = widths.next().unwrap();
    for width in widths {
        assert_eq!(
            width, first,
            "rows disagree on width: got {width}, expected {first}",
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

#[test]
fn render_shows_memory_percent_of_limit_when_set() {
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.peak_memory = Some(26 * 1024 * 1024);
    outcome.mem_limit_bytes = Some(256 * 1024 * 1024);
    let rendered = render(&outcome);
    assert!(rendered.contains("  memory   max 26.0 MiB (10.2% of 256.0 MiB)"));
}

#[test]
fn render_shows_memory_percent_of_host_when_no_limit() {
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.peak_memory = Some(26 * 1024 * 1024);
    outcome.mem_limit_bytes = None;
    outcome.system_memory_bytes = Some(16 * 1024 * 1024 * 1024);
    let rendered = render(&outcome);
    assert!(rendered.contains("  memory   max 26.0 MiB (0.2% of 16.0 GiB)"));
}

#[test]
fn render_omits_context_block_when_everything_enforced_and_no_warnings() {
    let outcome = RunOutcome::fixture_for_test();
    let rendered = render(&outcome);
    // Fixture defaults to fully-enforced capabilities and no warnings,
    // so render should produce ONLY the summary card — no `── scaler ──`
    // divider, no `backend` row, no `warning:` row.
    // Use `"── scaler ─"` to distinguish the divider line from the header
    // (`┌──── scaler summary ───┐` also contains `"── scaler"` so we need
    // the rule-after-space pattern unique to the divider).
    assert!(!rendered.contains("── scaler ─"));
    assert!(!rendered.contains("backend"));
    assert!(!rendered.contains("warning:"));
    // Card itself is still there.
    assert!(rendered.starts_with('┌'));
    assert!(rendered.contains(" scaler summary "));
}

#[test]
fn render_emits_context_block_when_capability_is_degraded() {
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.capabilities = CapabilityReport {
        platform: Platform::Macos,
        backend: BackendKind::MacosTaskpolicy,
        backend_state: CapabilityLevel::BestEffort,
        cpu: CapabilityLevel::BestEffort,
        memory: CapabilityLevel::BestEffort,
        interactive: CapabilityLevel::BestEffort,
        prerequisites: Vec::new(),
        warnings: Vec::new(),
    };
    let rendered = render(&outcome);
    // Divider precedes the context block.
    assert!(rendered.contains("── scaler ─"));
    // Each capability row appears with the [best_effort] tag.
    assert!(rendered.contains("backend"));
    assert!(rendered.contains("macos_taskpolicy"));
    assert!(rendered.contains("[best_effort]"));
    assert!(rendered.contains("cpu"));
    assert!(rendered.contains("memory"));
    assert!(rendered.contains("interactive"));
    // Card still renders below the context.
    assert!(rendered.contains(" scaler summary "));
}

#[test]
fn render_emits_warning_rows_when_warnings_present() {
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.warnings = vec![
        "monitor disabled: terminal too small".to_string(),
        "host probe failed".to_string(),
    ];
    let rendered = render(&outcome);
    // Even with everything enforced, warnings trigger the context
    // block (so users actually see the warning instead of having it
    // swallowed).
    assert!(rendered.contains("── scaler ─"));
    assert!(rendered.contains("warning: monitor disabled: terminal too small"));
    assert!(rendered.contains("warning: host probe failed"));
}

#[test]
fn render_facet_rows_show_value_text_without_tag() {
    // Facet rows (cpu/memory/interactive) carry the level as their
    // VALUE text, not as a separate tag. So a "best_effort" facet
    // renders as `interactive  best_effort` with no `[best_effort]`
    // suffix tag — the suffix would be redundant and noisy.
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.capabilities = CapabilityReport {
        platform: Platform::Linux,
        backend: BackendKind::LinuxSystemd,
        backend_state: CapabilityLevel::Enforced,
        cpu: CapabilityLevel::Enforced,
        memory: CapabilityLevel::Enforced,
        interactive: CapabilityLevel::BestEffort,
        prerequisites: Vec::new(),
        warnings: Vec::new(),
    };
    let rendered = render(&outcome);
    let interactive_line = rendered
        .lines()
        .find(|line| line.contains("interactive"))
        .expect("expected an `interactive` row");
    assert!(interactive_line.contains("best_effort"));
    // No bracketed tag on the facet row itself.
    assert!(!interactive_line.contains("[best_effort]"));
}

#[test]
fn render_backend_row_carries_level_tag() {
    // The backend row's value is the backend NAME (linux_systemd /
    // macos_taskpolicy / plain_fallback), so it needs the suffix tag
    // to communicate the enforcement level.
    let mut outcome = RunOutcome::fixture_for_test();
    outcome.capabilities = CapabilityReport {
        platform: Platform::Linux,
        backend: BackendKind::LinuxSystemd,
        backend_state: CapabilityLevel::Enforced,
        cpu: CapabilityLevel::Enforced,
        memory: CapabilityLevel::Enforced,
        // Trigger the context block by degrading at least one facet.
        interactive: CapabilityLevel::BestEffort,
        prerequisites: Vec::new(),
        warnings: Vec::new(),
    };
    let rendered = render(&outcome);
    let backend_line = rendered
        .lines()
        .find(|line| line.contains("backend") && line.contains("linux_systemd"))
        .expect("expected a `backend` row");
    assert!(backend_line.contains("[enforced]"));
}
