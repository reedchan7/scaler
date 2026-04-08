use scaler::cli::status::{LiveSnapshot, RunView, render_detail, render_list};
use scaler::detach::state::{Meta, RunResult, RunState};

fn meta(id: &str) -> Meta {
    Meta {
        version: 1,
        id: id.to_string(),
        started: "2026-04-08T14:30:22+00:00".into(),
        command: vec!["npm".into(), "install".into()],
        cwd: "/tmp".into(),
        cpu_limit_centi_cores: Some(80),
        mem_limit_bytes: Some(600 * 1024 * 1024),
        platform: "linux".into(),
        backend: "linux_systemd".into(),
        backend_state: "enforced".into(),
        pid: None,
        unit_name: Some(format!("scaler-run-{id}.service")),
        scaler_exe: "/usr/local/bin/scaler".into(),
        scaler_version: "1.0.1".into(),
        stdout_log: format!("/tmp/runs/{id}/stdout.log"),
        stderr_log: format!("/tmp/runs/{id}/stderr.log"),
    }
}

fn exited_result(id: &str, code: i32) -> RunResult {
    RunResult {
        version: 1,
        id: id.into(),
        ended: "2026-04-08T14:45:00+00:00".into(),
        state: RunState::Exited,
        exit_code: Some(code),
        signal: None,
        cpu_total_nanos: Some(600_000_000_000),
        memory_peak_bytes: Some(500 * 1024 * 1024),
        launch_error: None,
    }
}

#[test]
fn list_shows_exited_success() {
    let view = RunView {
        meta: meta("20260408-143022-a1b2"),
        result: Some(exited_result("20260408-143022-a1b2", 0)),
        live: None,
        gone: false,
    };
    let mut out = Vec::new();
    render_list(&mut out, &[view], false).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("20260408-143022-a1b2"));
    assert!(s.contains("exited(0)"));
    assert!(s.contains("npm install"));
}

#[test]
fn list_shows_running_with_live_duration() {
    let view = RunView {
        meta: meta("20260408-143022-a1b2"),
        result: None,
        live: Some(LiveSnapshot {
            cpu_total_nanos: Some(12_000_000_000),
            memory_current_bytes: Some(200 * 1024 * 1024),
            elapsed_secs: Some(125),
        }),
        gone: false,
    };
    let mut out = Vec::new();
    render_list(&mut out, &[view], false).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("running"));
    assert!(s.contains("2m05s"));
}

#[test]
fn list_marks_gone_runs() {
    let view = RunView {
        meta: meta("20260408-143022-a1b2"),
        result: None,
        live: None,
        gone: true,
    };
    let mut out = Vec::new();
    render_list(&mut out, &[view], false).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("gone"));
}

#[test]
fn detail_contains_all_expected_fields_for_exited() {
    let view = RunView {
        meta: meta("20260408-143022-a1b2"),
        result: Some(exited_result("20260408-143022-a1b2", 0)),
        live: None,
        gone: false,
    };
    let mut out = Vec::new();
    render_detail(&mut out, &view, false).unwrap();
    let s = String::from_utf8(out).unwrap();
    for needle in [
        "id:",
        "command:  npm install",
        "limits:   cpu=0.80c  mem=600 MiB",
        "backend:  linux_systemd (enforced)",
        "started:  2026-04-08T14:30:22+00:00",
        "ended:    2026-04-08T14:45:00+00:00",
        "state:    exited(0)",
        "cpu:      total",
        "memory:   peak 500 MiB",
        "stdout:   /tmp/runs/20260408-143022-a1b2/stdout.log",
    ] {
        assert!(s.contains(needle), "missing {needle:?} in:\n{s}");
    }
}

#[test]
fn detail_json_is_valid_and_round_trips_key_fields() {
    let view = RunView {
        meta: meta("20260408-143022-a1b2"),
        result: Some(exited_result("20260408-143022-a1b2", 0)),
        live: None,
        gone: false,
    };
    let mut out = Vec::new();
    render_detail(&mut out, &view, true).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["meta"]["id"], "20260408-143022-a1b2");
    assert_eq!(parsed["result"]["state"], "exited");
    assert_eq!(parsed["result"]["exit_code"], 0);
}
