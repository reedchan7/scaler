#![cfg(target_os = "linux")]

use std::ffi::OsString;

use scaler::core::{CpuLimit, LaunchPlan, MemoryLimit, Platform, ResourceSpec};
use scaler::detach::linux::build_detach_argv;

fn plan(cpu_centi: Option<u32>, mem_bytes: Option<u64>) -> LaunchPlan {
    LaunchPlan {
        argv: vec![OsString::from("echo"), OsString::from("hi")],
        resource_spec: ResourceSpec {
            cpu: cpu_centi.map(CpuLimit::from_centi_cores),
            mem: mem_bytes.map(MemoryLimit::from_bytes),
            ..ResourceSpec::default()
        },
        platform: Platform::Linux,
    }
}

fn joined(argv: &[OsString]) -> Vec<String> {
    argv.iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn detach_argv_uses_no_block_and_drops_pipe_wait() {
    let p = plan(Some(80), Some(600 * 1024 * 1024));
    let argv = build_detach_argv(
        &p,
        "scaler-run-20260408-143022-a1b2.service",
        "/var/state/runs/20260408-143022-a1b2/stdout.log",
        "/var/state/runs/20260408-143022-a1b2/stderr.log",
        "/usr/local/bin/scaler",
        "20260408-143022-a1b2",
        "/tmp",
    )
    .expect("argv build ok");
    let args = joined(&argv);
    assert!(
        args.iter().any(|s| s == "--no-block"),
        "must have --no-block"
    );
    assert!(args.iter().all(|s| s != "--pipe"), "must not have --pipe");
    assert!(args.iter().all(|s| s != "--wait"), "must not have --wait");
    assert!(
        args.iter()
            .any(|s| s.starts_with("--property=StandardOutput=append:")),
        "must have StandardOutput=append:"
    );
    assert!(
        args.iter()
            .any(|s| s.starts_with("--property=StandardError=append:")),
        "must have StandardError=append:"
    );
    assert!(
        args.iter().any(|s| s.contains("CPUQuota=")),
        "must have CPUQuota"
    );
    assert!(
        args.iter().any(|s| s.contains("MemoryMax=")),
        "must have MemoryMax"
    );
    assert!(
        args.iter().any(|s| s.contains("ExecStopPost=")),
        "must have ExecStopPost"
    );
    assert!(
        args.iter()
            .any(|s| s.contains("__finalize 20260408-143022-a1b2")),
        "must reference __finalize with run id"
    );
}

#[test]
fn detach_argv_includes_unit_name() {
    let p = plan(None, None);
    let argv = build_detach_argv(
        &p,
        "scaler-run-foo.service",
        "/o",
        "/e",
        "/usr/bin/scaler",
        "foo",
        "",
    )
    .expect("argv build ok");
    let args = joined(&argv);
    assert!(
        args.iter().any(|s| s == "--unit=scaler-run-foo.service"),
        "must have --unit=<name>"
    );
}

#[test]
fn detach_argv_memory_high_is_90_percent() {
    let bytes: u64 = 1_000_000_000;
    let p = plan(None, Some(bytes));
    let argv = build_detach_argv(&p, "u.service", "/o", "/e", "/bin/scaler", "id", "/")
        .expect("argv build ok");
    let args = joined(&argv);
    // MemoryHigh should be round((1_000_000_000 * 9 + 5) / 10) = 900_000_000
    assert!(
        args.iter().any(|s| s == "--property=MemoryHigh=900000000"),
        "MemoryHigh must be 90% of MemoryMax; got: {args:?}"
    );
    assert!(
        args.iter().any(|s| s == "--property=MemorySwapMax=0"),
        "must have MemorySwapMax=0"
    );
}

#[test]
fn detach_argv_cpu_quota_format() {
    let p = plan(Some(150), None); // 1.5 cores = 150 centi-cores
    let argv = build_detach_argv(&p, "u.service", "/o", "/e", "/bin/scaler", "id", "/")
        .expect("argv build ok");
    let args = joined(&argv);
    assert!(
        args.iter().any(|s| s == "--property=CPUQuota=150%"),
        "CPUQuota must use centi-cores as percent; got: {args:?}"
    );
}

#[test]
fn detach_argv_command_appended_after_separator() {
    let p = LaunchPlan {
        argv: vec![
            OsString::from("my-prog"),
            OsString::from("--flag"),
            OsString::from("arg"),
        ],
        resource_spec: ResourceSpec::default(),
        platform: Platform::Linux,
    };
    let argv = build_detach_argv(&p, "u.service", "/o", "/e", "/bin/scaler", "id", "/")
        .expect("argv build ok");
    let args = joined(&argv);
    let sep_pos = args.iter().position(|s| s == "--").expect("-- separator");
    assert_eq!(args[sep_pos + 1], "my-prog");
    assert_eq!(args[sep_pos + 2], "--flag");
    assert_eq!(args[sep_pos + 3], "arg");
}
