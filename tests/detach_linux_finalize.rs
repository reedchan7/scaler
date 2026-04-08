#![cfg(target_os = "linux")]

use std::collections::HashMap;

use scaler::detach::id::RunId;
use scaler::detach::linux::finalize_with_env;
use scaler::detach::state::{Meta, RunState, StateRoot, read_result, write_meta};
use tempfile::TempDir;

fn fixture_meta(id: &RunId) -> Meta {
    Meta {
        version: 1,
        id: id.as_str().to_string(),
        started: "2026-04-08T14:30:22+00:00".into(),
        command: vec!["echo".into(), "hi".into()],
        cwd: "/tmp".into(),
        cpu_limit_centi_cores: Some(80),
        mem_limit_bytes: Some(600 * 1024 * 1024),
        platform: "linux".into(),
        backend: "linux_systemd".into(),
        backend_state: "enforced".into(),
        pid: None,
        unit_name: Some("scaler-run-test.service".into()),
        scaler_exe: "/usr/local/bin/scaler".into(),
        scaler_version: "1.0.1".into(),
        stdout_log: "/tmp/o".into(),
        stderr_log: "/tmp/e".into(),
    }
}

#[test]
fn finalize_writes_exited_result_from_env() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    write_meta(&root, &id, &fixture_meta(&id)).unwrap();

    let mut env = HashMap::new();
    env.insert("SERVICE_RESULT".to_string(), "success".to_string());
    env.insert("EXIT_CODE".to_string(), "exited".to_string());
    env.insert("EXIT_STATUS".to_string(), "0".to_string());

    finalize_with_env(&root, id.as_str(), &env, None).unwrap();

    let result = read_result(&root, &id).unwrap();
    assert!(matches!(result.state, RunState::Exited));
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.signal, None);
    assert!(!result.ended.is_empty());
}

#[test]
fn finalize_writes_killed_result_from_env() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    write_meta(&root, &id, &fixture_meta(&id)).unwrap();

    let mut env = HashMap::new();
    env.insert("SERVICE_RESULT".to_string(), "signal".to_string());
    env.insert("EXIT_CODE".to_string(), "killed".to_string());
    env.insert("EXIT_STATUS".to_string(), "9".to_string());

    finalize_with_env(&root, id.as_str(), &env, None).unwrap();

    let result = read_result(&root, &id).unwrap();
    assert!(matches!(result.state, RunState::Killed));
    assert_eq!(result.signal.as_deref(), Some("SIGKILL"));
    assert_eq!(result.exit_code, Some(128 + 9));
}

#[test]
fn finalize_includes_cpu_and_mem_metrics_from_show_output() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    write_meta(&root, &id, &fixture_meta(&id)).unwrap();

    let mut env = HashMap::new();
    env.insert("SERVICE_RESULT".to_string(), "success".to_string());
    env.insert("EXIT_CODE".to_string(), "exited".to_string());
    env.insert("EXIT_STATUS".to_string(), "0".to_string());

    let show = "\
CPUUsageNSec=2892000000000
MemoryPeak=615514112
ExecMainStartTimestamp=Wed 2026-04-08 14:30:22 UTC
ExecMainExitTimestamp=Wed 2026-04-08 14:45:00 UTC
";
    finalize_with_env(&root, id.as_str(), &env, Some(show)).unwrap();

    let result = read_result(&root, &id).unwrap();
    assert_eq!(result.cpu_total_nanos, Some(2_892_000_000_000));
    assert_eq!(result.memory_peak_bytes, Some(615_514_112));
}
