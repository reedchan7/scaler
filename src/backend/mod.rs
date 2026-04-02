use crate::core::{CapabilityReport, LaunchPlan, RunningHandle, Sample, Signal};

pub trait Backend {
    fn detect() -> CapabilityReport;
    fn launch(plan: &LaunchPlan) -> anyhow::Result<RunningHandle>;
    fn try_wait(handle: &mut RunningHandle) -> anyhow::Result<Option<std::process::ExitStatus>>;
    fn sample(handle: &RunningHandle) -> anyhow::Result<Sample>;
    fn terminate(handle: &RunningHandle, signal: Signal) -> anyhow::Result<()>;
}
