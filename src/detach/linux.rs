//! Linux detach path: spawn `systemd-run --no-block` with a transient
//! unit whose `ExecStopPost` calls back into `scaler __finalize <id>`.
//!
//! Unlike the foreground path (`build_systemd_run_argv`), this path:
//! - Uses `--no-block` instead of `--pipe --wait` — systemd-run exits
//!   immediately after registering the transient unit.
//! - Redirects output to append-mode log files via
//!   `StandardOutput=append:` / `StandardError=append:` properties.
//! - Registers `ExecStopPost=scaler __finalize <id>` so the finalizer
//!   (Task 7) writes `result.json` after the child exits.

#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::ffi::OsString;
use std::process::Command;

use anyhow::{Context, Result};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::cli::status::{LiveSnapshot, RunView};
use crate::core::LaunchPlan;
use crate::detach::id::RunId;
use crate::detach::state::{
    Meta, RunResult, RunState, StateRoot, list_run_ids, read_meta, read_result, write_meta,
    write_result,
};

/// Build the argv vector for `systemd-run` in detach mode.
///
/// Exposed as `pub` so `tests/detach_linux_argv.rs` can drive it directly
/// without spinning up a real systemd session. Production code calls this
/// from [`launch`].
pub fn build_detach_argv(
    plan: &LaunchPlan,
    unit_name: &str,
    stdout_log: &str,
    stderr_log: &str,
    scaler_exe: &str,
    run_id: &str,
    cwd: &str,
) -> Result<Vec<OsString>> {
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");
    anyhow::ensure!(!unit_name.is_empty(), "unit name must not be empty");

    let mut argv: Vec<OsString> = Vec::new();
    argv.push("systemd-run".into());
    argv.push("--user".into());
    argv.push("--no-block".into());
    argv.push("--collect".into());
    argv.push("--quiet".into());
    argv.push(format!("--unit={unit_name}").into());

    if let Some(cpu) = plan.resource_spec.cpu {
        argv.push(format!("--property=CPUQuota={}%", cpu.centi_cores()).into());
    }

    if let Some(mem) = plan.resource_spec.mem {
        let bytes = mem.bytes();
        // Match foreground backend: MemoryHigh = round(bytes * 0.9),
        // MemoryMax = hard cap, MemorySwapMax = 0 to prevent swap escape.
        let memory_high = ((u128::from(bytes) * 9) + 5) / 10;
        argv.push(format!("--property=MemoryHigh={memory_high}").into());
        argv.push(format!("--property=MemoryMax={bytes}").into());
        argv.push(OsString::from("--property=MemorySwapMax=0"));
    }

    argv.push(format!("--property=StandardOutput=append:{stdout_log}").into());
    argv.push(format!("--property=StandardError=append:{stderr_log}").into());
    // ExecStopPost fires after the service exits (any exit code, any signal).
    // Task 7 implements `scaler __finalize` to write result.json.
    argv.push(format!("--property=ExecStopPost={scaler_exe} __finalize {run_id}").into());

    if !cwd.is_empty() {
        argv.push(format!("--property=WorkingDirectory={cwd}").into());
    }

    argv.push("--".into());

    // Shell mode: run a single script token through the specified shell.
    // Plain mode: extend argv directly — same as build_systemd_run_argv.
    match plan.resource_spec.shell {
        Some(shell) => {
            anyhow::ensure!(
                plan.argv.len() == 1,
                "shell launch plan requires exactly one script token"
            );
            let shell_prog: OsString = match shell {
                crate::core::ShellKind::Sh => "sh".into(),
                crate::core::ShellKind::Bash => "bash".into(),
                crate::core::ShellKind::Zsh => "zsh".into(),
            };
            argv.push(shell_prog);
            argv.push("-lc".into());
            argv.push(plan.argv[0].clone());
        }
        None => argv.extend(plan.argv.iter().cloned()),
    }

    Ok(argv)
}

/// Launch the command in the background via `systemd-run --no-block`.
///
/// Steps:
/// 1. Generate a fresh [`RunId`] and derive paths.
/// 2. Create the run directory and touch the log files (systemd's
///    `StandardOutput=append:` requires the file to exist beforehand).
/// 3. Write `meta.json` atomically.
/// 4. Build the `systemd-run` argv and spawn it.
/// 5. Return the id to the caller so it can print it to stdout.
pub fn launch(plan: &LaunchPlan, root: &StateRoot) -> Result<RunId> {
    let id = RunId::generate();
    let unit_name = format!("scaler-run-{}.service", id.as_str());

    let scaler_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "scaler".to_string());

    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let stdout_log_path = root.stdout_log_path(&id);
    let stderr_log_path = root.stderr_log_path(&id);

    // The run dir must exist before we touch the log files and before
    // write_meta runs. Idempotent: create_dir_all is a no-op if present.
    let run_dir = root.run_dir(&id);
    std::fs::create_dir_all(&run_dir)
        .with_context(|| format!("create run dir {}", run_dir.display()))?;
    std::fs::File::create(&stdout_log_path)
        .with_context(|| format!("touch {}", stdout_log_path.display()))?;
    std::fs::File::create(&stderr_log_path)
        .with_context(|| format!("touch {}", stderr_log_path.display()))?;

    let report = crate::backend::detect_host_capabilities();

    let meta = build_meta(
        &id,
        plan,
        &scaler_exe,
        Some(unit_name.clone()),
        stdout_log_path.to_string_lossy().into_owned(),
        stderr_log_path.to_string_lossy().into_owned(),
        report.backend_state.as_str(),
        &cwd,
    );
    write_meta(root, &id, &meta)?;

    let argv = build_detach_argv(
        plan,
        &unit_name,
        &stdout_log_path.to_string_lossy(),
        &stderr_log_path.to_string_lossy(),
        &scaler_exe,
        id.as_str(),
        &cwd,
    )?;

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    let output = cmd
        .output()
        .with_context(|| format!("spawn systemd-run for run {}", id.as_str()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        anyhow::bail!(
            "systemd-run failed (exit {:?}): {}",
            output.status.code(),
            stderr.trim()
        );
    }

    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn build_meta(
    id: &RunId,
    plan: &LaunchPlan,
    scaler_exe: &str,
    unit_name: Option<String>,
    stdout_log: String,
    stderr_log: String,
    backend_state: &str,
    cwd: &str,
) -> Meta {
    let started = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_offset(UtcOffset::UTC))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into());

    Meta {
        version: 1,
        id: id.as_str().to_string(),
        started,
        command: plan
            .argv
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect(),
        cwd: cwd.to_string(),
        cpu_limit_centi_cores: plan.resource_spec.cpu.map(|c| c.centi_cores()),
        mem_limit_bytes: plan.resource_spec.mem.map(|m| m.bytes()),
        platform: "linux".into(),
        backend: "linux_systemd".into(),
        backend_state: backend_state.into(),
        pid: None,
        unit_name,
        scaler_exe: scaler_exe.into(),
        scaler_version: env!("CARGO_PKG_VERSION").into(),
        stdout_log,
        stderr_log,
    }
}

/// Entry point invoked by `scaler __finalize <id>` (the hidden subcommand
/// that systemd runs via `ExecStopPost`). Reads env vars systemd injects
/// (`SERVICE_RESULT`, `EXIT_CODE`, `EXIT_STATUS`), best-effort queries
/// `systemctl --user show` for cumulative CPU/mem, and writes
/// `result.json`. Always returns Ok — we never want a non-zero exit from
/// the finalize hook to fail systemd's stop transition.
pub fn finalize(run_id: &str) -> Result<()> {
    let root = StateRoot::from_env()?;
    let env: HashMap<String, String> = std::env::vars().collect();
    let show = run_systemctl_show_metrics(run_id).ok();
    finalize_with_env(&root, run_id, &env, show.as_deref())
}

/// Pure core of `finalize`, exposed for unit/integration tests. Takes
/// a state root, a run id string, an env map, and optional pre-fetched
/// `systemctl show` output, and writes `result.json` to disk.
pub fn finalize_with_env(
    root: &StateRoot,
    run_id: &str,
    env: &HashMap<String, String>,
    show_output: Option<&str>,
) -> Result<()> {
    let id = RunId::parse(run_id).ok_or_else(|| anyhow::anyhow!("invalid run id: {run_id}"))?;

    let exit_code_label = env.get("EXIT_CODE").map(String::as_str).unwrap_or("");
    let exit_status_raw = env.get("EXIT_STATUS").map(String::as_str).unwrap_or("");

    // systemd ExecStopPost env semantics (per `man systemd.service`):
    //   EXIT_CODE=exited  => EXIT_STATUS is the numeric exit code (e.g. "0", "42")
    //   EXIT_CODE=killed  => EXIT_STATUS is the signal NAME without SIG prefix (e.g. "TERM")
    //   EXIT_CODE=dumped  => EXIT_STATUS is the signal NAME without SIG prefix
    let (state, exit_code, signal) = match exit_code_label {
        "exited" => {
            let code = exit_status_raw.parse::<i32>().ok();
            (RunState::Exited, code, None)
        }
        "killed" | "dumped" => {
            // exit_status_raw is a signal name like "TERM" — prefix with "SIG"
            // and look up the number for the 128+sig exit code convention.
            let sig_name = if exit_status_raw.is_empty() {
                "signal".to_string()
            } else {
                format!("SIG{exit_status_raw}")
            };
            let exit_code = signal_number(&sig_name).map(|n| 128 + n);
            (RunState::Killed, exit_code, Some(sig_name))
        }
        _ => (RunState::LaunchFailed, None, None),
    };

    let (cpu_total_nanos, memory_peak_bytes) = parse_show_metrics(show_output);

    let ended = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_offset(UtcOffset::UTC))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into());

    let result = RunResult {
        version: 1,
        id: id.as_str().to_string(),
        ended,
        state,
        exit_code,
        signal,
        cpu_total_nanos,
        memory_peak_bytes,
        launch_error: None,
    };
    write_result(root, &id, &result)
}

fn run_systemctl_show_metrics(run_id: &str) -> Result<String> {
    let unit = format!("scaler-run-{run_id}.service");
    let out = Command::new("systemctl")
        .args([
            "--user",
            "show",
            &unit,
            "--property=CPUUsageNSec,MemoryPeak,ExecMainStartTimestamp,ExecMainExitTimestamp",
        ])
        .output()
        .context("spawn systemctl show")?;
    if !out.status.success() {
        anyhow::bail!("systemctl show failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse_show_metrics(show: Option<&str>) -> (Option<u128>, Option<u64>) {
    let Some(text) = show else {
        return (None, None);
    };
    let mut cpu: Option<u128> = None;
    let mut mem: Option<u64> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("CPUUsageNSec=") {
            cpu = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("MemoryPeak=") {
            mem = rest.trim().parse().ok();
        }
    }
    (cpu, mem)
}

fn signal_number(sig_name: &str) -> Option<i32> {
    Some(match sig_name {
        "SIGHUP" => 1,
        "SIGINT" => 2,
        "SIGQUIT" => 3,
        "SIGABRT" => 6,
        "SIGKILL" => 9,
        "SIGSEGV" => 11,
        "SIGPIPE" => 13,
        "SIGTERM" => 15,
        _ => return None,
    })
}

/// Return a unified `RunView` for one run. Prefers `result.json` as the
/// source of truth (terminal state); falls back to `systemctl --user show`
/// for a live snapshot; if neither is available, marks the run as `gone`.
pub fn query_one(root: &StateRoot, id: &RunId) -> Result<RunView> {
    let meta = read_meta(root, id)?;
    if let Ok(result) = read_result(root, id) {
        return Ok(RunView {
            meta,
            result: Some(result),
            live: None,
            gone: false,
        });
    }
    if let Ok(show) = run_live_show(id)
        && let Some(live) = parse_live_show(&show)
    {
        return Ok(RunView {
            meta,
            result: None,
            live: Some(live),
            gone: false,
        });
    }
    Ok(RunView {
        meta,
        result: None,
        live: None,
        gone: true,
    })
}

/// Return all runs in the state dir, newest first. Best-effort: runs whose
/// meta.json fails to load are skipped with a stderr warning.
pub fn query_all(root: &StateRoot) -> Result<Vec<RunView>> {
    let ids = list_run_ids(root)?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match query_one(root, &id) {
            Ok(v) => out.push(v),
            Err(e) => eprintln!("scaler status: skipping {}: {e:#}", id.as_str()),
        }
    }
    Ok(out)
}

fn run_live_show(id: &RunId) -> Result<String> {
    let unit = format!("scaler-run-{}.service", id.as_str());
    let out = Command::new("systemctl")
        .args([
            "--user",
            "show",
            &unit,
            "--property=ActiveState,SubState,MainPID,CPUUsageNSec,MemoryCurrent,ActiveEnterTimestamp",
        ])
        .output()
        .context("spawn systemctl show")?;
    if !out.status.success() {
        anyhow::bail!("systemctl show failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse the key=value output of `systemctl --user show` into a
/// `LiveSnapshot`. Returns `None` when the unit is not currently active
/// (caller should then report `gone`).
pub fn parse_live_show(text: &str) -> Option<LiveSnapshot> {
    let mut cpu: Option<u128> = None;
    let mut mem: Option<u64> = None;
    let mut active: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("CPUUsageNSec=") {
            cpu = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("MemoryCurrent=") {
            mem = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("ActiveState=") {
            active = Some(rest.trim().to_string());
        }
    }
    if active.as_deref() != Some("active") {
        return None;
    }
    // elapsed_secs is deferred to a future task — computing it needs
    // parsing systemd's free-form ActiveEnterTimestamp ("Wed 2026-04-08
    // 14:30:22 UTC"). For v1 the detail renderer tolerates None.
    Some(LiveSnapshot {
        cpu_total_nanos: cpu,
        memory_current_bytes: mem,
        elapsed_secs: None,
    })
}
