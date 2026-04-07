use std::{
    ffi::OsString,
    io::Read,
    process::Stdio,
    sync::{Mutex, MutexGuard, OnceLock, mpsc},
    time::Duration,
};

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use scaler::core::{
    InteractiveMode, LaunchPlan, OutputStream, Platform, ResourceSpec, RunOutcome,
    output::{OutputCollector, next_sequence},
    run_loop::{
        PlainFallbackBackend, execute, plain_fallback_command_preview_for_test,
        record_interactive_mode_for_test, record_monitor_fallback_for_test,
        record_post_launch_monitor_failure_for_test, record_summary_timeline_for_test,
        record_ui_mode_for_test, reset_test_state, set_test_monitor_fail_after_launch_for_next_run,
        set_test_monitor_start_failure_for_next_run, set_test_poll_interval_for_next_run,
        set_test_terminal_state_for_next_run, take_output_frames_for_test,
    },
    summary,
};

#[test]
fn pipe_frames_keep_per_stream_order() {
    let _guard = test_guard();
    let mut sequence = 0;

    assert_eq!(next_sequence(&mut sequence), 0);
    assert_eq!(next_sequence(&mut sequence), 1);
    assert_eq!(next_sequence(&mut sequence), 2);

    let mut collector = OutputCollector::default();
    let stdout_first = collector.push_stdout(b"first");
    let stdout_second = collector.push_stdout(b"second");

    assert_eq!(stdout_first.sequence + 1, stdout_second.sequence);
    assert_eq!(stdout_first.sequence, 0);
    assert_eq!(stdout_second.sequence, 1);
    assert_eq!(stdout_first.stream, OutputStream::Stdout);
    assert_eq!(stdout_second.stream, OutputStream::Stdout);
    assert_eq!(stdout_first.bytes, b"first");
    assert_eq!(stdout_second.bytes, b"second");
}

#[test]
fn stderr_and_pty_frames_are_tagged_correctly() {
    let _guard = test_guard();
    let mut collector = OutputCollector::default();

    let stderr = collector.push_stderr(b"problem");
    let pty = collector.push_pty(b"prompt");

    assert_eq!(stderr.stream, OutputStream::Stderr);
    assert_eq!(pty.stream, OutputStream::PtyMerged);
    assert!(stderr.sequence < pty.sequence);
}

#[test]
fn summary_renderer_includes_status_and_peak_memory() {
    let _guard = test_guard();
    let rendered = summary::render(&RunOutcome::fixture_for_test());

    assert!(rendered.contains("exit     0"));
    assert!(rendered.contains("elapsed  3.000s"));
    assert!(rendered.contains("memory   max 4.0 MiB"));
    assert!(rendered.contains("cpu      avg 0.19c (18.8%), max 0.25c (25.0%)"));
}

#[test]
fn plain_fallback_executes_real_command_and_collects_output_frames() {
    let _guard = test_guard();
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
        &PlainFallbackBackend,
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
    assert!(record_post_launch_monitor_failure_for_test().is_empty());
}

#[test]
fn binary_run_executes_command_and_renders_summary() {
    let _guard = test_guard();
    let mut command = AssertCommand::cargo_bin("scaler").unwrap();

    command.args(["run", "--", "/bin/echo", "hi"]);
    command
        .assert()
        .success()
        // Child output stays on stdout so user pipelines can grep/jq it.
        .stdout(predicate::str::contains("hi"))
        // Summary card (header + body + footer) goes to stderr so it
        // never contaminates a piped stdout.
        .stderr(
            predicate::str::contains(" scaler summary ")
                .and(predicate::str::contains("  exit     0"))
                .and(predicate::str::contains("└──")),
        );
}

#[test]
fn binary_run_propagates_child_exit_code() {
    let _guard = test_guard();
    let mut command = AssertCommand::cargo_bin("scaler").unwrap();

    command.args(["run", "--", "/bin/sh", "-lc", "exit 7"]);
    command
        .assert()
        .code(7)
        .stderr(predicate::str::contains("exit     7"));
}

#[test]
fn binary_run_forwards_stdout_before_first_sample_tick() {
    let _guard = test_guard();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("scaler"))
        .args(["run", "--", "/bin/sh", "-lc", "printf ready; sleep 1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 5];
        stdout.read_exact(&mut buffer).unwrap();
        let _ = tx.send(buffer);
        let mut rest = Vec::new();
        let _ = stdout.read_to_end(&mut rest);
    });

    let first_bytes = rx.recv_timeout(Duration::from_millis(490)).unwrap();
    assert_eq!(&first_bytes, b"ready");
    assert!(child.wait().unwrap().success());
}

#[test]
fn linux_pty_fallback_command_preview_uses_util_linux_script_shape() {
    let _guard = test_guard();
    let preview = plain_fallback_command_preview_for_test(&LaunchPlan {
        argv: vec![OsString::from("/bin/echo"), OsString::from("hi there")],
        resource_spec: ResourceSpec {
            interactive: InteractiveMode::Always,
            ..ResourceSpec::default()
        },
        platform: Platform::Linux,
    })
    .unwrap();

    let preview = preview
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(preview[0], "script");
    assert_eq!(preview[1], "-q");
    assert_eq!(preview[2], "-e");
    assert_eq!(preview[3], "-c");
    assert_eq!(preview[5], "/dev/null");
    assert!(preview[4].contains("/bin/echo"));
    assert!(preview[4].contains("'hi there'"));
}

#[test]
fn final_summary_is_emitted_once_after_terminal_restore() {
    let _guard = test_guard();
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(10));
    set_test_terminal_state_for_next_run(true, true, true);

    let outcome = execute(
        LaunchPlan {
            argv: vec![
                OsString::from("/bin/sh"),
                OsString::from("-lc"),
                OsString::from("printf summary-check; sleep 0.05"),
            ],
            resource_spec: ResourceSpec {
                interactive: InteractiveMode::Never,
                monitor: true,
                ..ResourceSpec::default()
            },
            platform: host_platform(),
        },
        &PlainFallbackBackend,
    )
    .unwrap();

    assert!(outcome.exit_status.success());
    assert_eq!(
        record_summary_timeline_for_test(),
        vec!["launch", "restore_terminal", "render_summary"]
    );
    assert_eq!(record_ui_mode_for_test(), vec!["tui_renderer_active"]);
}

#[test]
fn monitor_start_failure_falls_back_to_plain_streaming() {
    let _guard = test_guard();
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(10));
    set_test_terminal_state_for_next_run(true, true, true);
    set_test_monitor_start_failure_for_next_run("simulated monitor init failure");

    let outcome = execute(
        LaunchPlan {
            argv: vec![
                OsString::from("/bin/sh"),
                OsString::from("-lc"),
                OsString::from("printf 'plain-out'; printf 'plain-err' >&2; sleep 0.05"),
            ],
            resource_spec: ResourceSpec {
                interactive: InteractiveMode::Never,
                monitor: true,
                ..ResourceSpec::default()
            },
            platform: host_platform(),
        },
        &PlainFallbackBackend,
    )
    .unwrap();

    let frames = take_output_frames_for_test();

    assert!(outcome.exit_status.success());
    assert!(frames.iter().any(|frame| {
        frame.stream == OutputStream::Stdout
            && String::from_utf8_lossy(&frame.bytes).contains("plain-out")
    }));
    assert!(frames.iter().any(|frame| {
        frame.stream == OutputStream::Stderr
            && String::from_utf8_lossy(&frame.bytes).contains("plain-err")
    }));
    assert_eq!(
        record_monitor_fallback_for_test(),
        vec!["monitor_unavailable", "plain_renderer_active"]
    );
    assert_eq!(record_ui_mode_for_test(), vec!["plain_renderer_active"]);
}

#[test]
fn interactive_pty_uses_compact_mode() {
    let _guard = test_guard();
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(10));
    set_test_terminal_state_for_next_run(true, true, true);

    let outcome = execute(
        LaunchPlan {
            argv: vec![
                OsString::from("/bin/sh"),
                OsString::from("-lc"),
                OsString::from("printf compact-pty; sleep 0.05"),
            ],
            resource_spec: ResourceSpec {
                interactive: InteractiveMode::Always,
                monitor: true,
                ..ResourceSpec::default()
            },
            platform: host_platform(),
        },
        &PlainFallbackBackend,
    )
    .unwrap();

    let frames = take_output_frames_for_test();

    assert!(outcome.exit_status.success());
    assert!(frames.iter().any(|frame| {
        frame.stream == OutputStream::PtyMerged
            && String::from_utf8_lossy(&frame.bytes).contains("compact-pty")
    }));
    assert_eq!(
        record_interactive_mode_for_test(),
        vec!["interactive_mode_selected", "pty_merged_stream"]
    );
    assert_eq!(
        record_ui_mode_for_test(),
        vec!["tui_renderer_active", "compact_interactive_mode"]
    );
}

#[test]
fn monitor_failure_after_launch_restores_terminal_and_keeps_output_flowing() {
    let _guard = test_guard();
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(10));
    set_test_terminal_state_for_next_run(true, true, true);
    set_test_monitor_fail_after_launch_for_next_run(1);

    let outcome = execute(
        LaunchPlan {
            argv: vec![
                OsString::from("/bin/sh"),
                OsString::from("-lc"),
                OsString::from("printf before; sleep 0.15; printf after"),
            ],
            resource_spec: ResourceSpec {
                interactive: InteractiveMode::Never,
                monitor: true,
                ..ResourceSpec::default()
            },
            platform: host_platform(),
        },
        &PlainFallbackBackend,
    )
    .unwrap();

    let frames = take_output_frames_for_test();

    assert!(outcome.exit_status.success());
    assert!(
        frames
            .iter()
            .any(|frame| { String::from_utf8_lossy(&frame.bytes).contains("before") })
    );
    assert!(
        frames
            .iter()
            .any(|frame| { String::from_utf8_lossy(&frame.bytes).contains("after") })
    );
    assert_eq!(
        record_post_launch_monitor_failure_for_test(),
        vec!["monitor_failed", "plain_renderer_continues"]
    );
    assert_eq!(
        record_summary_timeline_for_test(),
        vec!["launch", "restore_terminal", "render_summary"]
    );
}

fn host_platform() -> Platform {
    match std::env::consts::OS {
        "linux" => Platform::Linux,
        "macos" => Platform::Macos,
        _ => Platform::Unsupported,
    }
}

fn test_guard() -> MutexGuard<'static, ()> {
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
