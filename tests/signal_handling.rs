use std::{
    ffi::OsString,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use scaler::{
    backend::Backend,
    core::{
        CapabilityReport, IoMode, LaunchPlan, Platform, ResourceSpec, RunOutcome, RunningHandle,
        Sample, Signal,
        run_loop::{
            InterruptPlan, execute, request_interrupt_for_test, reset_test_state,
            set_test_interrupt_plan_for_next_run, set_test_poll_interval_for_next_run,
        },
    },
};

#[test]
fn interrupt_plan_is_sigint_then_sigterm_then_sigkill() {
    let plan = InterruptPlan::default();

    assert_eq!(plan.sigterm_after().as_secs(), 2);
    assert_eq!(plan.sigkill_after().as_secs(), 5);
}

#[test]
fn execute_escalates_interrupts_in_order() {
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(5));
    set_test_interrupt_plan_for_next_run(Duration::from_millis(20), Duration::from_millis(40));

    let backend = RecordingBackend::default();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1));
        request_interrupt_for_test();
    });

    let outcome = execute(
        LaunchPlan {
            argv: vec![OsString::from("fake-command")],
            resource_spec: ResourceSpec::default(),
            platform: host_platform(),
        },
        &backend,
    )
    .unwrap();

    assert!(outcome.exit_status.success());
    assert_eq!(
        backend.recorded_signals(),
        vec![Signal::Interrupt, Signal::Terminate, Signal::Kill]
    );
}

#[derive(Default)]
struct RecordingBackend {
    signals: Arc<Mutex<Vec<Signal>>>,
}

impl RecordingBackend {
    fn recorded_signals(&self) -> Vec<Signal> {
        self.signals.lock().unwrap().clone()
    }
}

impl Backend for RecordingBackend {
    fn detect(&self) -> CapabilityReport {
        CapabilityReport::unsupported()
    }

    fn launch(&self, _plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        Ok(RunningHandle {
            root_pid: 4242,
            launch_time: SystemTime::now(),
            io_mode: IoMode::Pipes,
        })
    }

    fn try_wait(
        &self,
        _handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        if self.signals.lock().unwrap().len() >= 3 {
            return Ok(Some(RunOutcome::fixture_for_test().exit_status));
        }

        Ok(None)
    }

    fn sample(&self, _handle: &RunningHandle) -> anyhow::Result<Sample> {
        Ok(Sample {
            captured_at: SystemTime::now(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            peak_memory_bytes: Some(0),
            child_process_count: Some(1),
        })
    }

    fn terminate(&self, _handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        self.signals.lock().unwrap().push(signal);
        Ok(())
    }
}

fn host_platform() -> Platform {
    match std::env::consts::OS {
        "linux" => Platform::Linux,
        "macos" => Platform::Macos,
        _ => Platform::Unsupported,
    }
}
