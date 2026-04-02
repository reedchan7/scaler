use scaler::core::{
    OutputStream, RunOutcome,
    output::{OutputCollector, next_sequence},
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
