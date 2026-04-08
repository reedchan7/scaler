use std::{
    ffi::OsString,
    process::ExitStatus,
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuLimit(u32);

impl CpuLimit {
    pub fn from_centi_cores(value: u32) -> Self {
        Self(value)
    }

    pub fn centi_cores(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimit(u64);

impl MemoryLimit {
    pub fn from_bytes(value: u64) -> Self {
        Self(value)
    }

    pub fn bytes(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
    Unsupported,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    LinuxSystemd,
    MacosTaskpolicy,
    PlainFallback,
    Unsupported,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxSystemd => "linux_systemd",
            Self::MacosTaskpolicy => "macos_taskpolicy",
            Self::PlainFallback => "plain_fallback",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityLevel {
    Enforced,
    BestEffort,
    Unavailable,
}

impl CapabilityLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::BestEffort => "best_effort",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrerequisiteStatus {
    Ok,
    Missing,
    Unreachable,
    Unsupported,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorPrerequisite {
    Check {
        key: &'static str,
        status: PrerequisiteStatus,
    },
    Note(&'static str),
}

impl DoctorPrerequisite {
    pub fn check(key: &'static str, status: PrerequisiteStatus) -> Self {
        Self::Check { key, status }
    }

    pub fn note(message: &'static str) -> Self {
        Self::Note(message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Interrupt,
    Terminate,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Sh,
    Bash,
    Zsh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    Pty,
    Pipes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
    PtyMerged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSpec {
    pub cpu: Option<CpuLimit>,
    pub mem: Option<MemoryLimit>,
    pub interactive: InteractiveMode,
    pub shell: Option<ShellKind>,
    pub monitor: bool,
}

impl Default for ResourceSpec {
    fn default() -> Self {
        Self {
            cpu: None,
            mem: None,
            interactive: InteractiveMode::Auto,
            shell: None,
            monitor: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<OsString>,
    pub resource_spec: ResourceSpec,
    pub platform: Platform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReport {
    pub platform: Platform,
    pub backend: BackendKind,
    pub backend_state: CapabilityLevel,
    pub cpu: CapabilityLevel,
    pub memory: CapabilityLevel,
    pub interactive: CapabilityLevel,
    pub prerequisites: Vec<DoctorPrerequisite>,
    pub warnings: Vec<String>,
}

impl CapabilityReport {
    pub fn unsupported() -> Self {
        Self {
            platform: Platform::Unsupported,
            backend: BackendKind::Unsupported,
            backend_state: CapabilityLevel::Unavailable,
            cpu: CapabilityLevel::Unavailable,
            memory: CapabilityLevel::Unavailable,
            interactive: CapabilityLevel::Unavailable,
            prerequisites: vec![DoctorPrerequisite::note(
                "no supported backend for this host",
            )],
            warnings: Vec::new(),
        }
    }

    /// Test-only "everything is fine" preset. Used by `RunOutcome::fixture_for_test`
    /// so summary unit tests don't accidentally trigger the context block.
    #[doc(hidden)]
    pub fn fully_enforced_for_test() -> Self {
        Self {
            platform: Platform::Linux,
            backend: BackendKind::LinuxSystemd,
            backend_state: CapabilityLevel::Enforced,
            cpu: CapabilityLevel::Enforced,
            memory: CapabilityLevel::Enforced,
            interactive: CapabilityLevel::Enforced,
            prerequisites: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningHandle {
    pub root_pid: u32,
    pub launch_time: SystemTime,
    pub io_mode: IoMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub captured_at: SystemTime,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub peak_memory_bytes: Option<u64>,
    pub child_process_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummarySample {
    pub captured_at: SystemTime,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFrame {
    pub sequence: u64,
    pub captured_at: SystemTime,
    pub stream: OutputStream,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub exit_status: ExitStatus,
    pub elapsed: Duration,
    pub peak_memory: Option<u64>,
    /// Memory cap requested via `--mem`, in bytes. Used by the summary
    /// to show peak usage as a percent of the user's own budget.
    pub mem_limit_bytes: Option<u64>,
    /// Host physical memory in bytes. Used by the summary as a fallback
    /// denominator when `--mem` was not supplied, so the memory row
    /// still has a meaningful percentage attached.
    pub system_memory_bytes: Option<u64>,
    pub samples: Vec<SummarySample>,
    /// CPU cap requested via `--cpu`, in centi-cores. Used by the summary
    /// to show the cpu peak as a percent of the user's own budget.
    pub cpu_limit_centi_cores: Option<u32>,
    /// Host logical core count. Used by the summary as a fallback
    /// denominator when `--cpu` was not supplied.
    pub host_logical_cores: Option<u32>,
    /// Capability snapshot from the backend at launch time. Drives the
    /// context block above the summary card so users can see exactly
    /// which guarantees applied for the run that just finished.
    pub capabilities: CapabilityReport,
    /// Warnings accumulated over the run (initial backend warnings plus
    /// any added by the run loop, e.g. when the monitor TUI fails after
    /// launch and we fall back to plain rendering).
    pub warnings: Vec<String>,
    /// Total CPU time consumed by the child, in nanoseconds. Populated on
    /// Linux via cgroup accounting (`CPUUsageNSec`); `None` on macOS and
    /// other platforms where the backend does not report wall-clock CPU time.
    pub total_cpu_nanos: Option<u128>,
}
