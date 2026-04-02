# Scaler V1 Design

Date: 2026-04-02
Status: Draft for review

## Summary

Scaler is a Rust CLI that runs existing host commands under constrained resources while preserving a simple, cross-platform command-line interface.

V1 targets two host platforms:

- Ubuntu/Linux with `systemd` and unified `cgroup v2` for strong resource enforcement
- macOS for best-effort slowing, de-prioritization, and live monitoring

The core user goal is to let heavy commands run slowly and visibly instead of locking up the machine or failing unpredictably.

## Goals

- Provide a single CLI shape across Ubuntu and macOS
- Support direct external commands and inline shell snippets
- Prefer gradual slowdown over abrupt termination where the platform supports it
- Show a live resource dashboard while the command runs
- Preserve command output and interactive terminal behavior as much as possible
- Make platform capability differences explicit without forcing users to learn platform-specific flags

## Non-Goals

- Docker, containers, namespaces, or filesystem isolation
- Windows support in v1
- Language-specific runners such as `scaler js` or `scaler py`
- A task history database, replay system, or background scheduler
- Direct Linux `cgroup v2` file management in v1
- Perfectly identical enforcement semantics across all operating systems

## User Experience

### Primary commands

V1 exposes these user-facing commands:

- `scaler run`
- `scaler doctor`
- `scaler version`

`scaler version` behavior:

- Prints the Scaler version and target build information
- Exits `0` on success
- Does not require backend detection and should work even on unsupported hosts

V1 also supports a shorthand where `run` may be omitted:

```bash
scaler --cpu 1c --mem 1g -- npm install -g openclaw@latest
scaler run --cpu 1c --mem 1g -- npm install -g openclaw@latest
scaler run --cpu 0.5c --mem 768m --shell sh -- 'npm install && npm test'
```

### Resource flags

- `--cpu <value>`
  - User intent: limit execution to roughly the throughput of the specified logical CPU count
  - Example values: `1c`, `0.5c`, `2c`
- `--mem <value>`
  - User intent: give the command a memory budget
  - Example values: `512m`, `1g`, `1536m`
- `--interactive <auto|always|never>`
  - Default: `auto`
  - `auto` selects PTY mode when all standard streams are attached to a terminal
- `--shell <sh|bash|zsh>`
  - Only applies when running an inline shell snippet
- `--no-monitor`
  - Disable the live monitoring panel

### Resource value parsing

V1 accepts decimal resource values and normalizes them deterministically.

CPU grammar:

- Accepted form: `N c`
- Written without spaces on the command line, for example `1c`, `0.5c`, `.25c`
- Regex shape: `^([0-9]+(\.[0-9]+)?|\.[0-9]+)c$`
- The suffix `c` is required and case-insensitive
- Values must be positive, finite, and greater than zero
- Values are normalized to centi-cores (`0.01c`) using half-up rounding
- If rounding would produce less than `0.01c`, parsing fails as below minimum
- Values that overflow the internal fixed-point representation fail as usage errors

Memory grammar:

- Accepted form: `N <unit>`
- Written without spaces on the command line, for example `512m`, `1g`, `1.5g`
- Regex shape: `^([0-9]+(\.[0-9]+)?|\.[0-9]+)(b|k|m|g|t)$`
- Unit suffixes are case-insensitive
- Units use a base of `1024`, following common host-tool conventions
- Values are converted to whole bytes using half-up rounding
- The normalized value must be at least `1 MiB`
- Values that overflow the internal byte representation fail as usage errors

Validation rules:

- Negative values are rejected
- Zero values are rejected
- Unknown suffixes are rejected
- Empty strings are rejected
- Scientific notation such as `1e3` is rejected in v1
- Thousands separators and underscores are rejected in v1

Backend conversion rule:

- If a backend requires coarser units than the parsed value, it must round upward rather than downward so the effective limit is never stricter than the user's requested budget by accident

### CLI contract

V1 command parsing is explicit rather than heuristic.

Direct execution grammar:

```bash
scaler run [SCALER_FLAGS...] -- <program> [arg1 ...]
scaler [SCALER_FLAGS...] -- <program> [arg1 ...]
```

Shell snippet grammar:

```bash
scaler run [SCALER_FLAGS...] --shell <sh|bash|zsh> -- '<script>'
scaler [SCALER_FLAGS...] --shell <sh|bash|zsh> -- '<script>'
```

Rules:

- The `--` delimiter is required for `run`
- Without `--shell`, Scaler executes the target program directly and does not insert a shell
- With `--shell`, Scaler launches exactly `<shell> -lc <script>`
- When `--shell` is present, everything after `--` must resolve to one script string
- Plain `scaler` or `scaler run` with no target command is a usage error
- `--shell` without exactly one script string after `--` is a usage error
- V1 does not infer “this looks like shell” from free-form trailing tokens
- The shorthand form is only a syntactic rewrite to `run`; parsing rules stay identical
- A command whose executable name begins with `-` is only valid after the `--` delimiter

Working directory and environment rules:

- By default, the child inherits the caller's current working directory
- By default, the child inherits the caller's environment variables
- V1 does not add environment isolation or environment rewriting beyond backend-specific variables strictly required for launch

Interactive mode rules:

- `always`: require the PTY-style launch path for the active backend; fail before launch if not available
- `never`: require the non-PTY launch path
- `auto`: use PTY mode when stdin, stdout, and stderr are all attached to a terminal; otherwise use non-PTY mode

### Output semantics

The UI must make a clear distinction between:

- `enforced`: the platform can strongly enforce the requested limit
- `best-effort`: the platform can only approximate the requested limit
- `unavailable`: the platform or current host setup cannot provide the requested capability

On macOS, if the user passes `--cpu` or `--mem`, the command still runs by default, but the terminal must print a clear warning before launch and the monitor must display the resulting capability state as `best-effort` or `unavailable`.

State usage rules:

- `enforced` is used when the requested capability is expected to be applied strongly enough for v1's contract
- `best-effort` is used when Scaler can still run with a meaningful approximation
- `unavailable` is used when the backend or host cannot provide that capability at all
- `doctor` must report capability state per feature
- `run` must fail only when the backend itself is unavailable or when the user explicitly requires a launch mode that cannot be provided, such as `--interactive always`

## Scope of Execution

V1 supports two execution forms:

- Direct external commands, for example `npm install -g openclaw@latest`
- Inline shell execution, for example `--shell sh -- 'npm install && npm test'`

V1 must treat both forms as a single controlled process tree. This is the key abstraction for the rest of the system.

## Platform Semantics

### Ubuntu/Linux

Ubuntu is the primary strong-enforcement platform in v1.

Required environment assumptions:

- Linux host
- `systemd-run` available
- unified `cgroup v2` enabled
- user-level systemd manager usable for transient scopes

Linux enforcement semantics:

- CPU limiting uses `CPUQuota=`
- Memory slowdown uses `MemoryHigh=`
- Hard memory cap uses `MemoryMax=`
- Swap should be constrained in v1 where possible via `MemorySwapMax=0`
- Interactive commands use `systemd-run --scope --pty`

Linux `--mem` mapping:

- `--mem X` sets `MemoryMax=X`
- `--mem X` also sets `MemoryHigh` to `90%` of `X`
- If the command exceeds `MemoryHigh`, the unit is expected to slow down under aggressive reclaim pressure
- If the command exceeds `MemoryMax`, the unit may be terminated by OOM handling within the unit
- `MemorySwapMax=0` is requested in v1 so the budget behaves closer to the user-visible memory target

The design intent is:

- `MemoryHigh` is the main “slow it down and reclaim aggressively” control
- `MemoryMax` is the safety backstop
- CPU quota is used to reduce throughput rather than kill the process

If the Linux host does not satisfy the environment assumptions above, v1 does not silently degrade to weak controls. `scaler doctor` must report the problem, and `scaler run` should fail with a targeted explanation.

### macOS

macOS is a compatibility platform in v1, not a strong-enforcement platform.

macOS best-effort semantics:

- Use `taskpolicy` to lower scheduling and I/O priority
- Use `taskpolicy -m` where supported for a memory limit or policy
- Use `renice` where applicable to further lower scheduling priority
- Propagate the chosen execution policy to child processes through the launched process tree

macOS fallback policy:

- If `taskpolicy` is unavailable, the macOS backend is `unavailable` and `run` fails before launch
- If `renice` is unavailable, `run` continues and prints a warning; CPU remains `best-effort`
- If memory-related `taskpolicy` support is unavailable, `run` continues, marks memory capability `unavailable`, and prints a warning when `--mem` was requested
- If PTY launch cannot be provided and the user selected `--interactive always`, `run` fails before launch
- If PTY launch cannot be provided and the user selected `--interactive auto`, `run` falls back to non-PTY mode with a warning

On macOS:

- `--cpu` means “run more gently” rather than “hard cap to exactly N cores”
- `--mem` means “apply the closest supported memory policy and warn”
- The UI must surface the difference clearly before and during execution

## Recommended V1 Architecture

V1 uses a mixed layered design:

- A shared Rust CLI and monitor
- Platform-specific launch and sampling backends
- Linux starts with `systemd-run`
- macOS starts with `taskpolicy`
- Linux may later gain a direct `cgroup v2` backend without changing the CLI or monitor contracts

This design minimizes initial implementation risk while preserving a clean upgrade path.

## Internal Modules

### `cli`

Responsibilities:

- Parse arguments
- Normalize shorthand `scaler -- ...` into `run`
- Validate user input format
- Render user-facing errors and warnings

### `core`

Responsibilities:

- Convert user flags into a platform-neutral execution intent
- Define shared data structures
- Decide whether the launch should be marked `enforced`, `best-effort`, or `unavailable`
- Coordinate lifecycle between backend and UI

Suggested core structures:

```rust
struct ResourceSpec {
    cpu: Option<CpuLimit>,
    mem: Option<Bytes>,
    interactive: InteractiveMode,
    shell: Option<ShellKind>,
    monitor: bool,
}

struct LaunchPlan {
    argv: Vec<String>,
    resource_spec: ResourceSpec,
    platform: Platform,
}

struct CapabilityReport {
    platform: Platform,
    backend: BackendKind,
    backend_state: CapabilityLevel,
    cpu: CapabilityLevel,
    memory: CapabilityLevel,
    interactive: CapabilityLevel,
    warnings: Vec<String>,
}

struct RunOutcome {
    exit_status: ExitStatus,
    runtime: Duration,
    peak_memory: Option<u64>,
    samples: Vec<SummarySample>,
}
```

Suggested runtime data structures:

```rust
enum IoMode {
    Pty,
    Pipes,
}

struct RunningHandle {
    root_pid: u32,
    launch_time: SystemTime,
    io_mode: IoMode,
}

struct Sample {
    captured_at: SystemTime,
    cpu_percent: f32,
    memory_bytes: u64,
    peak_memory_bytes: Option<u64>,
    child_process_count: Option<u32>,
}

struct SummarySample {
    captured_at: SystemTime,
    cpu_percent: f32,
    memory_bytes: u64,
}

enum OutputStream {
    Stdout,
    Stderr,
    PtyMerged,
}

struct OutputFrame {
    sequence: u64,
    captured_at: SystemTime,
    stream: OutputStream,
    bytes: Vec<u8>,
}
```

I/O ownership contract:

- The backend owns process creation and returns a `RunningHandle` configured for either PTY mode or pipe mode
- `core` owns runtime I/O orchestration after launch
- In pipe mode, `core` reads stdout and stderr separately and forwards `OutputFrame` items to `ui`
- In PTY mode, `core` reads from the PTY master as a single merged interactive stream
- `ui` never owns OS process handles directly; it only renders samples, warnings, and output frames provided by `core`
- When monitor fallback is triggered, `core` keeps the same underlying I/O source and switches only the presentation path

Pipe-mode ordering rules:

- Within one stream, byte order is preserved exactly as read
- `core` assigns a monotonically increasing `sequence` to each emitted frame
- Across `stdout` and `stderr`, v1 does not guarantee a global causal ordering beyond observed arrival order inside Scaler
- The UI should render pipe-mode frames in ascending `sequence` order
- In PTY mode, stdout/stderr separation is not available and all output is treated as `PtyMerged`

Suggested capability levels:

```rust
enum CapabilityLevel {
    Enforced,
    BestEffort,
    Unavailable,
}
```

### `backend`

Responsibilities:

- Detect platform support
- Launch the process tree with platform-specific controls
- Sample runtime metrics
- Terminate the process tree on demand

Suggested trait:

```rust
trait Backend {
    fn detect() -> CapabilityReport;
    fn launch(plan: &LaunchPlan) -> Result<RunningHandle>;
    fn try_wait(handle: &mut RunningHandle) -> Result<Option<ExitStatus>>;
    fn sample(handle: &RunningHandle) -> Result<Sample>;
    fn terminate(handle: &RunningHandle, signal: Signal) -> Result<()>;
}
```

`core` may implement child-exit notification by polling `try_wait` inside the run loop; v1 does not require callback-based backend APIs.

Initial backend implementations:

- `linux_systemd_backend`
- `macos_taskpolicy_backend`

### `ui`

Responsibilities:

- Render the live dashboard
- Stream command stdout and stderr
- Show warnings, capability labels, and final summary
- Adapt presentation for interactive vs non-interactive commands

### Execution lifecycle contract

`core` owns the run loop and is the only layer that coordinates backend, monitor, and terminal teardown.

Lifecycle stages:

1. Detect capabilities
2. Validate requested launch mode against capabilities
3. Initialize monitor or decide to fall back to plain streaming mode
4. Launch the controlled process tree through the backend
5. Enter run loop: collect output, periodic samples, and user interrupts
6. Initiate shutdown on child exit, user interrupt, or fatal backend error
7. Restore terminal state
8. Print final summary and exit with the resolved status

Minimal event contract:

- backend provides launch success or failure
- backend provides child-exit notification
- backend provides periodic resource samples
- backend exposes enough process identity for signal forwarding
- ui consumes log frames and samples and may emit a user interrupt request
- core decides escalation, terminal restoration, final summary emission, and final exit code

Cadence rules:

- The default resource sampling cadence is `500ms`
- The monitor refresh cadence should not exceed the sampling cadence in v1
- Output forwarding stays event-driven and is not delayed to the sampling tick

Final summary ownership:

- `core` is the single owner of final-summary timing and emission
- `core` emits the final summary only after terminal restoration is complete
- `ui` may provide formatting helpers, but it does not independently decide when to print a summary

## Backend Details

### Linux backend

The Linux backend launches the target command under a transient systemd scope.

Expected behavior:

- Non-interactive commands can run with standard output relay and live sampling
- Interactive commands use `--pty`
- The backend records the scope name and uses it to sample runtime data

The backend should prefer stable, documented systemd options rather than shell tricks.

Lifecycle expectations:

- The backend must retain enough identity to signal the full launched process tree
- A normal child exit returns the same exit code from `scaler`
- A child terminated by signal should be surfaced using standard CLI conventions where supported, typically `128 + signal`

### macOS backend

The macOS backend launches through `taskpolicy` and applies additional priority lowering where useful.

Expected behavior:

- It should preserve a single process tree abstraction
- It should collect metrics by aggregating the launched process and descendants
- It should never claim strong CPU or memory enforcement

Lifecycle expectations:

- The backend must identify and manage the launched process group or equivalent descendant set
- A normal child exit returns the same exit code from `scaler`
- Forced termination must target the controlled process tree, not only the immediate launcher shim

## Monitoring Panel

The monitor is on by default.

Recommended layout:

- Top area: command, backend, enforcement label, runtime, live status
- Middle area: limits and current metrics
- Bottom area: stdout and stderr log stream

Required live fields:

- Command being run
- Backend in use
- Capability state per requested feature
- Elapsed runtime
- Current CPU usage
- Current memory usage
- Peak memory usage
- Child process count
- Exit state or running state

Linux-specific metrics when available:

- CPU throttle count
- CPU throttled time
- Current cgroup memory
- Memory events
- Pressure indicators when cheap to sample

macOS-specific runtime indicators when available:

- Effective taskpolicy mode
- Nice value or priority hints
- Aggregated process tree RSS

### Interactive behavior

V1 should favor execution stability over a perfect full-screen dashboard.

The monitor should support a compact mode for interactive commands so that:

- Users can still type into the child process
- Command output remains visible
- Resource status remains visible in a reduced form

If a full-screen TUI conflicts with the child process terminal experience, the monitor should degrade to a simpler streaming mode instead of breaking interaction.

Monitor failure policy:

- Failure to initialize or maintain the monitor must not discard a command that can otherwise run
- If monitor startup fails before launch, Scaler falls back to plain output streaming and prints a warning
- If the monitor fails after launch, Scaler tears down the UI, restores the terminal, and continues relaying child output if possible
- Only a launch/backend failure prevents command execution

## `doctor` Command

`scaler doctor` is a first-class command in v1.

It should report:

- Detected operating system
- Selected backend
- Whether the backend itself is available
- Whether CPU control is enforced, best-effort, or unavailable
- Whether memory control is enforced, best-effort, or unavailable
- Whether interactive mode is supported
- Host prerequisites and missing dependencies

Linux doctor checks:

- `systemd-run` exists
- `cgroup v2` is active
- the user service manager is reachable enough to create transient scopes

macOS doctor checks:

- `taskpolicy` exists
- `renice` exists
- the platform version supports the required invocation path

Doctor output must classify each reported feature as `enforced`, `best-effort`, or `unavailable`.

## Error Handling

V1 must fail clearly in the following cases:

- plain `scaler` or `scaler run` with no target
- malformed CPU or memory input
- missing command after `--`
- unsupported shell selection
- `--shell` without exactly one script string
- selected backend unavailable on the current host
- child process launch failure

Failure messaging rules:

- say what failed
- say whether the command ran or did not run
- say whether limits were enforced, best-effort, or unavailable
- suggest `scaler doctor` where relevant

Signal and termination rules:

- `Ctrl-C` from the user is forwarded to the controlled process tree
- The first interrupt sends `SIGINT` immediately
- If the controlled process tree is still alive after `2s`, Scaler escalates to `SIGTERM`
- If the controlled process tree is still alive `3s` after `SIGTERM`, Scaler escalates to `SIGKILL`
- Scaler must restore terminal state before exiting, whether the child exits normally, the user interrupts, or the monitor fails

## Testing Strategy

### Unit tests

- argument parsing
- CPU and memory value parsing
- CPU centi-core normalization and below-minimum rejection
- memory byte normalization, minimum bound, and overflow rejection
- shorthand `run` normalization
- capability labeling
- launch plan construction
- lifecycle state transitions for launch, running, and teardown
- unsupported capability classification
- output-frame sequencing and per-stream ordering behavior

### Integration tests

Linux-focused:

- a CPU-heavy command is visibly throttled under requested CPU quota
- a memory-heavy command slows or gets constrained by `MemoryHigh`
- a command that exceeds hard memory cap is terminated cleanly and reported accurately
- interactive launch path works with `--pty`
- signal forwarding reaches the full transient scope
- shorthand parsing works for commands whose executable begins with `-` after the delimiter
- missing user-manager or unsupported host setup fails clearly without launching the child
- `doctor` reports unavailable capabilities correctly
- monitor teardown restores terminal state after interrupt
- final summary is emitted exactly once after terminal restoration

macOS-focused:

- launch path works through `taskpolicy`
- warnings appear for best-effort CPU and memory limits
- process tree metrics aggregate correctly
- interactive command path remains usable
- signal forwarding reaches the launched process tree
- monitor teardown restores terminal state
- unsupported shell selection and invalid script forms fail before launch
- `doctor` reports best-effort and unavailable capabilities correctly
- warning paths are exercised when `--mem` is requested but memory control is unavailable
- final summary is emitted exactly once after terminal restoration

### Manual acceptance checks

Representative commands:

```bash
scaler run --cpu 1c --mem 1g -- npm install -g openclaw@latest
scaler run --cpu 0.5c --mem 512m --shell sh -- 'yes > /dev/null'
scaler run --shell sh -- 'read -p "name? " x; echo "$x"'
```

## V1 Boundaries

Included in v1:

- cross-platform CLI shape
- direct commands and inline shell snippets
- live monitor
- interactive support with stability-first degradation
- Ubuntu strong enforcement
- macOS best-effort mode
- doctor command

Explicitly deferred:

- direct Linux `cgroup v2` backend
- Windows
- language-specific code runners
- persistent job history
- multi-job scheduling

## Open Questions Resolved for V1

- Cross-platform CLI remains unified
- macOS runs with warning instead of refusing execution
- Interactive commands are supported, but direct one-line commands are the implementation priority
- `run` remains the canonical subcommand, with a shorthand that omits it

## Implementation Notes for the Next Phase

The next phase should produce an implementation plan that starts with:

1. CLI argument model
2. capability detection and `doctor`
3. backend abstraction
4. Linux `systemd-run` backend
5. macOS `taskpolicy` backend
6. monitor integration
7. interactive-mode degradation rules

This sequencing keeps the user-facing contract stable while reducing platform-specific risk early.
