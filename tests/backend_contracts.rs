use scaler::{
    backend::Backend,
    core::{CapabilityReport, LaunchPlan, RunningHandle, Sample, Signal},
};

struct DummyBackend;

impl Backend for DummyBackend {
    fn detect(&self) -> CapabilityReport {
        CapabilityReport::unsupported()
    }

    fn launch(&self, _plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        unimplemented!()
    }

    fn try_wait(
        &self,
        _handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        unimplemented!()
    }

    fn sample(&self, _handle: &RunningHandle) -> anyhow::Result<Sample> {
        unimplemented!()
    }

    fn terminate(&self, _handle: &RunningHandle, _signal: Signal) -> anyhow::Result<()> {
        unimplemented!()
    }
}

#[test]
fn backend_trait_is_object_safe() {
    let backend: Box<dyn Backend> = Box::new(DummyBackend);
    let report = backend.detect();

    assert_eq!(report.platform.as_str(), "unsupported");
}
