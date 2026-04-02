use crate::core::{CapabilityReport, LaunchPlan, RunningHandle, Sample, Signal};

#[cfg(target_os = "linux")]
pub mod linux_systemd;

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

#[cfg(target_os = "linux")]
pub fn detect_host_capabilities() -> CapabilityReport {
    linux_systemd::detect_linux_capabilities(linux_systemd::probe_linux_host())
}

#[cfg(not(target_os = "linux"))]
pub fn detect_host_capabilities() -> CapabilityReport {
    CapabilityReport::unsupported()
}
