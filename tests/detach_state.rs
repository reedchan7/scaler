use scaler::detach::id::RunId;
use scaler::detach::state::{
    Meta, Result as RunResult, RunState, StateRoot, list_run_ids, read_meta, read_result,
    write_meta, write_result,
};
use tempfile::TempDir;

fn fake_meta(id: &RunId) -> Meta {
    Meta {
        version: 1,
        id: id.as_str().to_string(),
        started: "2026-04-08T14:30:22+08:00".to_string(),
        command: vec!["echo".into(), "hello".into()],
        cwd: "/tmp".into(),
        cpu_limit_centi_cores: Some(80),
        mem_limit_bytes: Some(629_145_600),
        platform: "linux".into(),
        backend: "linux_systemd".into(),
        backend_state: "enforced".into(),
        pid: None,
        unit_name: Some("scaler-run-test.service".into()),
        scaler_exe: "/usr/local/bin/scaler".into(),
        scaler_version: env!("CARGO_PKG_VERSION").into(),
        stdout_log: "/tmp/stdout.log".into(),
        stderr_log: "/tmp/stderr.log".into(),
    }
}

fn fake_result(id: &RunId) -> RunResult {
    RunResult {
        version: 1,
        id: id.as_str().to_string(),
        ended: "2026-04-08T14:40:00+08:00".to_string(),
        state: RunState::Exited,
        exit_code: Some(0),
        signal: None,
        cpu_total_nanos: Some(1_000_000_000),
        memory_peak_bytes: Some(500_000_000),
        launch_error: None,
    }
}

#[test]
fn state_root_honors_xdg_state_home() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    assert_eq!(root.runs_dir(), tmp.path().join("scaler").join("runs"));
}

#[test]
fn meta_round_trip_through_atomic_write() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    let meta = fake_meta(&id);
    write_meta(&root, &id, &meta).unwrap();
    let loaded = read_meta(&root, &id).unwrap();
    assert_eq!(loaded.id, meta.id);
    assert_eq!(loaded.command, meta.command);
    assert_eq!(loaded.cpu_limit_centi_cores, Some(80));
}

#[test]
fn result_round_trip_through_atomic_write() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    write_meta(&root, &id, &fake_meta(&id)).unwrap();
    write_result(&root, &id, &fake_result(&id)).unwrap();
    let loaded = read_result(&root, &id).unwrap();
    assert!(matches!(loaded.state, RunState::Exited));
    assert_eq!(loaded.exit_code, Some(0));
}

#[test]
fn read_result_returns_err_when_absent() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    write_meta(&root, &id, &fake_meta(&id)).unwrap();
    assert!(read_result(&root, &id).is_err());
}

#[test]
fn list_run_ids_returns_sorted_newest_first() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let a = RunId::parse("20260408-090000-aaaa").unwrap();
    let b = RunId::parse("20260408-143022-bbbb").unwrap();
    let c = RunId::parse("20260407-120000-cccc").unwrap();
    write_meta(&root, &a, &fake_meta(&a)).unwrap();
    write_meta(&root, &b, &fake_meta(&b)).unwrap();
    write_meta(&root, &c, &fake_meta(&c)).unwrap();
    let ids = list_run_ids(&root).unwrap();
    assert_eq!(
        ids.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
        vec![
            "20260408-143022-bbbb",
            "20260408-090000-aaaa",
            "20260407-120000-cccc",
        ]
    );
}

#[test]
fn atomic_write_overwrites_existing_file() {
    let tmp = TempDir::new().unwrap();
    let root = StateRoot::with_base(tmp.path().to_path_buf());
    let id = RunId::parse("20260408-143022-a1b2").unwrap();
    let mut meta = fake_meta(&id);
    write_meta(&root, &id, &meta).unwrap();
    meta.command = vec!["new".into(), "command".into()];
    write_meta(&root, &id, &meta).unwrap();
    let reloaded = read_meta(&root, &id).unwrap();
    assert_eq!(
        reloaded.command,
        vec!["new".to_string(), "command".to_string()]
    );
}

#[test]
fn run_dir_has_restrictive_permissions_on_unix() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let root = StateRoot::with_base(tmp.path().to_path_buf());
        let id = RunId::parse("20260408-143022-a1b2").unwrap();
        write_meta(&root, &id, &fake_meta(&id)).unwrap();
        let dir = root.run_dir(&id);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "run dir mode");
    }
}
