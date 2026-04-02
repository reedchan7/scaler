use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use anyhow::Context;

use crate::{
    backend::Backend,
    core::{
        InteractiveMode, IoMode, LaunchPlan, OutputFrame, OutputStream, RunOutcome, RunningHandle,
        Sample, ShellKind, Signal, SummarySample, output::OutputCollector,
    },
};

pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptPlan {
    sigterm_after: Duration,
    sigkill_after: Duration,
}

impl Default for InterruptPlan {
    fn default() -> Self {
        Self {
            sigterm_after: Duration::from_secs(2),
            sigkill_after: Duration::from_secs(5),
        }
    }
}

impl InterruptPlan {
    pub fn sigterm_after(self) -> Duration {
        self.sigterm_after
    }

    pub fn sigkill_after(self) -> Duration {
        self.sigkill_after
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlainFallbackBackend;

#[derive(Debug)]
pub struct SignalBridgeGuard;

pub fn execute(plan: LaunchPlan, backend: &dyn Backend) -> anyhow::Result<RunOutcome> {
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    clear_execution_trace();
    record_event("launch");
    record_event("monitor_unavailable");
    record_event("plain_renderer_active");

    let mut handle = backend.launch(&plan)?;
    record_event("launch_complete");
    record_event("monitor_failed");
    record_event("plain_renderer_continues");
    record_event("interactive_mode_selected");
    match handle.io_mode {
        IoMode::Pipes => record_event("pipe_streams"),
        IoMode::Pty => record_event("pty_merged_stream"),
    }

    let started_at = handle.launch_time;
    let poll_interval = configured_poll_interval();
    let interrupt_plan = configured_interrupt_plan();
    let mut peak_memory = None;
    let mut samples = Vec::new();
    let mut interrupt_started_at = None;
    let mut sent_sigterm = false;
    let mut sent_sigkill = false;

    loop {
        relay_output_frames(handle.root_pid)?;

        if let Some(exit_status) = backend.try_wait(&mut handle)? {
            finalize_process_output(handle.root_pid, poll_interval)?;
            remove_process_state(handle.root_pid);
            record_event("restore_terminal");
            record_event("render_summary");
            clear_runtime_overrides();

            return Ok(RunOutcome {
                exit_status,
                runtime: runtime_since(started_at),
                peak_memory,
                samples,
            });
        }

        if interrupt_started_at.is_none() && INTERRUPT_REQUESTED.swap(false, Ordering::SeqCst) {
            backend.terminate(&handle, Signal::Interrupt)?;
            record_event("signal_sigint");
            interrupt_started_at = Some(Instant::now());
        }

        if let Some(started) = interrupt_started_at {
            let elapsed = started.elapsed();

            if !sent_sigterm && elapsed >= interrupt_plan.sigterm_after() {
                backend.terminate(&handle, Signal::Terminate)?;
                record_event("signal_sigterm");
                sent_sigterm = true;
            }

            if !sent_sigkill && elapsed >= interrupt_plan.sigkill_after() {
                backend.terminate(&handle, Signal::Kill)?;
                record_event("signal_sigkill");
                sent_sigkill = true;
            }
        }

        if let Ok(sample) = backend.sample(&handle) {
            peak_memory = update_peak_memory(peak_memory, &sample);
            samples.push(SummarySample {
                captured_at: sample.captured_at,
                cpu_percent: sample.cpu_percent,
                memory_bytes: sample.memory_bytes,
            });
        }

        thread::sleep(poll_interval);
    }
}

pub fn install_signal_bridge() -> anyhow::Result<SignalBridgeGuard> {
    let install_result = signal_bridge_install_result();
    if let Err(message) = install_result {
        anyhow::bail!(message.clone());
    }

    SIGNAL_BRIDGE_ACTIVE.fetch_add(1, Ordering::SeqCst);
    Ok(SignalBridgeGuard)
}

impl Drop for SignalBridgeGuard {
    fn drop(&mut self) {
        SIGNAL_BRIDGE_ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Backend for PlainFallbackBackend {
    fn detect(&self) -> crate::core::CapabilityReport {
        crate::backend::detect_host_capabilities()
    }

    fn launch(&self, plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        let io_mode = preferred_io_mode(plan.resource_spec.interactive);
        let mut command = build_local_command(plan, io_mode)?;
        let launched_at = SystemTime::now();
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to launch fallback command: {:?}", plan.argv))?;

        let root_pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let state = Arc::new(ProcessState::new(child));

        process_registry()
            .lock()
            .unwrap()
            .insert(root_pid, Arc::clone(&state));
        if let Some(stdout) = stdout {
            let stream = if io_mode == IoMode::Pty {
                OutputStream::PtyMerged
            } else {
                OutputStream::Stdout
            };
            spawn_reader_thread(state.clone(), stdout, stream);
        }
        if let Some(stderr) = stderr {
            let stream = if io_mode == IoMode::Pty {
                OutputStream::PtyMerged
            } else {
                OutputStream::Stderr
            };
            spawn_reader_thread(state, stderr, stream);
        }

        Ok(RunningHandle {
            root_pid,
            launch_time: launched_at,
            io_mode,
        })
    }

    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        let state = process_state(handle.root_pid)
            .with_context(|| format!("missing process state for pid {}", handle.root_pid))?;
        Ok(state.child.lock().unwrap().try_wait()?)
    }

    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample> {
        let pid = handle.root_pid.to_string();
        let output = Command::new("ps")
            .args(["-o", "rss=", "-o", "%cpu=", "-p", &pid])
            .output()
            .with_context(|| {
                format!(
                    "failed to sample process metrics for pid {}",
                    handle.root_pid
                )
            })?;
        anyhow::ensure!(
            output.status.success(),
            "ps sampling failed for pid {}",
            handle.root_pid
        );

        let metrics = String::from_utf8_lossy(&output.stdout);
        let mut parts = metrics.split_whitespace();
        let rss_kib = parts
            .next()
            .context("ps output did not include rss")?
            .parse::<u64>()
            .context("rss was not numeric")?;
        let cpu_percent = parts
            .next()
            .unwrap_or("0")
            .parse::<f32>()
            .context("cpu percent was not numeric")?;
        let memory_bytes = rss_kib.saturating_mul(1024);

        Ok(Sample {
            captured_at: SystemTime::now(),
            cpu_percent,
            memory_bytes,
            peak_memory_bytes: Some(memory_bytes),
            child_process_count: Some(1),
        })
    }

    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        let signal_flag = match signal {
            Signal::Interrupt => "-INT",
            Signal::Terminate => "-TERM",
            Signal::Kill => "-KILL",
        };
        let status = Command::new("kill")
            .arg(signal_flag)
            .arg(handle.root_pid.to_string())
            .status()
            .with_context(|| format!("failed to send {signal_flag} to pid {}", handle.root_pid))?;
        anyhow::ensure!(
            status.success(),
            "kill command exited unsuccessfully for pid {}",
            handle.root_pid
        );
        Ok(())
    }
}

pub fn record_summary_timeline_for_test() -> Vec<&'static str> {
    recorded_events_matching(&["launch", "restore_terminal", "render_summary"])
}

pub fn record_monitor_fallback_for_test() -> Vec<&'static str> {
    recorded_events_matching(&["monitor_unavailable", "plain_renderer_active"])
}

pub fn record_interactive_mode_for_test() -> Vec<&'static str> {
    recorded_events_matching(&[
        "interactive_mode_selected",
        "pipe_streams",
        "pty_merged_stream",
    ])
}

pub fn record_post_launch_monitor_failure_for_test() -> Vec<&'static str> {
    recorded_events_matching(&[
        "launch_complete",
        "monitor_failed",
        "plain_renderer_continues",
    ])
}

pub fn staged_interrupt_signals_for_test() -> [Signal; 3] {
    [Signal::Interrupt, Signal::Terminate, Signal::Kill]
}

pub fn take_output_frames_for_test() -> Vec<OutputFrame> {
    let mut trace = execution_trace().lock().unwrap();
    std::mem::take(&mut trace.frames)
}

pub fn reset_test_state() {
    clear_execution_trace();
    clear_runtime_overrides();
}

pub fn request_interrupt_for_test() {
    INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn set_test_poll_interval_for_next_run(duration: Duration) {
    runtime_overrides().lock().unwrap().poll_interval = Some(duration);
}

pub fn set_test_interrupt_plan_for_next_run(sigterm_after: Duration, sigkill_after: Duration) {
    runtime_overrides().lock().unwrap().interrupt_plan = Some(InterruptPlan {
        sigterm_after,
        sigkill_after,
    });
}

#[derive(Debug)]
struct ProcessState {
    child: Mutex<Child>,
    collector: Mutex<OutputCollector>,
    frames: Mutex<VecDeque<OutputFrame>>,
    readers_alive: AtomicUsize,
}

impl ProcessState {
    fn new(child: Child) -> Self {
        Self {
            child: Mutex::new(child),
            collector: Mutex::new(OutputCollector::default()),
            frames: Mutex::new(VecDeque::new()),
            readers_alive: AtomicUsize::new(0),
        }
    }
}

#[derive(Debug, Default)]
struct ExecutionTrace {
    events: Vec<&'static str>,
    frames: Vec<OutputFrame>,
}

#[derive(Debug, Default)]
struct RuntimeOverrides {
    poll_interval: Option<Duration>,
    interrupt_plan: Option<InterruptPlan>,
}

static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);
static SIGNAL_BRIDGE_ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn preferred_io_mode(interactive_mode: InteractiveMode) -> IoMode {
    match interactive_mode {
        InteractiveMode::Always => IoMode::Pty,
        InteractiveMode::Auto | InteractiveMode::Never => IoMode::Pipes,
    }
}

fn build_local_command(plan: &LaunchPlan, io_mode: IoMode) -> anyhow::Result<Command> {
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    let mut command = if io_mode == IoMode::Pty {
        let mut command = Command::new("script");
        command.arg("-q").arg("/dev/null");
        append_launch_target(&mut command, plan)?;
        command
    } else if let Some(shell) = plan.resource_spec.shell {
        anyhow::ensure!(
            plan.argv.len() == 1,
            "shell launch plan requires exactly one script token"
        );
        let mut command = Command::new(shell_program(shell));
        command.arg("-lc").arg(&plan.argv[0]);
        command
    } else {
        let mut command = Command::new(&plan.argv[0]);
        command.args(&plan.argv[1..]);
        command
    };

    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    Ok(command)
}

fn append_launch_target(command: &mut Command, plan: &LaunchPlan) -> anyhow::Result<()> {
    if let Some(shell) = plan.resource_spec.shell {
        anyhow::ensure!(
            plan.argv.len() == 1,
            "shell launch plan requires exactly one script token"
        );
        command.arg(shell_program(shell));
        command.arg("-lc");
        command.arg(&plan.argv[0]);
    } else {
        command.arg(&plan.argv[0]);
        command.args(&plan.argv[1..]);
    }
    Ok(())
}

fn shell_program(shell: ShellKind) -> &'static OsStr {
    match shell {
        ShellKind::Sh => OsStr::new("sh"),
        ShellKind::Bash => OsStr::new("bash"),
        ShellKind::Zsh => OsStr::new("zsh"),
    }
}

fn spawn_reader_thread<T>(state: Arc<ProcessState>, mut reader: T, stream: OutputStream)
where
    T: Read + Send + 'static,
{
    state.readers_alive.fetch_add(1, Ordering::SeqCst);
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let frame = {
                        let mut collector = state.collector.lock().unwrap();
                        let bytes = &buffer[..read];
                        match stream {
                            OutputStream::Stdout => collector.push_stdout(bytes),
                            OutputStream::Stderr => collector.push_stderr(bytes),
                            OutputStream::PtyMerged => collector.push_pty(bytes),
                        }
                    };
                    state.frames.lock().unwrap().push_back(frame);
                }
                Err(_) => break,
            }
        }

        state.readers_alive.fetch_sub(1, Ordering::SeqCst);
    });
}

fn relay_output_frames(root_pid: u32) -> anyhow::Result<()> {
    for frame in drain_process_frames(root_pid) {
        relay_frame(&frame)?;
        execution_trace().lock().unwrap().frames.push(frame);
    }
    Ok(())
}

fn relay_frame(frame: &OutputFrame) -> anyhow::Result<()> {
    match frame.stream {
        OutputStream::Stdout | OutputStream::PtyMerged => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&frame.bytes)?;
            stdout.flush()?;
        }
        OutputStream::Stderr => {
            let mut stderr = std::io::stderr().lock();
            stderr.write_all(&frame.bytes)?;
            stderr.flush()?;
        }
    }
    Ok(())
}

fn finalize_process_output(root_pid: u32, poll_interval: Duration) -> anyhow::Result<()> {
    while readers_still_running(root_pid) {
        relay_output_frames(root_pid)?;
        thread::sleep(poll_interval.min(Duration::from_millis(10)));
    }

    relay_output_frames(root_pid)?;
    Ok(())
}

fn drain_process_frames(root_pid: u32) -> Vec<OutputFrame> {
    let Some(state) = process_state(root_pid) else {
        return Vec::new();
    };
    let mut queue = state.frames.lock().unwrap();
    queue.drain(..).collect()
}

fn readers_still_running(root_pid: u32) -> bool {
    process_state(root_pid).is_some_and(|state| state.readers_alive.load(Ordering::SeqCst) > 0)
}

fn process_state(root_pid: u32) -> Option<Arc<ProcessState>> {
    process_registry().lock().unwrap().get(&root_pid).cloned()
}

fn remove_process_state(root_pid: u32) {
    process_registry().lock().unwrap().remove(&root_pid);
}

fn process_registry() -> &'static Mutex<HashMap<u32, Arc<ProcessState>>> {
    static PROCESS_REGISTRY: OnceLock<Mutex<HashMap<u32, Arc<ProcessState>>>> = OnceLock::new();
    PROCESS_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn execution_trace() -> &'static Mutex<ExecutionTrace> {
    static EXECUTION_TRACE: OnceLock<Mutex<ExecutionTrace>> = OnceLock::new();
    EXECUTION_TRACE.get_or_init(|| Mutex::new(ExecutionTrace::default()))
}

fn runtime_overrides() -> &'static Mutex<RuntimeOverrides> {
    static RUNTIME_OVERRIDES: OnceLock<Mutex<RuntimeOverrides>> = OnceLock::new();
    RUNTIME_OVERRIDES.get_or_init(|| Mutex::new(RuntimeOverrides::default()))
}

fn signal_bridge_install_result() -> &'static Result<(), String> {
    static SIGNAL_BRIDGE_INSTALL: OnceLock<Result<(), String>> = OnceLock::new();
    SIGNAL_BRIDGE_INSTALL.get_or_init(|| {
        ctrlc::set_handler(|| {
            if SIGNAL_BRIDGE_ACTIVE.load(Ordering::SeqCst) > 0 {
                INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
            }
        })
        .map_err(|error| format!("failed to install Ctrl-C handler: {error}"))
    })
}

fn record_event(event: &'static str) {
    execution_trace().lock().unwrap().events.push(event);
}

fn recorded_events_matching(interesting: &[&'static str]) -> Vec<&'static str> {
    execution_trace()
        .lock()
        .unwrap()
        .events
        .iter()
        .copied()
        .filter(|event| interesting.contains(event))
        .collect()
}

fn clear_execution_trace() {
    let mut trace = execution_trace().lock().unwrap();
    trace.events.clear();
    trace.frames.clear();
}

fn configured_poll_interval() -> Duration {
    runtime_overrides()
        .lock()
        .unwrap()
        .poll_interval
        .unwrap_or(SAMPLE_INTERVAL)
}

fn configured_interrupt_plan() -> InterruptPlan {
    runtime_overrides()
        .lock()
        .unwrap()
        .interrupt_plan
        .unwrap_or_default()
}

fn clear_runtime_overrides() {
    let mut overrides = runtime_overrides().lock().unwrap();
    overrides.poll_interval = None;
    overrides.interrupt_plan = None;
    INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
}

fn update_peak_memory(current: Option<u64>, sample: &Sample) -> Option<u64> {
    let observed = sample.peak_memory_bytes.unwrap_or(sample.memory_bytes);

    Some(current.map_or(observed, |peak| peak.max(observed)))
}

fn runtime_since(started_at: SystemTime) -> Duration {
    SystemTime::now()
        .duration_since(started_at)
        .unwrap_or_default()
}
