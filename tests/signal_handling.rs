use std::{
    ffi::OsString,
    io::{Read, Write},
    process::{ExitStatus, Stdio},
    sync::mpsc,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

use assert_cmd::cargo::cargo_bin;
use scaler::{
    backend::Backend,
    core::{
        CapabilityReport, InteractiveMode, IoMode, LaunchPlan, Platform, ResourceSpec, RunOutcome,
        RunningHandle, Sample, Signal,
        run_loop::{
            InterruptPlan, PlainFallbackBackend, execute, request_interrupt_for_test,
            reset_test_state, set_test_interrupt_plan_for_next_run,
            set_test_poll_interval_for_next_run,
        },
    },
};
use tempfile::NamedTempFile;

#[test]
fn interrupt_plan_is_sigint_then_sigterm_then_sigkill() {
    let plan = InterruptPlan::default();

    assert_eq!(plan.sigterm_after().as_secs(), 2);
    assert_eq!(plan.sigkill_after().as_secs(), 5);
}

#[test]
fn execute_escalates_interrupts_in_order() {
    let _guard = signal_test_guard();
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
    assert!(timings[1] >= Duration::from_millis(20));
    assert!(timings[1] < Duration::from_millis(120));
    assert!(timings[2] >= Duration::from_millis(40));
    assert!(timings[2] < Duration::from_millis(180));
}

#[test]
fn os_sigint_triggers_interrupt_flow_when_signal_bridge_is_active() {
    let _guard = signal_test_guard();
    let mut child = std::process::Command::new(cargo_bin("scaler"))
        .args([
            "run",
            "--",
            "/bin/sh",
            "-lc",
            "printf ready; trap 'exit 130' INT; while true; do sleep 1; done",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<[u8; 5]>();
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 5];
        stdout.read_exact(&mut buffer).unwrap();
        let _ = tx.send(buffer);
        let mut rest = Vec::new();
        let _ = stdout.read_to_end(&mut rest);
        let mut full_output = buffer.to_vec();
        full_output.extend(rest);
        full_output
    });

    let first_bytes = rx.recv_timeout(Duration::from_millis(490)).unwrap();
    assert_eq!(&first_bytes, b"ready");

    let signal_status = std::process::Command::new("kill")
        .arg("-INT")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(signal_status.success());

    let status = child.wait().unwrap();
    let stdout = String::from_utf8(reader.join().unwrap()).unwrap();
    assert_eq!(resolved_code(status), Some(130));
    assert!(stdout.contains("exit_status:"));
    assert!(stdout.contains("runtime:"));
}

#[test]
fn plain_fallback_termination_targets_the_process_group() {
    let _guard = signal_test_guard();
    reset_test_state();
    set_test_poll_interval_for_next_run(Duration::from_millis(5));
    set_test_interrupt_plan_for_next_run(Duration::from_millis(20), Duration::from_millis(40));

    let pidfile = NamedTempFile::new().unwrap();
    let pidfile_path = pidfile.path().to_string_lossy().into_owned();
    let mut script = NamedTempFile::new().unwrap();
    write!(
        script,
        "#!/bin/sh\ntrap '' INT TERM\n/bin/sh -lc \"echo \\$\\$ > '$1'; trap '' INT TERM; while :; do sleep 1; done\" &\nwhile :; do sleep 1; done\n"
    )
    .unwrap();
    script.flush().unwrap();
    let script_path = script.path().to_string_lossy().into_owned();

    let backend = PlainFallbackBackend;
    let pidfile_for_interrupt = pidfile_path.clone();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if std::fs::read_to_string(&pidfile_for_interrupt)
                .map(|contents| !contents.trim().is_empty())
                .unwrap_or(false)
            {
                request_interrupt_for_test();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        request_interrupt_for_test();
    });

    let outcome = execute(
        LaunchPlan {
            argv: vec![
                OsString::from("/bin/sh"),
                OsString::from(script_path),
                OsString::from(pidfile_path),
            ],
            resource_spec: ResourceSpec {
                interactive: InteractiveMode::Never,
                ..ResourceSpec::default()
            },
            platform: host_platform(),
        },
        &backend,
    )
    .unwrap();

    let child_pid = std::fs::read_to_string(pidfile.path())
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(child_pid.to_string())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();

    assert!(!outcome.exit_status.success());
    assert!(!alive);
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

fn signal_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static SIGNAL_TEST_GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    SIGNAL_TEST_GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap()
}

fn resolved_code(status: ExitStatus) -> Option<i32> {
    if let Some(code) = status.code() {
        return Some(code);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        status.signal().map(|signal| 128 + signal)
    }

    #[cfg(not(unix))]
    {
        None
    }
}
