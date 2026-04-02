use std::{
    ffi::OsString,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use assert_cmd::cargo::cargo_bin;
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
        backend.recorded_signal_order(),
        vec![Signal::Interrupt, Signal::Terminate, Signal::Kill]
    );
    let timings = backend.recorded_signal_timings();
    assert!(timings[0] < Duration::from_millis(20));
    assert!(timings[1] >= Duration::from_millis(20));
    assert!(timings[1] < Duration::from_millis(80));
    assert!(timings[2] >= Duration::from_millis(40));
    assert!(timings[2] < Duration::from_millis(120));
}

#[test]
fn os_sigint_triggers_interrupt_flow_when_signal_bridge_is_active() {
    let child = std::process::Command::new(cargo_bin("scaler"))
        .args([
            "run",
            "--",
            "/bin/sh",
            "-lc",
            "trap 'exit 130' INT; while true; do sleep 1; done",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    std::thread::sleep(Duration::from_secs(1));
    let signal_status = std::process::Command::new("kill")
        .arg("-INT")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(signal_status.success());

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("exit_status:"));
    assert!(stdout.contains("runtime:"));
}

#[derive(Default)]
struct RecordingBackend {
    launched_at: Arc<Mutex<Option<Instant>>>,
    signals: Arc<Mutex<Vec<(Signal, Duration)>>>,
}

impl RecordingBackend {
    fn recorded_signal_order(&self) -> Vec<Signal> {
        self.signals
            .lock()
            .unwrap()
            .iter()
            .map(|(signal, _)| *signal)
            .collect()
    }

    fn recorded_signal_timings(&self) -> Vec<Duration> {
        self.signals
            .lock()
            .unwrap()
            .iter()
            .map(|(_, elapsed)| *elapsed)
            .collect()
    }
}

impl Backend for RecordingBackend {
    fn detect(&self) -> CapabilityReport {
        CapabilityReport::unsupported()
    }

    fn launch(&self, _plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        *self.launched_at.lock().unwrap() = Some(Instant::now());
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
        let launched_at = *self.launched_at.lock().unwrap();
        let elapsed = launched_at.unwrap().elapsed();
        self.signals.lock().unwrap().push((signal, elapsed));
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
