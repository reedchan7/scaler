use std::{ffi::OsString, time::Duration};

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use scaler::core::{
    InteractiveMode, LaunchPlan, OutputStream, Platform, ResourceSpec, RunOutcome,
    output::{OutputCollector, next_sequence},
    run_loop::{
        PlainFallbackBackend, execute, record_interactive_mode_for_test,
        record_monitor_fallback_for_test, record_post_launch_monitor_failure_for_test,
        record_summary_timeline_for_test, reset_test_state, set_test_poll_interval_for_next_run,
        take_output_frames_for_test,
    },
    summary,
};

#[test]
fn pipe_frames_keep_per_stream_order() {
    let mut sequence = 0;

    assert_eq!(next_sequence(&mut sequence), 1);
    assert_eq!(next_sequence(&mut sequence), 2);

    let mut collector = OutputCollector::default();
    let stdout_first = collector.push_stdout(b"first");
    let stdout_second = collector.push_stdout(b"second");

    assert_eq!(stdout_first.sequence + 1, stdout_second.sequence);
    assert_eq!(stdout_first.stream, OutputStream::Stdout);
    assert_eq!(stdout_second.stream, OutputStream::Stdout);
    assert_eq!(stdout_first.bytes, b"first");
    assert_eq!(stdout_second.bytes, b"second");
}

#[test]
fn stderr_and_pty_frames_are_tagged_correctly() {
    let mut collector = OutputCollector::default();

    let stderr = collector.push_stderr(b"problem");
    let pty = collector.push_pty(b"prompt");

    assert_eq!(stderr.stream, OutputStream::Stderr);
    assert_eq!(pty.stream, OutputStream::PtyMerged);
    assert!(stderr.sequence < pty.sequence);
}

#[test]
fn summary_renderer_includes_status_and_peak_memory() {
    let rendered = summary::render(&RunOutcome::fixture_for_test());

    assert!(rendered.contains("exit_status"));
    assert!(rendered.contains("peak_memory"));
    assert!(rendered.contains("runtime"));
}

#[test]
fn plain_fallback_executes_real_command_and_collects_output_frames() {
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(10));

    let outcome = execute(
        LaunchPlan {
            argv: vec![
                OsString::from("/bin/sh"),
                OsString::from("-lc"),
                OsString::from("printf 'out'; printf 'err' >&2; sleep 0.15"),
            ],
            resource_spec: ResourceSpec {
                interactive: InteractiveMode::Never,
                ..ResourceSpec::default()
            },
            platform: host_platform(),
        },
        &PlainFallbackBackend::default(),
    )
    .unwrap();

    let frames = take_output_frames_for_test();

    assert!(outcome.exit_status.success());
    assert!(!outcome.samples.is_empty());
    assert!(frames.iter().any(|frame| {
        frame.stream == OutputStream::Stdout
            && String::from_utf8_lossy(&frame.bytes).contains("out")
    }));
    assert!(frames.iter().any(|frame| {
        frame.stream == OutputStream::Stderr
            && String::from_utf8_lossy(&frame.bytes).contains("err")
    }));
    assert_eq!(
        record_monitor_fallback_for_test(),
        vec!["monitor_unavailable", "plain_renderer_active"]
    );
    assert_eq!(
        record_summary_timeline_for_test(),
        vec!["launch", "restore_terminal", "render_summary"]
    );
    assert_eq!(
        record_interactive_mode_for_test(),
        vec!["interactive_mode_selected", "pipe_streams"]
    );
    assert_eq!(
        record_post_launch_monitor_failure_for_test(),
        vec![
            "launch_complete",
            "monitor_failed",
            "plain_renderer_continues"
        ]
    );
}

#[test]
fn binary_run_executes_command_and_renders_summary() {
    let mut command = AssertCommand::cargo_bin("scaler").unwrap();

    command.args(["run", "--", "/bin/echo", "hi"]);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("hi").and(predicate::str::contains("exit_status:")));
}

fn host_platform() -> Platform {
    match std::env::consts::OS {
        "linux" => Platform::Linux,
        "macos" => Platform::Macos,
        _ => Platform::Unsupported,
    }
}
