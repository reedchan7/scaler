use std::process::ExitStatus;

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
    Unsupported,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxSystemd => "linux-systemd",
            Self::MacosTaskpolicy => "macos-taskpolicy",
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceSpec {
    pub cpu: Option<CpuLimit>,
    pub memory: Option<MemoryLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<String>,
    pub shell: Option<ShellKind>,
    pub interactive: InteractiveMode,
    pub io_mode: IoMode,
    pub resources: ResourceSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReport {
    pub platform: Platform,
    pub backend: BackendKind,
    pub backend_state: CapabilityLevel,
    pub cpu: CapabilityLevel,
    pub memory: CapabilityLevel,
    pub interactive: CapabilityLevel,
    pub prerequisite: &'static str,
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
            prerequisite: "no supported backend for this host",
        }
    }
}

#[derive(Debug)]
pub struct RunningHandle {
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sample {
    pub cpu_micros: Option<u64>,
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SummarySample {
    pub peak_cpu_micros: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFrame {
    pub stream: OutputStream,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub status: ExitStatus,
    pub summary: SummarySample,
    pub output: Vec<OutputFrame>,
}
