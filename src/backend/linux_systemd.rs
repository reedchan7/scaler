use std::{
    env,
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;

use crate::backend::Backend;
use crate::core::run_loop::{
    command_from_argv, preferred_io_mode, spawn_with_bookkeeping, terminate_process_group,
    try_wait_via_registry,
};
use crate::core::{
    BackendKind, CapabilityLevel, DoctorPrerequisite, InteractiveMode, LaunchPlan, Platform,
    PrerequisiteStatus, RunningHandle, Sample, ShellKind, Signal,
};

pub struct LinuxProbe {
    pub has_systemd_run: bool,
    pub has_cgroup_v2: bool,
    pub user_manager_reachable: bool,
}

pub fn build_systemd_run_argv(plan: &LaunchPlan) -> anyhow::Result<Vec<OsString>> {
    anyhow::ensure!(
        plan.platform == Platform::Linux,
        "linux systemd backend requires a linux launch plan"
    );
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    let mut argv = vec![
        OsString::from("systemd-run"),
        OsString::from("--user"),
        OsString::from("--scope"),
    ];

    if plan.resource_spec.interactive == InteractiveMode::Always {
        argv.push(OsString::from("--pty"));
    }

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
        let argv = build_systemd_run_argv(plan)?;
        let command = command_from_argv(&argv, io_mode)?;
        spawn_with_bookkeeping(command, io_mode)
    }

    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        try_wait_via_registry(handle.root_pid)
    }

    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample> {
        crate::core::sampling::sample_process_tree(handle.root_pid)
    }

    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        terminate_process_group(handle.root_pid, signal)
    }
}

/// Test seam: returns the argv that `LinuxSystemdBackend.launch` would
/// hand to `command_from_argv`. Used by integration tests so they can
/// assert on the wiring without spawning a real process.
#[doc(hidden)]
pub fn linux_systemd_command_preview_for_test(
    plan: &LaunchPlan,
) -> anyhow::Result<Vec<std::ffi::OsString>> {
    build_systemd_run_argv(plan)
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
