use crate::core::{CapabilityReport, LaunchPlan, RunningHandle, Sample, Signal};

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
