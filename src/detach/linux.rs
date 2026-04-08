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

use std::ffi::OsString;
use std::process::Command;

use anyhow::{Context, Result};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::core::LaunchPlan;
use crate::detach::id::RunId;
use crate::detach::state::{Meta, StateRoot, write_meta};

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
