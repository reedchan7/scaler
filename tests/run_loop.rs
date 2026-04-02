use std::{ffi::OsString, io::Read, process::Stdio, sync::mpsc, time::Duration};

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use scaler::core::{
    InteractiveMode, LaunchPlan, OutputStream, Platform, ResourceSpec, RunOutcome,
    output::{OutputCollector, next_sequence},
    run_loop::{
        PlainFallbackBackend, execute, plain_fallback_command_preview_for_test,
        record_interactive_mode_for_test,
        record_monitor_fallback_for_test, record_post_launch_monitor_failure_for_test,
        record_summary_timeline_for_test, reset_test_state, set_test_poll_interval_for_next_run,
        take_output_frames_for_test,
    },
    summary,
};

#[test]
fn pipe_frames_keep_per_stream_order() {
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
        vec!["launch", "restore_terminal"]
    );
    assert_eq!(
        record_interactive_mode_for_test(),
        vec!["interactive_mode_selected", "pipe_streams"]
    );
    assert!(record_post_launch_monitor_failure_for_test().is_empty());
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

#[test]
fn binary_run_propagates_child_exit_code() {
    let mut command = AssertCommand::cargo_bin("scaler").unwrap();

    command.args(["run", "--", "/bin/sh", "-lc", "exit 7"]);
    command
        .assert()
        .code(7)
        .stdout(predicate::str::contains("exit_status:").and(predicate::str::contains("7")));
}

#[test]
fn binary_run_forwards_stdout_before_first_sample_tick() {
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

fn host_platform() -> Platform {
    match std::env::consts::OS {
        "linux" => Platform::Linux,
        "macos" => Platform::Macos,
        _ => Platform::Unsupported,
    }
}
