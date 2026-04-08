//! macOS detach path: double-fork + `setsid` + stdio redirection + headless
//! `core::run_loop::execute_headless`, then write `result.json` on exit.

#![cfg(target_os = "macos")]

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use anyhow::{Context, Result};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

use crate::backend;
use crate::cli::status::{LiveSnapshot, RunView};
use crate::core::run_loop::execute_headless;
use crate::core::{CapabilityLevel, LaunchPlan};
use crate::detach::id::RunId;
use crate::detach::state::{
    Meta, RunResult, RunState, StateRoot, list_run_ids, read_meta, read_result, write_meta,
    write_result,
};

/// Launch the command in the background via a double-fork daemonize pattern.
///
/// Steps:
/// 1. Generate a fresh [`RunId`] and create the run directory + log files.
/// 2. Write `meta.json` (without pid — we don't know the grandchild's pid yet).
/// 3. Fork once, then `setsid()` in the first child to detach from the
///    controlling terminal, then fork again so the grandchild is no longer
///    a session leader (and can never re-acquire a tty).
/// 4. In the grandchild: redirect stdin/stdout/stderr, rewrite `meta.json`
///    with the real pid, run `execute_headless`, and write `result.json`.
/// 5. In the original process: return the id.
pub fn launch(plan: &LaunchPlan, root: &StateRoot) -> Result<RunId> {
    let id = RunId::generate();
    std::fs::create_dir_all(root.run_dir(&id))
        .with_context(|| format!("create run dir {}", root.run_dir(&id).display()))?;

    let stdout_log = root.stdout_log_path(&id);
    let stderr_log = root.stderr_log_path(&id);
    std::fs::File::create(&stdout_log)
        .with_context(|| format!("touch {}", stdout_log.display()))?;
    std::fs::File::create(&stderr_log)
        .with_context(|| format!("touch {}", stderr_log.display()))?;

    let scaler_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "scaler".into());

    let meta = build_meta(
        &id,
        plan,
        &scaler_exe,
        stdout_log.to_string_lossy().into_owned(),
        stderr_log.to_string_lossy().into_owned(),
    );
    write_meta(root, &id, &meta)?;

    // First fork.
    // SAFETY: At this point the process is single-threaded — no reader
    // threads have been spawned (those only start inside `execute_headless`
    // in the grandchild). Forking a multi-threaded process is unsafe; we
    // verify the single-threaded assumption holds by design: the detach
    // launch path is called from `scaler run --detach`, which never enters
    // the foreground run loop (and thus never spawns reader threads).
    let first = unsafe { libc::fork() };
    if first < 0 {
        anyhow::bail!("fork failed: {}", std::io::Error::last_os_error());
    }
    if first > 0 {
        // Original process: return the id so the caller can print it.
        return Ok(id);
    }

    // First child: detach from the controlling terminal by becoming a
    // session leader. SAFETY: see first fork comment above.
    if unsafe { libc::setsid() } < 0 {
        std::process::exit(127);
    }

    // Second fork: grandchild is not a session leader and therefore cannot
    // accidentally re-acquire a controlling tty. SAFETY: still
    // single-threaded — no threads have been started yet.
    let second = unsafe { libc::fork() };
    if second < 0 {
        std::process::exit(127);
    }
    if second > 0 {
        // First child exits; grandchild continues.
        std::process::exit(0);
    }

    // Grandchild. Redirect stdio so the daemon's output goes to the logs.
    let _ = redirect_stdio(&stdout_log, &stderr_log);

    // Rewrite meta with the grandchild's pid. Best-effort: if this fails
    // `query_one` will just not find a live process and report `gone`.
    let mut meta_with_pid = meta.clone();
    meta_with_pid.pid = Some(std::process::id());
    let _ = write_meta(root, &id, &meta_with_pid);

    // Run the command headless. `execute_headless` spawns reader threads
    // internally, which is fine — we are now fully daemonized.
    let backend_box = backend::select_backend();
    let outcome_result = execute_headless(plan.clone(), backend_box.as_ref());

    // Write result.json regardless of success/failure.
    let result_payload = to_result_json(
        &id,
        outcome_result.as_ref().ok(),
        outcome_result.as_ref().err(),
    );
    let _ = write_result(root, &id, &result_payload);

    let code = outcome_result
        .ok()
        .and_then(|o| o.exit_status.code())
        .unwrap_or(0);
    std::process::exit(code);
}

/// Return a unified [`RunView`] for one run. Prefers `result.json` as the
/// source of truth (terminal state). For live runs queries `ps` for a
/// snapshot. Falls back to `gone` when neither is available.
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
    // Check if the pid is still alive with signal 0 (no-op).
    // SAFETY: kill(pid, 0) only tests reachability — it never delivers a
    // signal and has no side effects on the target process.
    if let Some(pid) = meta.pid
        && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
    {
        let live = snapshot_from_ps(pid as i32).ok();
        return Ok(RunView {
            meta,
            result: None,
            live,
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
/// `meta.json` fails to load are skipped with a stderr warning.
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

fn build_meta(
    id: &RunId,
    plan: &LaunchPlan,
    scaler_exe: &str,
    stdout_log: String,
    stderr_log: String,
) -> Meta {
    let started = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_offset(UtcOffset::UTC))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into());

    let report = backend::detect_host_capabilities();
    let backend_state = match report.backend_state {
        CapabilityLevel::Enforced => "enforced",
        CapabilityLevel::BestEffort => "best_effort",
        CapabilityLevel::Unavailable => "unavailable",
    };

    Meta {
        version: 1,
        id: id.as_str().to_string(),
        started,
        command: plan
            .argv
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        cpu_limit_centi_cores: plan.resource_spec.cpu.map(|c| c.centi_cores()),
        mem_limit_bytes: plan.resource_spec.mem.map(|m| m.bytes()),
        platform: "macos".into(),
        backend: "macos_taskpolicy".into(),
        backend_state: backend_state.into(),
        pid: None,
        unit_name: None,
        scaler_exe: scaler_exe.into(),
        scaler_version: env!("CARGO_PKG_VERSION").into(),
        stdout_log,
        stderr_log,
    }
}

fn redirect_stdio(
    stdout_log: &std::path::Path,
    stderr_log: &std::path::Path,
) -> std::io::Result<()> {
    let devnull = OpenOptions::new().read(true).open("/dev/null")?;
    let out = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(stdout_log)?;
    let err = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(stderr_log)?;
    // SAFETY: dup2 is always safe to call with valid fds. stdin (0),
    // stdout (1), and stderr (2) are always open in a process. The source
    // fds (`devnull`, `out`, `err`) are owned File values with live
    // descriptors. After dup2 the new fd references the same open file
    // description as the source fd; the originals remain open and are
    // closed when the File values drop at end of this function.
    unsafe {
        libc::dup2(devnull.as_raw_fd(), 0);
        libc::dup2(out.as_raw_fd(), 1);
        libc::dup2(err.as_raw_fd(), 2);
    }
    Ok(())
}

fn to_result_json(
    id: &RunId,
    outcome: Option<&crate::core::RunOutcome>,
    err: Option<&anyhow::Error>,
) -> RunResult {
    let ended = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc().to_offset(UtcOffset::UTC))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into());

    if let Some(o) = outcome {
        use std::os::unix::process::ExitStatusExt;
        if let Some(code) = o.exit_status.code() {
            return RunResult {
                version: 1,
                id: id.as_str().into(),
                ended,
                state: RunState::Exited,
                exit_code: Some(code),
                signal: None,
                cpu_total_nanos: o.total_cpu_nanos,
                memory_peak_bytes: o.peak_memory,
                launch_error: None,
            };
        }
        if let Some(sig) = o.exit_status.signal() {
            return RunResult {
                version: 1,
                id: id.as_str().into(),
                ended,
                state: RunState::Killed,
                exit_code: Some(128 + sig),
                signal: Some(macos_signal_name(sig).unwrap_or("signal").to_string()),
                cpu_total_nanos: o.total_cpu_nanos,
                memory_peak_bytes: o.peak_memory,
                launch_error: None,
            };
        }
    }
    RunResult {
        version: 1,
        id: id.as_str().into(),
        ended,
        state: RunState::LaunchFailed,
        exit_code: None,
        signal: None,
        cpu_total_nanos: None,
        memory_peak_bytes: None,
        launch_error: err.map(|e| format!("{e:#}")),
    }
}

fn macos_signal_name(signum: i32) -> Option<&'static str> {
    Some(match signum {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        6 => "SIGABRT",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        _ => return None,
    })
}

fn snapshot_from_ps(pid: i32) -> Result<LiveSnapshot> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=,%cpu=,etime=", "-p", &pid.to_string()])
        .output()
        .context("spawn ps")?;
    if !out.status.success() {
        anyhow::bail!("ps failed for pid {pid}");
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let mut parts = line.split_whitespace();
    let rss_kib: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let _pcpu: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let etime = parts.next().unwrap_or("");
    Ok(LiveSnapshot {
        cpu_total_nanos: None,
        memory_current_bytes: Some(rss_kib * 1024),
        elapsed_secs: parse_ps_etime(etime),
    })
}

/// Parse the `ps etime` column into total seconds.
///
/// `ps` on macOS emits one of four formats:
/// - `"ss"` — seconds only
/// - `"mm:ss"` — minutes:seconds
/// - `"hh:mm:ss"` — hours:minutes:seconds
/// - `"dd-hh:mm:ss"` — days-hours:minutes:seconds
///
/// Returns `None` on any parse failure or unexpected format.
fn parse_ps_etime(s: &str) -> Option<u64> {
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let pieces: Vec<&str> = rest.split(':').collect();
    let (h, m, sec) = match pieces.as_slice() {
        [s] => (0u64, 0u64, s.parse::<u64>().ok()?),
        [m, s] => (0, m.parse::<u64>().ok()?, s.parse::<u64>().ok()?),
        [h, m, s] => (
            h.parse::<u64>().ok()?,
            m.parse::<u64>().ok()?,
            s.parse::<u64>().ok()?,
        ),
        _ => return None,
    };
    Some(days * 86_400 + h * 3600 + m * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_etime_seconds() {
        assert_eq!(parse_ps_etime("42"), Some(42));
    }

    #[test]
    fn parse_etime_mm_ss() {
        assert_eq!(parse_ps_etime("02:05"), Some(125));
    }

    #[test]
    fn parse_etime_hh_mm_ss() {
        assert_eq!(parse_ps_etime("01:02:03"), Some(3723));
    }

    #[test]
    fn parse_etime_dd_hh_mm_ss() {
        assert_eq!(parse_ps_etime("2-01:00:00"), Some(2 * 86400 + 3600));
    }

    #[test]
    fn parse_etime_garbage() {
        assert_eq!(parse_ps_etime("garbage"), None);
    }
}
