#![cfg(target_os = "linux")]

use scaler::cli::status::LiveSnapshot;
use scaler::detach::id::RunId;
use scaler::detach::linux::{parse_live_show, query_one};
use scaler::detach::state::{Meta, RunResult, RunState, StateRoot, write_meta, write_result};
use tempfile::TempDir;

fn fixture_meta(id: &RunId) -> Meta {
    Meta {
        version: 1,
        id: id.as_str().into(),
        started: "2026-04-08T14:30:22+00:00".into(),
        command: vec!["sleep".into(), "60".into()],
        cwd: "/tmp".into(),
        cpu_limit_centi_cores: None,
        mem_limit_bytes: None,
        platform: "linux".into(),
        backend: "linux_systemd".into(),
        backend_state: "enforced".into(),
        pid: None,
        unit_name: Some("scaler-run-test.service".into()),
        scaler_exe: "/bin/scaler".into(),
        scaler_version: "1.0.1".into(),
        stdout_log: "/tmp/o".into(),
        stderr_log: "/tmp/e".into(),
    }
}

#[test]
fn query_one_uses_result_json_when_present() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    write_meta(&root, &id, &fixture_meta(&id)).unwrap();
    write_result(
        &root,
        &id,
        &RunResult {
            version: 1,
            id: id.as_str().into(),
            ended: "2026-04-08T14:40:00+00:00".into(),
            state: RunState::Exited,
            exit_code: Some(0),
            signal: None,
            cpu_total_nanos: Some(1_000_000_000),
            memory_peak_bytes: Some(500_000_000),
            launch_error: None,
        },
    )
    .unwrap();

    let view = query_one(&root, &id).unwrap();
    assert!(view.result.is_some());
    assert!(view.live.is_none());
    assert!(!view.gone);
}

#[test]
fn parse_live_show_extracts_metrics_from_active_unit() {
    let show = "\
ActiveState=active
SubState=running
MainPID=12345
CPUUsageNSec=12000000000
MemoryCurrent=209715200
ActiveEnterTimestamp=Wed 2026-04-08 14:30:22 UTC
";
    let live: LiveSnapshot = parse_live_show(show).expect("active unit parses");
    assert_eq!(live.cpu_total_nanos, Some(12_000_000_000));
    assert_eq!(live.memory_current_bytes, Some(209_715_200));
}

#[test]
fn parse_live_show_returns_none_for_inactive_unit() {
    let show = "\
ActiveState=inactive
SubState=dead
CPUUsageNSec=5000000000
MemoryCurrent=0
";
    assert!(parse_live_show(show).is_none());
}
