use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;

use crate::backend::Backend;
use crate::core::run_loop::{
    command_from_argv, preferred_io_mode, spawn_with_bookkeeping, try_wait_via_registry,
};
use crate::core::{
    BackendKind, CapabilityLevel, DoctorPrerequisite, LaunchPlan, Platform, PrerequisiteStatus,
    RunningHandle, Sample, ShellKind, Signal,
};

pub struct LinuxProbe {
    pub has_systemd_run: bool,
    pub has_cgroup_v2: bool,
    pub user_manager_reachable: bool,
}

pub fn build_systemd_run_argv(plan: &LaunchPlan, unit_name: &str) -> anyhow::Result<Vec<OsString>> {
    anyhow::ensure!(
        plan.platform == Platform::Linux,
        "linux systemd backend requires a linux launch plan"
    );
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");
    anyhow::ensure!(!unit_name.is_empty(), "unit name must not be empty");

    // We launch the child as a transient `.service` (NOT a `--scope`)
    // because:
    //   * `--scope` is incompatible with `--pipe`, and on systemd 255+
    //     `systemd-run --scope` with non-TTY stdio errors out with
    //     "--pty/--pipe is not compatible in timer or --scope mode."
    //   * `--pipe --wait` makes systemd-run stay in the foreground,
    //     forward the unit's stdout/stderr through its own pipes (which
    //     scaler captures), and propagate the unit's exit code.
    //   * `--collect` auto-removes the transient unit on exit so we
    //     don't leak failed units in `systemctl --user list-units`.
    //
    // The cgroup limits (CPUQuota / MemoryHigh / MemoryMax / MemorySwapMax)
    // still apply because they are properties of the transient service.
    let mut argv = vec![
        OsString::from("systemd-run"),
        OsString::from("--user"),
        OsString::from("--pipe"),
        OsString::from("--wait"),
        OsString::from("--collect"),
        // Suppress systemd-run's "Running as unit: ..." stderr line so
        // scaler's stderr only contains scaler-owned output.
        OsString::from("--quiet"),
        OsString::from(format!("--unit={unit_name}")),
    ];

    if let Some(cpu) = plan.resource_spec.cpu {
        argv.push(OsString::from(format!(
            "--property=CPUQuota={}%",
            cpu.centi_cores()
        )));
    }

    if let Some(mem) = plan.resource_spec.mem {
        let bytes = mem.bytes();
        let memory_high = ((u128::from(bytes) * 9) + 5) / 10;

        argv.push(OsString::from(format!(
            "--property=MemoryHigh={memory_high}"
        )));
        argv.push(OsString::from(format!("--property=MemoryMax={bytes}")));
        argv.push(OsString::from("--property=MemorySwapMax=0"));
    }

    argv.push(OsString::from("--"));

    match plan.resource_spec.shell {
        Some(shell) => {
            anyhow::ensure!(
                plan.argv.len() == 1,
                "shell launch plan requires exactly one script token"
            );
            argv.push(shell_program(shell));
            argv.push(OsString::from("-lc"));
            argv.push(plan.argv[0].clone());
        }
        None => argv.extend(plan.argv.iter().cloned()),
    }

    Ok(argv)
}

pub fn detect_linux_capabilities(probe: LinuxProbe) -> crate::core::CapabilityReport {
    let mut warnings = Vec::new();
    let all_prerequisites_satisfied =
        probe.has_systemd_run && probe.has_cgroup_v2 && probe.user_manager_reachable;
    let enforced_when_ready = if all_prerequisites_satisfied {
        CapabilityLevel::Enforced
    } else {
        CapabilityLevel::Unavailable
    };

    if !probe.has_systemd_run {
        warnings.push("systemd-run is not available in PATH".to_string());
    }

    if !probe.has_cgroup_v2 {
        warnings.push("unified cgroup v2 is not available".to_string());
    }

    if !probe.user_manager_reachable {
        warnings.push("systemd user manager is unreachable".to_string());
    }

    let prerequisites = vec![
        DoctorPrerequisite::check(
            "systemd_run",
            if probe.has_systemd_run {
                PrerequisiteStatus::Ok
            } else {
                PrerequisiteStatus::Missing
            },
        ),
        DoctorPrerequisite::check(
            "cgroup_v2",
            if probe.has_cgroup_v2 {
                PrerequisiteStatus::Ok
            } else {
                PrerequisiteStatus::Missing
            },
        ),
        DoctorPrerequisite::check(
            "user_manager",
            if !probe.has_systemd_run {
                PrerequisiteStatus::Skipped
            } else if probe.user_manager_reachable {
                PrerequisiteStatus::Ok
            } else {
                PrerequisiteStatus::Unreachable
            },
        ),
    ];

    crate::core::CapabilityReport {
        platform: Platform::Linux,
        backend: BackendKind::LinuxSystemd,
        backend_state: enforced_when_ready,
        cpu: enforced_when_ready,
        memory: enforced_when_ready,
        interactive: enforced_when_ready,
        prerequisites,
        warnings,
    }
}

pub fn probe_linux_host() -> LinuxProbe {
    let has_systemd_run = find_in_path("systemd-run").is_some();
    let has_cgroup_v2 = Path::new("/sys/fs/cgroup/cgroup.controllers").exists();
    let user_manager_reachable = if has_systemd_run {
        probe_user_manager()
    } else {
        true
    };

    LinuxProbe {
        has_systemd_run,
        has_cgroup_v2,
        user_manager_reachable,
    }
}

fn shell_program(shell: ShellKind) -> OsString {
    match shell {
        ShellKind::Sh => OsString::from("sh"),
        ShellKind::Bash => OsString::from("bash"),
        ShellKind::Zsh => OsString::from("zsh"),
    }
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;

    env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    match path.metadata() {
        Ok(metadata) => metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

fn probe_user_manager() -> bool {
    match Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "true"])
        .status()
        .context("failed to query systemd user manager")
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxSystemdBackend;

impl Backend for LinuxSystemdBackend {
    fn detect(&self) -> crate::core::CapabilityReport {
        detect_linux_capabilities(probe_linux_host())
    }

    fn launch(&self, plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        let io_mode = preferred_io_mode(plan.resource_spec.interactive);
        let unit_name = generate_unit_name();
        let argv = build_systemd_run_argv(plan, &unit_name)?;
        let command = command_from_argv(&argv, io_mode)?;
        let handle = spawn_with_bookkeeping(command, io_mode)?;
        unit_registry()
            .lock()
            .unwrap()
            .insert(handle.root_pid, UnitState::new(unit_name));
        Ok(handle)
    }

    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        let status = try_wait_via_registry(handle.root_pid)?;
        if status.is_some() {
            unit_registry().lock().unwrap().remove(&handle.root_pid);
        }
        Ok(status)
    }

    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample> {
        let main_pid = resolve_main_pid(handle.root_pid)
            .context("transient unit MainPID not yet available")?;
        crate::core::sampling::sample_process_tree(main_pid)
    }

    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        let unit_name = unit_registry()
            .lock()
            .unwrap()
            .get(&handle.root_pid)
            .map(|state| state.unit_name.clone());
        let Some(unit_name) = unit_name else {
            // Unit already gone (e.g. process exited between try_wait and
            // terminate); nothing to do.
            return Ok(());
        };
        let signal_flag = match signal {
            Signal::Interrupt => "SIGINT",
            Signal::Terminate => "SIGTERM",
            Signal::Kill => "SIGKILL",
        };
        let status = Command::new("systemctl")
            .args([
                "--user",
                "kill",
                "--kill-whom=all",
                &format!("--signal={signal_flag}"),
                &unit_name,
            ])
            .status()
            .with_context(|| format!("failed to send {signal_flag} to unit {unit_name}"))?;
        // Don't hard-fail on non-zero status: the unit may have just exited.
        let _ = status;
        Ok(())
    }
}

#[derive(Debug)]
struct UnitState {
    unit_name: String,
    cached_main_pid: Option<u32>,
}

impl UnitState {
    fn new(unit_name: String) -> Self {
        Self {
            unit_name,
            cached_main_pid: None,
        }
    }
}

fn unit_registry() -> &'static Mutex<HashMap<u32, UnitState>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u32, UnitState>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn generate_unit_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("scaler-run-{}-{nanos}.service", std::process::id())
}

fn resolve_main_pid(root_pid: u32) -> Option<u32> {
    let unit_name = {
        let registry = unit_registry().lock().unwrap();
        let state = registry.get(&root_pid)?;
        if let Some(cached) = state.cached_main_pid {
            return Some(cached);
        }
        state.unit_name.clone()
    };
    let main_pid = query_main_pid(&unit_name)?;
    let mut registry = unit_registry().lock().unwrap();
    if let Some(state) = registry.get_mut(&root_pid) {
        state.cached_main_pid = Some(main_pid);
    }
    Some(main_pid)
}

fn query_main_pid(unit_name: &str) -> Option<u32> {
    let output = Command::new("systemctl")
        .args(["--user", "show", "--property=MainPID", "--value", unit_name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()?;
    if value == 0 { None } else { Some(value) }
}

/// Test seam: returns the argv that `LinuxSystemdBackend.launch` would
/// hand to `command_from_argv`. Used by integration tests so they can
/// assert on the wiring without spawning a real process.
#[doc(hidden)]
pub fn linux_systemd_command_preview_for_test(
    plan: &LaunchPlan,
) -> anyhow::Result<Vec<std::ffi::OsString>> {
    build_systemd_run_argv(plan, "scaler-run-test.service")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::is_executable;

    #[test]
    fn is_executable_requires_execute_permissions() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("scaler-linux-systemd-{unique}"));
        let candidate = temp_dir.join("systemd-run");

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&candidate, b"#!/bin/sh\n").unwrap();

        let mut permissions = fs::metadata(&candidate).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&candidate, permissions).unwrap();
        assert!(!is_executable(&candidate));

        let mut permissions = fs::metadata(&candidate).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&candidate, permissions).unwrap();
        assert!(is_executable(&candidate));

        fs::remove_file(&candidate).unwrap();
        fs::remove_dir(&temp_dir).unwrap();
    }
}
