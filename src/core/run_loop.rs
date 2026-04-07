use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::{IsTerminal, Read},
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
    ui::{self, MonitorSnapshot, Renderer as _, UiContext, plain::PlainRenderer, tui::TuiRenderer},
};

pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const CONTROL_INTERVAL_CAP: Duration = Duration::from_millis(10);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalState {
    stdin: bool,
    stdout: bool,
    stderr: bool,
}

impl TerminalState {
    fn all_terminals(self) -> bool {
        self.stdin && self.stdout && self.stderr
    }
}

#[derive(Debug, Clone)]
struct SelectedExecution {
    plan: LaunchPlan,
    io_mode: IoMode,
    use_tui: bool,
    compact: bool,
}

#[derive(Debug)]
enum ActiveUi {
    Plain(PlainRenderer),
    Tui(Box<TuiRenderer>),
}

impl ActiveUi {
    fn render_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()> {
        match self {
            Self::Plain(renderer) => renderer.render_frame(frame),
            Self::Tui(renderer) => renderer.render_frame(frame),
        }
    }

    fn render_snapshot(&mut self, snapshot: &MonitorSnapshot) -> anyhow::Result<()> {
        match self {
            Self::Plain(renderer) => renderer.render_snapshot(snapshot),
            Self::Tui(renderer) => renderer.render_snapshot(snapshot),
        }
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Plain(renderer) => renderer.finish(),
            Self::Tui(renderer) => renderer.finish(),
        }
    }

    fn is_tui(&self) -> bool {
        matches!(self, Self::Tui(_))
    }
}

#[derive(Debug)]
struct UiSession {
    renderer: ActiveUi,
    base_context: UiContext,
    restored: bool,
}

impl UiSession {
    fn start(
        selection: &SelectedExecution,
        capabilities: crate::core::CapabilityReport,
    ) -> anyhow::Result<Self> {
        let base_context = UiContext {
            command: display_command(&selection.plan),
            capabilities: capabilities.clone(),
            compact: selection.compact,
            warnings: capabilities.warnings.clone(),
        };

        if selection.use_tui {
            let options = configured_tui_options();
            match TuiRenderer::start(base_context.clone(), options) {
                Ok(renderer) => {
                    record_event("tui_renderer_active");
                    if selection.compact {
                        record_event("compact_interactive_mode");
                    }
                    return Ok(Self {
                        renderer: ActiveUi::Tui(Box::new(renderer)),
                        base_context,
                        restored: false,
                    });
                }
                Err(error) => {
                    record_event("monitor_unavailable");
                    let context = context_with_extra_warning(
                        &base_context,
                        format!("monitor disabled: {error}"),
                    );
                    let renderer = PlainRenderer::new(&context)?;
                    record_event("plain_renderer_active");
                    return Ok(Self {
                        renderer: ActiveUi::Plain(renderer),
                        base_context,
                        restored: false,
                    });
                }
            }
        }

        if selection.plan.resource_spec.monitor {
            record_event("monitor_unavailable");
        }
        let renderer = PlainRenderer::new(&base_context)?;
        record_event("plain_renderer_active");
        Ok(Self {
            renderer: ActiveUi::Plain(renderer),
            base_context,
            restored: false,
        })
    }

    fn render_frame(
        &mut self,
        frame: &OutputFrame,
        rendered_frames: &[OutputFrame],
    ) -> anyhow::Result<()> {
        if let Err(error) = self.renderer.render_frame(frame) {
            return self.handle_runtime_failure(error, rendered_frames);
        }
        Ok(())
    }

    fn render_snapshot(
        &mut self,
        snapshot: &MonitorSnapshot,
        rendered_frames: &[OutputFrame],
    ) -> anyhow::Result<()> {
        if let Err(error) = self.renderer.render_snapshot(snapshot) {
            return self.handle_runtime_failure(error, rendered_frames);
        }
        Ok(())
    }

    fn restore_once(&mut self) -> anyhow::Result<()> {
        if self.restored {
            return Ok(());
        }

        self.renderer.finish()?;
        record_event("restore_terminal");
        self.restored = true;
        Ok(())
    }

    fn handle_runtime_failure(
        &mut self,
        error: anyhow::Error,
        rendered_frames: &[OutputFrame],
    ) -> anyhow::Result<()> {
        if !self.renderer.is_tui() {
            return Err(error);
        }

        record_event("monitor_failed");
        self.restore_once()?;
        let context = context_with_extra_warning(
            &self.base_context,
            format!("monitor disabled after launch: {error}"),
        );
        let mut renderer = PlainRenderer::new(&context)?;
        renderer.replay(rendered_frames)?;
        self.renderer = ActiveUi::Plain(renderer);
        record_event("plain_renderer_continues");
        Ok(())
    }
}

pub fn execute(plan: LaunchPlan, backend: &dyn Backend) -> anyhow::Result<RunOutcome> {
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    clear_execution_trace();
    let capabilities = backend.detect();
    let selection = select_execution(&plan)?;
    record_event("launch");
    let mut handle = backend.launch(&selection.plan)?;
    record_event("launch_complete");
    let mut ui = UiSession::start(&selection, capabilities)?;
    record_event("interactive_mode_selected");
    match selection.io_mode {
        IoMode::Pipes => record_event("pipe_streams"),
        IoMode::Pty => record_event("pty_merged_stream"),
    }

    let started_at = handle.launch_time;
    let sample_interval = configured_sample_interval();
    let control_interval = sample_interval.min(CONTROL_INTERVAL_CAP);
    let interrupt_plan = configured_interrupt_plan();
    let mut peak_memory = None;
    let mut samples = Vec::new();
    let mut interrupt_started_at = None;
    let mut sent_sigterm = false;
    let mut sent_sigkill = false;
    let mut next_sample_due = Instant::now();
    let mut rendered_frames = Vec::new();

    loop {
        drain_output_frames(handle.root_pid, &mut ui, &mut rendered_frames)?;

        if let Some(exit_status) = backend.try_wait(&mut handle)? {
            finalize_process_output(handle.root_pid, control_interval)?;
            drain_output_frames(handle.root_pid, &mut ui, &mut rendered_frames)?;
            remove_process_state(handle.root_pid);
            ui.restore_once()?;
            let outcome = RunOutcome {
                exit_status,
                elapsed: runtime_since(started_at),
                peak_memory,
                mem_limit_bytes: plan.resource_spec.mem.map(|limit| limit.bytes()),
                samples,
            };
            // Summary goes to stderr so user pipelines like
            //   scaler run -- jq < x.json > out.json
            // keep `out.json` clean of scaler's metadata. The leading blank
            // line gives a clear visual break from the child output; the
            // top + bottom box-drawing rule lives inside summary::render.
            eprintln!();
            eprintln!("{}", crate::core::summary::render(&outcome));
            record_event("render_summary");
            clear_runtime_overrides();

            return Ok(outcome);
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

        if Instant::now() >= next_sample_due {
            if let Ok(sample) = backend.sample(&handle) {
                peak_memory = update_peak_memory(peak_memory, &sample);
                samples.push(SummarySample {
                    captured_at: sample.captured_at,
                    cpu_percent: sample.cpu_percent,
                    memory_bytes: sample.memory_bytes,
                });
                ui.render_snapshot(
                    &MonitorSnapshot {
                        elapsed: runtime_since(started_at),
                        cpu_percent: Some(sample.cpu_percent),
                        memory_bytes: Some(sample.memory_bytes),
                        peak_memory_bytes: peak_memory,
                        child_count: sample.child_process_count,
                        run_status: "running".to_string(),
                    },
                    &rendered_frames,
                )?;
            }
            next_sample_due = Instant::now() + sample_interval;
        }

        thread::sleep(control_interval);
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
        let command = build_local_command(plan, io_mode)?;
        spawn_with_bookkeeping(command, io_mode)
    }

    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        try_wait_via_registry(handle.root_pid)
    }

    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample> {
        crate::core::sampling::sample_process_tree(handle.root_pid)
    }

    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        terminate_process_group(handle.root_pid, signal)
    }
}

#[doc(hidden)]
pub fn record_summary_timeline_for_test() -> Vec<&'static str> {
    recorded_events_matching(&["launch", "restore_terminal", "render_summary"])
}

#[doc(hidden)]
pub fn record_monitor_fallback_for_test() -> Vec<&'static str> {
    recorded_events_matching(&["monitor_unavailable", "plain_renderer_active"])
}

#[doc(hidden)]
pub fn record_interactive_mode_for_test() -> Vec<&'static str> {
    recorded_events_matching(&[
        "interactive_mode_selected",
        "pipe_streams",
        "pty_merged_stream",
    ])
}

#[doc(hidden)]
pub fn record_post_launch_monitor_failure_for_test() -> Vec<&'static str> {
    recorded_events_matching(&["monitor_failed", "plain_renderer_continues"])
}

#[doc(hidden)]
pub fn record_ui_mode_for_test() -> Vec<&'static str> {
    recorded_events_matching(&[
        "tui_renderer_active",
        "plain_renderer_active",
        "compact_interactive_mode",
    ])
}

#[doc(hidden)]
pub fn take_output_frames_for_test() -> Vec<OutputFrame> {
    let mut trace = execution_trace().lock().unwrap();
    std::mem::take(&mut trace.frames)
}

#[doc(hidden)]
pub fn reset_test_state() {
    clear_execution_trace();
    clear_runtime_overrides();
}

#[doc(hidden)]
pub fn request_interrupt_for_test() {
    INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
}

#[doc(hidden)]
pub fn set_test_poll_interval_for_next_run(duration: Duration) {
    runtime_overrides().lock().unwrap().poll_interval = Some(duration);
}

#[doc(hidden)]
pub fn set_test_interrupt_plan_for_next_run(sigterm_after: Duration, sigkill_after: Duration) {
    runtime_overrides().lock().unwrap().interrupt_plan = Some(InterruptPlan {
        sigterm_after,
        sigkill_after,
    });
}

#[doc(hidden)]
pub fn set_test_terminal_state_for_next_run(stdin: bool, stdout: bool, stderr: bool) {
    runtime_overrides().lock().unwrap().terminal_state = Some(TerminalState {
        stdin,
        stdout,
        stderr,
    });
}

#[doc(hidden)]
pub fn set_test_monitor_start_failure_for_next_run(message: &str) {
    runtime_overrides().lock().unwrap().monitor_start_failure = Some(message.to_string());
}

#[doc(hidden)]
pub fn set_test_monitor_fail_after_launch_for_next_run(draws_before_failure: usize) {
    runtime_overrides().lock().unwrap().monitor_fail_after_draws = Some(draws_before_failure);
}

#[doc(hidden)]
pub fn plain_fallback_command_preview_for_test(plan: &LaunchPlan) -> anyhow::Result<Vec<OsString>> {
    let io_mode = preferred_io_mode(plan.resource_spec.interactive);
    let command = build_local_command(plan, io_mode)?;
    let mut preview = Vec::new();
    preview.push(command.get_program().to_os_string());
    preview.extend(command.get_args().map(|arg| arg.to_os_string()));
    Ok(preview)
}

#[derive(Debug)]
struct ProcessState {
    child: Mutex<Child>,
    collector: Mutex<OutputCollector>,
    pending_frames: Mutex<Vec<OutputFrame>>,
    readers_alive: AtomicUsize,
}

impl ProcessState {
    fn new(child: Child) -> Self {
        Self {
            child: Mutex::new(child),
            collector: Mutex::new(OutputCollector::default()),
            pending_frames: Mutex::new(Vec::new()),
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
    terminal_state: Option<TerminalState>,
    monitor_start_failure: Option<String>,
    monitor_fail_after_draws: Option<usize>,
}

static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);
static SIGNAL_BRIDGE_ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn preferred_io_mode(interactive_mode: InteractiveMode) -> IoMode {
    match interactive_mode {
        InteractiveMode::Always => IoMode::Pty,
        InteractiveMode::Auto | InteractiveMode::Never => IoMode::Pipes,
    }
}

fn select_execution(plan: &LaunchPlan) -> anyhow::Result<SelectedExecution> {
    let terminal_state = detected_terminal_state();
    let all_terminals = terminal_state.all_terminals();
    let pty_available = pty_path_available(plan.platform);

    let io_mode = match plan.resource_spec.interactive {
        InteractiveMode::Always => {
            anyhow::ensure!(
                pty_available,
                "interactive=always requires PTY support before launch"
            );
            IoMode::Pty
        }
        InteractiveMode::Never => IoMode::Pipes,
        InteractiveMode::Auto if all_terminals && pty_available => IoMode::Pty,
        InteractiveMode::Auto => IoMode::Pipes,
    };

    let mut selected_plan = plan.clone();
    selected_plan.resource_spec.interactive = if io_mode == IoMode::Pty {
        InteractiveMode::Always
    } else {
        InteractiveMode::Never
    };

    Ok(SelectedExecution {
        plan: selected_plan,
        io_mode,
        use_tui: plan.resource_spec.monitor && all_terminals,
        compact: io_mode == IoMode::Pty,
    })
}

fn detected_terminal_state() -> TerminalState {
    configured_terminal_state().unwrap_or(TerminalState {
        stdin: std::io::stdin().is_terminal(),
        stdout: std::io::stdout().is_terminal(),
        stderr: std::io::stderr().is_terminal(),
    })
}

fn pty_path_available(platform: crate::core::Platform) -> bool {
    matches!(
        platform,
        crate::core::Platform::Linux
            | crate::core::Platform::Macos
            | crate::core::Platform::Unsupported
    ) && command_available("script")
}

fn command_available(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).exists())
    })
}

fn configured_tui_options() -> ui::tui::InitOptions {
    let overrides = runtime_overrides().lock().unwrap();

    ui::tui::InitOptions {
        headless: overrides.terminal_state.is_some(),
        fail_on_start: overrides.monitor_start_failure.clone(),
        fail_after_draws: overrides.monitor_fail_after_draws,
    }
}

fn configured_terminal_state() -> Option<TerminalState> {
    runtime_overrides().lock().unwrap().terminal_state
}

fn context_with_extra_warning(base: &UiContext, warning: String) -> UiContext {
    let mut context = base.clone();
    context.warnings.push(warning);
    context
}

fn display_command(plan: &LaunchPlan) -> String {
    render_launch_target_command(plan).unwrap_or_else(|_| {
        plan.argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

fn build_local_command(plan: &LaunchPlan, io_mode: IoMode) -> anyhow::Result<Command> {
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    let mut command = if io_mode == IoMode::Pty {
        build_pty_command(plan)?
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        command.process_group(0);
    }
    Ok(command)
}

fn build_pty_command(plan: &LaunchPlan) -> anyhow::Result<Command> {
    let mut command = Command::new("script");

    match plan.platform {
        crate::core::Platform::Linux => {
            command.arg("-q");
            command.arg("-e");
            command.arg("-c");
            command.arg(render_launch_target_command(plan)?);
            command.arg("/dev/null");
        }
        crate::core::Platform::Macos | crate::core::Platform::Unsupported => {
            command.arg("-q");
            command.arg("/dev/null");
            append_launch_target(&mut command, plan)?;
        }
    }

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

fn render_launch_target_command(plan: &LaunchPlan) -> anyhow::Result<String> {
    if let Some(shell) = plan.resource_spec.shell {
        anyhow::ensure!(
            plan.argv.len() == 1,
            "shell launch plan requires exactly one script token"
        );

        return Ok(format!(
            "{} -lc {}",
            shell_program(shell).to_string_lossy(),
            shell_escape(plan.argv[0].to_string_lossy().as_ref())
        ));
    }

    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");
    Ok(plan
        .argv
        .iter()
        .map(|value| shell_escape(value.to_string_lossy().as_ref()))
        .collect::<Vec<_>>()
        .join(" "))
}

fn shell_program(shell: ShellKind) -> &'static OsStr {
    match shell {
        ShellKind::Sh => OsStr::new("sh"),
        ShellKind::Bash => OsStr::new("bash"),
        ShellKind::Zsh => OsStr::new("zsh"),
    }
}

fn shell_escape(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "/._-".contains(character))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
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
                    execution_trace().lock().unwrap().frames.push(frame.clone());
                    state.pending_frames.lock().unwrap().push(frame);
                }
                Err(_) => break,
            }
        }

        state.readers_alive.fetch_sub(1, Ordering::SeqCst);
    });
}

fn drain_output_frames(
    root_pid: u32,
    ui: &mut UiSession,
    rendered_frames: &mut Vec<OutputFrame>,
) -> anyhow::Result<()> {
    let Some(state) = process_state(root_pid) else {
        return Ok(());
    };

    let frames = {
        let mut pending = state.pending_frames.lock().unwrap();
        std::mem::take(&mut *pending)
    };

    for frame in frames {
        ui.render_frame(&frame, rendered_frames)?;
        rendered_frames.push(frame);
    }

    Ok(())
}

fn finalize_process_output(root_pid: u32, poll_interval: Duration) -> anyhow::Result<()> {
    while readers_still_running(root_pid) {
        thread::sleep(poll_interval.min(Duration::from_millis(10)));
    }
    Ok(())
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

fn configured_sample_interval() -> Duration {
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
    overrides.terminal_state = None;
    overrides.monitor_start_failure = None;
    overrides.monitor_fail_after_draws = None;
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

/// Polls the registered process for the given pid and returns its exit
/// status if available. Used by all platform backend `try_wait` impls.
pub fn try_wait_via_registry(root_pid: u32) -> anyhow::Result<Option<std::process::ExitStatus>> {
    let state = process_state(root_pid)
        .with_context(|| format!("missing process state for pid {root_pid}"))?;
    Ok(state.child.lock().unwrap().try_wait()?)
}

/// Sends `signal` to the process group rooted at `root_pid`. Used by all
/// platform backend `terminate` impls.
pub fn terminate_process_group(root_pid: u32, signal: Signal) -> anyhow::Result<()> {
    let signal_flag = match signal {
        Signal::Interrupt => "-INT",
        Signal::Terminate => "-TERM",
        Signal::Kill => "-KILL",
    };
    let process_group = format!("-{root_pid}");
    let status = Command::new("kill")
        .arg(signal_flag)
        .arg("--")
        .arg(&process_group)
        .status()
        .with_context(|| format!("failed to send {signal_flag} to process group {root_pid}"))?;
    anyhow::ensure!(
        status.success(),
        "kill command exited unsuccessfully for process group {root_pid}"
    );
    Ok(())
}

/// Build a `Command` from a flat argv (`argv[0]` is the program). Wires
/// stdio for pipe vs PTY mode and puts the child in its own process group
/// on unix. This is the only place that knows how to materialize a child
/// process for ANY backend that already produced a complete argv.
pub(crate) fn command_from_argv(
    argv: &[std::ffi::OsString],
    io_mode: IoMode,
) -> anyhow::Result<Command> {
    anyhow::ensure!(!argv.is_empty(), "command argv must not be empty");

    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    let _ = io_mode; // io_mode reserved for future PTY-specific stdio decisions
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Ok(command)
}

/// Spawn a `Command` and wire it into the run loop's process registry +
/// reader threads. All `Backend::launch` impls funnel through this so the
/// run loop owns the spawn machinery in exactly one place.
pub(crate) fn spawn_with_bookkeeping(
    mut command: Command,
    io_mode: IoMode,
) -> anyhow::Result<RunningHandle> {
    let program = command.get_program().to_os_string();
    let launched_at = SystemTime::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn command: {program:?}"))?;

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
