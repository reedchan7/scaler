#[cfg(target_os = "macos")]
use crate::core::InteractiveMode;
use crate::core::{
    BackendKind, CapabilityLevel, CapabilityReport, LaunchPlan, RunningHandle, Sample, Signal,
};

#[cfg(target_os = "linux")]
pub mod linux_systemd;
#[cfg(target_os = "macos")]
pub mod macos_taskpolicy;

pub trait Backend {
    fn detect(&self) -> CapabilityReport;
    fn launch(&self, plan: &LaunchPlan) -> anyhow::Result<RunningHandle>;
    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>>;
    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample>;
    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()>;
}

const FORCE_BACKEND_ENV: &str = "SCALER_FORCE_BACKEND";

#[cfg(target_os = "linux")]
pub fn detect_host_capabilities() -> CapabilityReport {
    linux_systemd::detect_linux_capabilities(linux_systemd::probe_linux_host())
}

#[cfg(target_os = "macos")]
pub fn detect_host_capabilities() -> CapabilityReport {
    macos_taskpolicy::detect_macos_capabilities(
        macos_taskpolicy::probe_macos_host(),
        InteractiveMode::Auto,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn detect_host_capabilities() -> CapabilityReport {
    CapabilityReport::unsupported()
}

/// Returns the backend that `Command::Run` will actually use right now,
/// honoring the `SCALER_FORCE_BACKEND` test escape hatch when set.
pub fn select_backend() -> Box<dyn Backend> {
    if let Some(forced) = forced_backend() {
        return forced;
    }
    select_backend_from_capabilities()
}

/// Returns the same backend kind that `select_backend` would pick, without
/// instantiating it. Used by `doctor` so its `effective_backend:` line
/// matches what `run` would do.
pub fn effective_backend_kind() -> BackendKind {
    if let Some(kind) = forced_backend_kind() {
        return kind;
    }
    let report = detect_host_capabilities();
    if report.backend_state == CapabilityLevel::Unavailable {
        BackendKind::PlainFallback
    } else {
        report.backend
    }
}

fn forced_backend_kind() -> Option<BackendKind> {
    match std::env::var(FORCE_BACKEND_ENV).ok().as_deref() {
        Some("linux_systemd") => Some(BackendKind::LinuxSystemd),
        Some("macos_taskpolicy") => Some(BackendKind::MacosTaskpolicy),
        Some("plain_fallback") => Some(BackendKind::PlainFallback),
        _ => None,
    }
}

fn forced_backend() -> Option<Box<dyn Backend>> {
    let kind = forced_backend_kind()?;
    Some(boxed_backend_for_kind(kind))
}

#[cfg(target_os = "linux")]
fn select_backend_from_capabilities() -> Box<dyn Backend> {
    let report = detect_host_capabilities();
    if report.backend_state != CapabilityLevel::Unavailable {
        Box::new(linux_systemd::LinuxSystemdBackend)
    } else {
        Box::new(crate::core::run_loop::PlainFallbackBackend)
    }
}

#[cfg(target_os = "macos")]
fn select_backend_from_capabilities() -> Box<dyn Backend> {
    let report = detect_host_capabilities();
    if report.backend_state != CapabilityLevel::Unavailable {
        Box::new(macos_taskpolicy::MacosTaskpolicyBackend)
    } else {
        Box::new(crate::core::run_loop::PlainFallbackBackend)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn select_backend_from_capabilities() -> Box<dyn Backend> {
    Box::new(crate::core::run_loop::PlainFallbackBackend)
}

#[cfg(target_os = "linux")]
fn boxed_backend_for_kind(kind: BackendKind) -> Box<dyn Backend> {
    match kind {
        BackendKind::LinuxSystemd => Box::new(linux_systemd::LinuxSystemdBackend),
        _ => Box::new(crate::core::run_loop::PlainFallbackBackend),
    }
}

#[cfg(target_os = "macos")]
fn boxed_backend_for_kind(kind: BackendKind) -> Box<dyn Backend> {
    match kind {
        BackendKind::MacosTaskpolicy => Box::new(macos_taskpolicy::MacosTaskpolicyBackend),
        _ => Box::new(crate::core::run_loop::PlainFallbackBackend),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn boxed_backend_for_kind(_kind: BackendKind) -> Box<dyn Backend> {
    Box::new(crate::core::run_loop::PlainFallbackBackend)
}
