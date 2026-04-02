use std::{
    env,
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::Context;

use crate::core::{
    BackendKind, CapabilityLevel, CapabilityReport, InteractiveMode, LaunchPlan, Platform,
    ShellKind,
};

pub struct MacosProbe {
    pub has_taskpolicy: bool,
    pub has_renice: bool,
    pub has_memory_support: bool,
    pub has_pty_support: bool,
    pub platform_version_supported: bool,
}

pub fn build_taskpolicy_argv(plan: &LaunchPlan) -> anyhow::Result<Vec<OsString>> {
    anyhow::ensure!(
        plan.platform == Platform::Macos,
        "macos taskpolicy backend requires a macos launch plan"
    );
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    let mut argv = vec![
        OsString::from("taskpolicy"),
        OsString::from("-b"),
        OsString::from("-d"),
        OsString::from("throttle"),
        OsString::from("-g"),
        OsString::from("default"),
        OsString::from("--"),
    ];

    if let Some(mem) = plan.resource_spec.mem {
        let mib = mem.bytes().div_ceil(1_048_576);
        argv.pop();
        argv.push(OsString::from("-m"));
        argv.push(OsString::from(mib.to_string()));
        argv.push(OsString::from("--"));
    }

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

pub fn detect_macos_capabilities(
    probe: MacosProbe,
    interactive: InteractiveMode,
) -> CapabilityReport {
    let mut warnings = Vec::new();
    let backend_available = probe.has_taskpolicy && probe.platform_version_supported;

    if !probe.has_taskpolicy {
        warnings.push("taskpolicy is not available in PATH".to_string());
    }

    if !probe.platform_version_supported {
        warnings.push("macOS platform version is not supported by the taskpolicy backend".into());
    }

    if !probe.has_renice {
        warnings.push("renice is not available; CPU lowering is best-effort only".to_string());
    }

    if !probe.has_memory_support {
        warnings.push("taskpolicy memory support is unavailable on this host".to_string());
    }

    if !probe.has_pty_support {
        warnings.push("PTY support is unavailable for interactive taskpolicy launches".into());
    }

    let backend_state = capability_when_backend_ready(backend_available);
    let cpu = capability_when_backend_ready(backend_available);
    let memory = if backend_available && probe.has_memory_support {
        CapabilityLevel::BestEffort
    } else {
        CapabilityLevel::Unavailable
    };
    let interactive = if !backend_available {
        CapabilityLevel::Unavailable
    } else {
        match interactive {
            InteractiveMode::Never => CapabilityLevel::BestEffort,
            InteractiveMode::Auto => CapabilityLevel::BestEffort,
            InteractiveMode::Always if probe.has_pty_support => CapabilityLevel::BestEffort,
            InteractiveMode::Always => CapabilityLevel::Unavailable,
        }
    };

    CapabilityReport {
        platform: Platform::Macos,
        backend: BackendKind::MacosTaskpolicy,
        backend_state,
        cpu,
        memory,
        interactive,
        warnings,
    }
}

pub fn probe_macos_host() -> MacosProbe {
    let has_taskpolicy = find_in_path("taskpolicy").is_some();

    MacosProbe {
        has_taskpolicy,
        has_renice: find_in_path("renice").is_some(),
        has_memory_support: has_taskpolicy && probe_memory_support(),
        has_pty_support: probe_pty_support(),
        platform_version_supported: probe_supported_platform_version(),
    }
}

fn capability_when_backend_ready(ready: bool) -> CapabilityLevel {
    if ready {
        CapabilityLevel::BestEffort
    } else {
        CapabilityLevel::Unavailable
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

fn probe_memory_support() -> bool {
    run_quiet_command("taskpolicy", ["-m", "1", "--", "true"])
}

fn probe_pty_support() -> bool {
    run_quiet_command("script", ["-q", "/dev/null", "true"])
}

fn probe_supported_platform_version() -> bool {
    let version = Command::new("sw_vers")
        .arg("-productVersion")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("failed to query macOS version");

    let Ok(output) = version else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let version = String::from_utf8_lossy(&output.stdout);
    let major = version
        .trim()
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok());

    matches!(major, Some(major) if major >= 11)
}

fn run_quiet_command<const N: usize>(program: &str, args: [&str; N]) -> bool {
    match Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run probe command: {program}"))
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}
