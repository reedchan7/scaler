# Platform Backend Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unconditional `PlainFallbackBackend` wiring in `lib.rs` with real Linux (`systemd-run`) and macOS (`taskpolicy`) backend implementations so that `--cpu` / `--mem` flags are actually enforced; report the runtime-effective backend in `doctor`; warn at run time when limits are silently dropped.

**Architecture:** Three `Backend` impls share spawn/registry/sampling helpers extracted from `run_loop.rs`. `select_backend()` in `src/backend/mod.rs` picks the right one based on `detect_host_capabilities()` plus an `SCALER_FORCE_BACKEND` test escape hatch. `PlainFallbackBackend` remains as a tree-aggregating fallback that prints a warning to stderr when the user requested resource limits but the platform backend is unavailable.

**Tech Stack:** Rust 2024, `clap`, `anyhow`, `thiserror`, `assert_cmd`, `tempfile`, `predicates`. No new dependencies.

**Out of scope (deferred):** aarch64 release artifacts (TODO note added in `release.yml` only); cgroup-file-based sampling; `systemctl --user stop` based termination.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/core/sampling.rs` | CREATE | Pure parser for `ps -e` table + BFS aggregation over root pid's descendants. Side-effecting `sample_process_tree(root_pid)` that calls `ps` and aggregates. |
| `src/core/mod.rs` | MODIFY | `pub mod sampling;` |
| `src/core/types.rs` | MODIFY | Add `BackendKind::PlainFallback` variant + matching `as_str` arm. |
| `src/core/run_loop.rs` | MODIFY | Extract `spawn_with_bookkeeping(command, io_mode)` and `command_from_argv(argv, io_mode)` as `pub(crate)` free helpers. `PlainFallbackBackend::launch` delegates to them. `PlainFallbackBackend::sample` delegates to `core::sampling::sample_process_tree`. |
| `src/backend/mod.rs` | MODIFY | Add `pub fn select_backend() -> Box<dyn Backend>` and `pub fn effective_backend_kind() -> BackendKind`, both honoring `SCALER_FORCE_BACKEND`. Re-export the new backend structs. |
| `src/backend/linux_systemd.rs` | MODIFY | Add `pub struct LinuxSystemdBackend; impl Backend for LinuxSystemdBackend`. Add `pub fn linux_systemd_command_preview_for_test(plan)` test seam. |
| `src/backend/macos_taskpolicy.rs` | MODIFY | Add `pub struct MacosTaskpolicyBackend; impl Backend for MacosTaskpolicyBackend`. Change `build_taskpolicy_argv` to take `include_memory_flag: bool` so the `-m` flag is gated on probed memory support. Add `pub fn macos_taskpolicy_command_preview_for_test(plan, include_memory_flag)` test seam. |
| `src/cli/mod.rs` | MODIFY | `render_doctor_output` gains a second argument `effective: BackendKind` and emits one extra line `effective_backend: <kind>` after the `interactive: ...` line. |
| `src/lib.rs` | MODIFY | `Command::Doctor` calls `effective_backend_kind()` and passes both report + kind into `render_doctor_output`. `Command::Run` uses `select_backend()` instead of the hardcoded `PlainFallbackBackend`; if the effective kind is `PlainFallback` AND the user requested `--cpu` or `--mem`, prints a stderr warning before launch. |
| `tests/backend_linux.rs` | MODIFY | Add Linux unit tests for `LinuxSystemdBackend.detect()` and the new command-preview test seam. Add a PATH-shim integration test that spawns the scaler binary with `SCALER_FORCE_BACKEND=linux_systemd`, a fake `systemd-run` shim on `PATH`, and asserts the recorded argv. Update `build_taskpolicy_argv` related expectations only if needed. |
| `tests/backend_macos.rs` | MODIFY | Update `build_taskpolicy_argv` callsites to pass `include_memory_flag: true`. Add macOS unit tests for `MacosTaskpolicyBackend.detect()` and command-preview seam. Add PATH-shim integration test for macOS. |
| `tests/doctor_cli.rs` | MODIFY | Update slice indices for the new `effective_backend:` line; update the renderer expected-string test; add a renderer test that asserts a fallback case prints `effective_backend: plain_fallback`. |
| `README.md` | MODIFY | Add an "Install from release" section with curl + tar + chmod + mv example for `x86_64-unknown-linux-gnu`. Add one sentence noting the `systemctl --user` requirement for Linux enforcement. |
| `.github/workflows/release.yml` | MODIFY | Add a TODO comment block above the matrix listing aarch64 targets to add later. |
| `Cargo.toml` | MODIFY | Bump `version = "0.1.0"` → `version = "0.2.0"` (semantic bump for the meaningful behavior change). |
| `CHANGELOG.md` | CREATE | Single entry for `0.2.0` describing the backend wiring fix. |

---

## Task 1: Process tree sampling helper

**Files:**
- Create: `src/core/sampling.rs`
- Modify: `src/core/mod.rs:1-7`
- Test: inline `#[cfg(test)]` mod inside `src/core/sampling.rs`

- [ ] **Step 1: Write the failing parser tests**

Create `src/core/sampling.rs`:

```rust
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::core::Sample;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsRow {
    pub pid: u32,
    pub ppid: u32,
    pub rss_kib: u64,
    pub cpu_percent: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AggregatedMetrics {
    pub rss_bytes: u64,
    pub cpu_percent: f32,
    pub process_count: u32,
}

pub fn parse_ps_table(input: &str) -> Vec<PsRow> {
    input
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let ppid = fields.next()?.parse::<u32>().ok()?;
            let rss_kib = fields.next()?.parse::<u64>().ok()?;
            let cpu_percent = fields.next()?.parse::<f32>().ok()?;
            Some(PsRow {
                pid,
                ppid,
                rss_kib,
                cpu_percent,
            })
        })
        .collect()
}

pub fn aggregate_descendants(rows: &[PsRow], root_pid: u32) -> AggregatedMetrics {
    let mut included = std::collections::HashSet::new();
    let mut frontier = vec![root_pid];

    while let Some(pid) = frontier.pop() {
        if !included.insert(pid) {
            continue;
        }
        for row in rows {
            if row.ppid == pid && !included.contains(&row.pid) {
                frontier.push(row.pid);
            }
        }
    }

    let mut metrics = AggregatedMetrics::default();
    for row in rows {
        if included.contains(&row.pid) {
            metrics.rss_bytes = metrics.rss_bytes.saturating_add(row.rss_kib * 1024);
            metrics.cpu_percent += row.cpu_percent;
            metrics.process_count += 1;
        }
    }
    metrics
}

pub fn sample_process_tree(root_pid: u32) -> Result<Sample> {
    let output = Command::new("ps")
        .args(["-e", "-o", "pid=,ppid=,rss=,%cpu="])
        .output()
        .with_context(|| format!("failed to invoke ps for pid {root_pid}"))?;
    anyhow::ensure!(
        output.status.success(),
        "ps exited with non-success status while sampling pid {root_pid}"
    );

    let table = String::from_utf8_lossy(&output.stdout);
    let rows = parse_ps_table(&table);
    let metrics = aggregate_descendants(&rows, root_pid);

    Ok(Sample {
        captured_at: SystemTime::now(),
        cpu_percent: metrics.cpu_percent,
        memory_bytes: metrics.rss_bytes,
        peak_memory_bytes: Some(metrics.rss_bytes),
        child_process_count: Some(metrics.process_count),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_ps_lines() {
        let input = "  100   1  4096  3.5\n  101 100  2048  1.0\n";
        let rows = parse_ps_table(input);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pid, 100);
        assert_eq!(rows[0].ppid, 1);
        assert_eq!(rows[0].rss_kib, 4096);
        assert!((rows[0].cpu_percent - 3.5).abs() < 1e-6);
        assert_eq!(rows[1].pid, 101);
        assert_eq!(rows[1].ppid, 100);
    }

    #[test]
    fn ignores_unparseable_rows() {
        let input = "header line\n100 1 4096 3.5\n  garbage\n";
        let rows = parse_ps_table(input);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pid, 100);
    }

    #[test]
    fn aggregates_only_descendants_of_root() {
        let rows = vec![
            PsRow { pid: 1, ppid: 0, rss_kib: 100, cpu_percent: 0.0 },
            PsRow { pid: 100, ppid: 1, rss_kib: 4096, cpu_percent: 5.0 },
            PsRow { pid: 200, ppid: 100, rss_kib: 2048, cpu_percent: 2.0 },
            PsRow { pid: 201, ppid: 100, rss_kib: 1024, cpu_percent: 1.0 },
            PsRow { pid: 300, ppid: 200, rss_kib: 512, cpu_percent: 0.5 },
            PsRow { pid: 999, ppid: 1, rss_kib: 10000, cpu_percent: 50.0 },
        ];

        let metrics = aggregate_descendants(&rows, 100);

        assert_eq!(metrics.process_count, 4);
        assert_eq!(
            metrics.rss_bytes,
            (4096 + 2048 + 1024 + 512) * 1024
        );
        assert!((metrics.cpu_percent - 8.5).abs() < 1e-3);
    }

    #[test]
    fn aggregates_root_only_when_no_children() {
        let rows = vec![
            PsRow { pid: 100, ppid: 1, rss_kib: 4096, cpu_percent: 5.0 },
        ];
        let metrics = aggregate_descendants(&rows, 100);
        assert_eq!(metrics.process_count, 1);
        assert_eq!(metrics.rss_bytes, 4096 * 1024);
    }

    #[test]
    fn aggregates_zero_when_root_missing() {
        let rows = vec![
            PsRow { pid: 200, ppid: 100, rss_kib: 1024, cpu_percent: 1.0 },
        ];
        let metrics = aggregate_descendants(&rows, 100);
        assert_eq!(metrics.process_count, 1); // root is "included" once even with no row
        assert_eq!(metrics.rss_bytes, 0);
        assert!((metrics.cpu_percent - 0.0).abs() < 1e-6);
    }
}
```

- [ ] **Step 2: Wire the new module into `src/core/mod.rs`**

Replace `src/core/mod.rs` with:

```rust
pub mod output;
pub mod run_loop;
pub mod sampling;
pub mod summary;
pub mod types;

pub use types::*;
```

- [ ] **Step 3: Run the new tests to verify they pass**

Run: `cargo test --lib core::sampling`
Expected: 5 tests pass.

- [ ] **Step 4: Run the full lib test suite to verify nothing else regressed**

Run: `cargo test --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/core/sampling.rs src/core/mod.rs
git commit -m "feat: add process tree sampling helper"
```

---

## Task 2: Extract spawn_with_bookkeeping and command_from_argv helpers

**Files:**
- Modify: `src/core/run_loop.rs`

This is a refactor. The "test" is the existing run_loop test suite continuing to pass.

- [ ] **Step 1: Add the two new pub(crate) helpers near the bottom of `src/core/run_loop.rs`**

After the `runtime_since` function (around line 938), add:

```rust
/// Build a `Command` from a flat argv (`argv[0]` is the program). Wires
/// stdio for pipe vs PTY mode and puts the child in its own process group
/// on unix. This is the only place that knows how to materialize a child
/// process for ANY backend.
pub(crate) fn command_from_argv(
    argv: &[std::ffi::OsString],
    io_mode: IoMode,
) -> anyhow::Result<Command> {
    anyhow::ensure!(!argv.is_empty(), "command argv must not be empty");

    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    let _ = io_mode; // io_mode is reserved for future PTY-specific stdio decisions
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
/// run loop only ever has one place that owns the spawn machinery.
pub(crate) fn spawn_with_bookkeeping(
    mut command: Command,
    io_mode: IoMode,
) -> anyhow::Result<RunningHandle> {
    let launched_at = SystemTime::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn command: {:?}", command.get_program()))?;

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
```

- [ ] **Step 2: Replace `PlainFallbackBackend::launch` body to delegate**

In `src/core/run_loop.rs`, find `impl Backend for PlainFallbackBackend` (around line 345). Replace the `launch` method body with:

```rust
    fn launch(&self, plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        let io_mode = preferred_io_mode(plan.resource_spec.interactive);
        let command = build_local_command(plan, io_mode)?;
        spawn_with_bookkeeping(command, io_mode)
    }
```

Note: `build_local_command` already builds a fully-configured `Command` with stdio + process_group set. It is NOT replaced by `command_from_argv` for this backend — `build_local_command` is the plain-fallback-specific path that handles PTY shimming via `script` and shell wrapping. The new `command_from_argv` is for the platform backends that already have a complete argv.

- [ ] **Step 3: Replace `PlainFallbackBackend::sample` body to use the new aggregator**

In the same `impl` block, replace the `sample` method body with:

```rust
    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample> {
        crate::core::sampling::sample_process_tree(handle.root_pid)
    }
```

Delete the now-unused `pid` formatting + ad-hoc `ps -p` invocation that lived in the old body.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS. The refactor is purely structural; existing tests must still pass. If `linux_pty_fallback_command_preview_uses_util_linux_script_shape` or any other existing run_loop test fails, you've changed observable behavior — revert and re-do.

- [ ] **Step 5: Commit**

```bash
git add src/core/run_loop.rs
git commit -m "refactor: extract spawn and sample helpers from run loop"
```

---

## Task 3: Add BackendKind::PlainFallback variant

**Files:**
- Modify: `src/core/types.rs`

- [ ] **Step 1: Add the variant**

In `src/core/types.rs`, find the `BackendKind` enum (around line 51):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    LinuxSystemd,
    MacosTaskpolicy,
    Unsupported,
}
```

Replace with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    LinuxSystemd,
    MacosTaskpolicy,
    PlainFallback,
    Unsupported,
}
```

- [ ] **Step 2: Add the matching `as_str` arm**

In the same file, find the `impl BackendKind` block:

```rust
impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxSystemd => "linux_systemd",
            Self::MacosTaskpolicy => "macos_taskpolicy",
            Self::Unsupported => "unsupported",
        }
    }
}
```

Replace with:

```rust
impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxSystemd => "linux_systemd",
            Self::MacosTaskpolicy => "macos_taskpolicy",
            Self::PlainFallback => "plain_fallback",
            Self::Unsupported => "unsupported",
        }
    }
}
```

- [ ] **Step 3: Build to confirm exhaustive match coverage**

Run: `cargo build --tests`
Expected: PASS. If a `match` somewhere in the codebase becomes non-exhaustive, the compiler will tell you exactly where — fix those by adding the new arm. As of the current code, no other `match` enumerates `BackendKind` arms, so this should compile clean.

- [ ] **Step 4: Commit**

```bash
git add src/core/types.rs
git commit -m "feat: add plain_fallback backend kind variant"
```

---

## Task 4: select_backend() and effective_backend_kind() in backend module

**Files:**
- Modify: `src/backend/mod.rs`

- [ ] **Step 1: Add the env-var helper at the top of `src/backend/mod.rs`**

Replace `src/backend/mod.rs` entirely with:

```rust
use crate::core::{BackendKind, CapabilityLevel, CapabilityReport, InteractiveMode, LaunchPlan, RunningHandle, Sample, Signal};

#[cfg(target_os = "linux")]
pub mod linux_systemd;
#[cfg(target_os = "macos")]
pub mod macos_taskpolicy;

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

const FORCE_BACKEND_ENV: &str = "SCALER_FORCE_BACKEND";

#[cfg(target_os = "linux")]
pub fn detect_host_capabilities() -> CapabilityReport {
    linux_systemd::detect_linux_capabilities(linux_systemd::probe_linux_host())
}

#[cfg(target_os = "macos")]
pub fn detect_host_capabilities() -> CapabilityReport {
    macos_taskpolicy::detect_macos_capabilities(
        macos_taskpolicy::probe_macos_host(),
        InteractiveMode::Auto,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn detect_host_capabilities() -> CapabilityReport {
    CapabilityReport::unsupported()
}

/// Returns the backend that `Command::Run` will actually use right now,
/// honoring the `SCALER_FORCE_BACKEND` test escape hatch when set.
pub fn select_backend() -> Box<dyn Backend> {
    if let Some(forced) = forced_backend() {
        return forced;
    }
    select_backend_from_capabilities()
}

/// Returns the same backend kind that `select_backend` would pick, without
/// instantiating it. Used by `doctor` so its `effective_backend:` line
/// matches what `run` would do.
pub fn effective_backend_kind() -> BackendKind {
    if let Some(kind) = forced_backend_kind() {
        return kind;
    }
    let report = detect_host_capabilities();
    if report.backend_state == CapabilityLevel::Unavailable {
        BackendKind::PlainFallback
    } else {
        report.backend
    }
}

fn forced_backend_kind() -> Option<BackendKind> {
    match std::env::var(FORCE_BACKEND_ENV).ok().as_deref() {
        Some("linux_systemd") => Some(BackendKind::LinuxSystemd),
        Some("macos_taskpolicy") => Some(BackendKind::MacosTaskpolicy),
        Some("plain_fallback") => Some(BackendKind::PlainFallback),
        _ => None,
    }
}

fn forced_backend() -> Option<Box<dyn Backend>> {
    let kind = forced_backend_kind()?;
    Some(boxed_backend_for_kind(kind))
}

#[cfg(target_os = "linux")]
fn select_backend_from_capabilities() -> Box<dyn Backend> {
    let report = detect_host_capabilities();
    if report.backend_state != CapabilityLevel::Unavailable {
        Box::new(linux_systemd::LinuxSystemdBackend)
    } else {
        Box::new(crate::core::run_loop::PlainFallbackBackend)
    }
}

#[cfg(target_os = "macos")]
fn select_backend_from_capabilities() -> Box<dyn Backend> {
    let report = detect_host_capabilities();
    if report.backend_state != CapabilityLevel::Unavailable {
        Box::new(macos_taskpolicy::MacosTaskpolicyBackend)
    } else {
        Box::new(crate::core::run_loop::PlainFallbackBackend)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn select_backend_from_capabilities() -> Box<dyn Backend> {
    Box::new(crate::core::run_loop::PlainFallbackBackend)
}

#[cfg(target_os = "linux")]
fn boxed_backend_for_kind(kind: BackendKind) -> Box<dyn Backend> {
    match kind {
        BackendKind::LinuxSystemd => Box::new(linux_systemd::LinuxSystemdBackend),
        _ => Box::new(crate::core::run_loop::PlainFallbackBackend),
    }
}

#[cfg(target_os = "macos")]
fn boxed_backend_for_kind(kind: BackendKind) -> Box<dyn Backend> {
    match kind {
        BackendKind::MacosTaskpolicy => Box::new(macos_taskpolicy::MacosTaskpolicyBackend),
        _ => Box::new(crate::core::run_loop::PlainFallbackBackend),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn boxed_backend_for_kind(_kind: BackendKind) -> Box<dyn Backend> {
    Box::new(crate::core::run_loop::PlainFallbackBackend)
}
```

Note: this references `linux_systemd::LinuxSystemdBackend` and `macos_taskpolicy::MacosTaskpolicyBackend` which don't exist yet. The build will fail until Tasks 5 and 6 land. **That is expected** — we will not run `cargo build` between this step and the next two tasks.

- [ ] **Step 2: Stage but do not commit yet**

```bash
git add src/backend/mod.rs
```

We'll commit after Tasks 5 and 6 add the backend types.

---

## Task 5: LinuxSystemdBackend impl Backend

**Files:**
- Modify: `src/backend/linux_systemd.rs`
- Test: `tests/backend_linux.rs`

- [ ] **Step 1: Write the failing command-preview test**

Add to `tests/backend_linux.rs` inside the existing `mod linux_tests { ... }` block, after the existing tests:

```rust
    #[test]
    fn linux_backend_command_preview_includes_systemd_run_and_resource_properties() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("/bin/echo"), OsString::from("hi")],
            resource_spec: ResourceSpec {
                cpu: Some(CpuLimit::from_centi_cores(50)),
                mem: Some(MemoryLimit::from_bytes(67_108_864)),
                interactive: InteractiveMode::Never,
                shell: None,
                monitor: false,
            },
            platform: Platform::Linux,
        };

        let preview =
            scaler::backend::linux_systemd::linux_systemd_command_preview_for_test(&plan).unwrap();
        let preview = preview
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(preview[0], "systemd-run");
        assert!(preview.iter().any(|value| value == "--user"));
        assert!(preview.iter().any(|value| value == "--scope"));
        assert!(preview.iter().any(|value| value == "--property=CPUQuota=50%"));
        assert!(preview.iter().any(|value| value == "--property=MemoryMax=67108864"));
        assert!(preview.iter().any(|value| value == "--property=MemorySwapMax=0"));
        let dash_dash = preview.iter().position(|value| value == "--").unwrap();
        assert_eq!(&preview[dash_dash + 1..], &["/bin/echo", "hi"]);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test backend_linux linux_backend_command_preview_includes_systemd_run_and_resource_properties`
Expected: FAIL with "no function or associated item named `linux_systemd_command_preview_for_test` found".

- [ ] **Step 3: Add `LinuxSystemdBackend` and the test seam**

In `src/backend/linux_systemd.rs`, after the existing `pub fn build_systemd_run_argv` function (around line 73), append:

```rust
use crate::backend::Backend;
use crate::core::{IoMode, RunningHandle, Sample, Signal};
use crate::core::run_loop::{command_from_argv, preferred_io_mode_for, spawn_with_bookkeeping};

#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxSystemdBackend;

impl Backend for LinuxSystemdBackend {
    fn detect(&self) -> crate::core::CapabilityReport {
        detect_linux_capabilities(probe_linux_host())
    }

    fn launch(&self, plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        let io_mode = preferred_io_mode_for(plan.resource_spec.interactive);
        let argv = build_systemd_run_argv(plan)?;
        let command = command_from_argv(&argv, io_mode)?;
        spawn_with_bookkeeping(command, io_mode)
    }

    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        crate::core::run_loop::try_wait_via_registry(handle.root_pid)
    }

    fn sample(&self, handle: &RunningHandle) -> anyhow::Result<Sample> {
        crate::core::sampling::sample_process_tree(handle.root_pid)
    }

    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        crate::core::run_loop::terminate_process_group(handle.root_pid, signal)
    }
}

/// Test seam: returns the argv that `LinuxSystemdBackend.launch` would
/// hand to `command_from_argv`. Used by integration tests so they can
/// assert on the wiring without spawning a real process.
pub fn linux_systemd_command_preview_for_test(
    plan: &LaunchPlan,
) -> anyhow::Result<Vec<std::ffi::OsString>> {
    build_systemd_run_argv(plan)
}
```

This references three functions that need to be made available from `run_loop.rs`:
- `preferred_io_mode_for` — wraps the existing private `preferred_io_mode` so backends can call it
- `try_wait_via_registry` — wraps the existing `PlainFallbackBackend::try_wait` body
- `terminate_process_group` — wraps the existing `PlainFallbackBackend::terminate` body

- [ ] **Step 4: Expose the three helpers from `run_loop.rs`**

In `src/core/run_loop.rs`, add the following public helpers near the other `pub(crate)` helpers added in Task 2 (after `spawn_with_bookkeeping`):

```rust
/// Public alias for the run loop's PTY-vs-pipes selection rule. Backends
/// call this so PTY/pipe semantics stay centralized.
pub fn preferred_io_mode_for(interactive: InteractiveMode) -> IoMode {
    preferred_io_mode(interactive)
}

/// Polls the registered process for the given pid and returns its exit
/// status if available. Used by all platform backend `try_wait` impls.
pub fn try_wait_via_registry(
    root_pid: u32,
) -> anyhow::Result<Option<std::process::ExitStatus>> {
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
        .with_context(|| {
            format!(
                "failed to send {signal_flag} to process group {root_pid}"
            )
        })?;
    anyhow::ensure!(
        status.success(),
        "kill command exited unsuccessfully for process group {root_pid}"
    );
    Ok(())
}
```

Then change `PlainFallbackBackend::try_wait` and `PlainFallbackBackend::terminate` to delegate:

```rust
    fn try_wait(
        &self,
        handle: &mut RunningHandle,
    ) -> anyhow::Result<Option<std::process::ExitStatus>> {
        try_wait_via_registry(handle.root_pid)
    }

    fn terminate(&self, handle: &RunningHandle, signal: Signal) -> anyhow::Result<()> {
        terminate_process_group(handle.root_pid, signal)
    }
```

Delete the now-duplicated bodies.

- [ ] **Step 5: Run the new test to verify it passes**

Run: `cargo test --test backend_linux linux_backend_command_preview_includes_systemd_run_and_resource_properties`
Expected: PASS on Linux. On macOS, the test is gated by `#[cfg(target_os = "linux")]` and is silently absent.

- [ ] **Step 6: Run the full Linux backend test suite**

Run: `cargo test --test backend_linux`
Expected: all existing tests still PASS.

- [ ] **Step 7: Stage but do not commit yet**

```bash
git add src/backend/linux_systemd.rs src/core/run_loop.rs tests/backend_linux.rs
```

We'll commit after Task 6 lands the macOS counterpart so the build is green on both platforms in one atomic change.

---

## Task 6: MacosTaskpolicyBackend impl Backend (with -m gating)

**Files:**
- Modify: `src/backend/macos_taskpolicy.rs`
- Modify: `tests/backend_macos.rs`

The current `build_taskpolicy_argv` always emits `-m <mib>` when `--mem` is requested, even when the host's `taskpolicy` does not support it. That spawn would fail at runtime. We fix this by gating `-m` on a probed `include_memory_flag`.

- [ ] **Step 1: Update the existing `build_taskpolicy_argv` signature and add the gate**

In `src/backend/macos_taskpolicy.rs`, replace `pub fn build_taskpolicy_argv(plan: &LaunchPlan) -> anyhow::Result<Vec<OsString>>` with:

```rust
pub fn build_taskpolicy_argv(
    plan: &LaunchPlan,
    include_memory_flag: bool,
) -> anyhow::Result<Vec<OsString>> {
    anyhow::ensure!(
        plan.platform == Platform::Macos,
        "macos taskpolicy backend requires a macos launch plan"
    );
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    let mut argv = vec![
        OsString::from("taskpolicy"),
        OsString::from("-b"),
        OsString::from("-d"),
        OsString::from("throttle"),
        OsString::from("-g"),
        OsString::from("default"),
        OsString::from("--"),
    ];

    if include_memory_flag {
        if let Some(mem) = plan.resource_spec.mem {
            let mib = mem.bytes().div_ceil(1_048_576);
            argv.pop();
            argv.push(OsString::from("-m"));
            argv.push(OsString::from(mib.to_string()));
            argv.push(OsString::from("--"));
        }
    }

    match plan.resource_spec.shell {
        Some(shell) => {
            anyhow::ensure!(
                plan.argv.len() == 1,
                "shell launch plan requires exactly one script token"
            );
            argv.push(shell_program(shell));
            argv.push(OsString::from("-lc"));
            argv.push(plan.argv[0].clone());
        }
        None => argv.extend(plan.argv.iter().cloned()),
    }

    Ok(argv)
}
```

- [ ] **Step 2: Update existing macOS unit tests to pass the new flag**

In `tests/backend_macos.rs`, every call to `build_taskpolicy_argv(&plan)` becomes `build_taskpolicy_argv(&plan, true)` for the existing tests that expect `-m` to be present, and the test that asserts dash-prefixed-executable preservation (`macos_command_preserves_dash_prefixed_executable_after_delimiter`, around line 338) should pass `false` since it does not request memory.

Search-and-replace pattern: `build_taskpolicy_argv(&plan)` → `build_taskpolicy_argv(&plan, true)` everywhere except the dash-prefixed test, which becomes `build_taskpolicy_argv(&plan, false)`.

After the substitution, the existing test `macos_command_builds_taskpolicy_argv` (around line 271) still expects `-m` to be present — leave that one as `true`.

Add a new test asserting the gate works:

```rust
    #[test]
    fn macos_command_omits_memory_flag_when_unsupported() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("echo"), OsString::from("ok")],
            resource_spec: ResourceSpec {
                cpu: None,
                mem: Some(MemoryLimit::from_bytes(67_108_864)),
                interactive: InteractiveMode::Never,
                shell: None,
                monitor: true,
            },
            platform: Platform::Macos,
        };

        let argv = build_taskpolicy_argv(&plan, false).unwrap();
        let argv = argv
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(!argv.iter().any(|value| value == "-m"));
        assert_eq!(&argv[argv.len() - 2..], ["echo", "ok"]);
    }
```

- [ ] **Step 3: Write the failing macOS backend command-preview test**

Add inside `tests/backend_macos.rs`'s `mod macos_tests { ... }`:

```rust
    #[test]
    fn macos_backend_command_preview_uses_taskpolicy() {
        let plan = LaunchPlan {
            argv: vec![OsString::from("/bin/echo"), OsString::from("hi")],
            resource_spec: ResourceSpec {
                cpu: Some(CpuLimit::from_centi_cores(100)),
                mem: Some(MemoryLimit::from_bytes(67_108_864)),
                interactive: InteractiveMode::Never,
                shell: None,
                monitor: false,
            },
            platform: Platform::Macos,
        };

        let preview =
            scaler::backend::macos_taskpolicy::macos_taskpolicy_command_preview_for_test(
                &plan, true,
            )
            .unwrap();
        let preview = preview
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(preview[0], "taskpolicy");
        assert!(preview.iter().any(|value| value == "-d"));
        assert!(preview.iter().any(|value| value == "-g"));
        assert!(preview.iter().any(|value| value == "-m"));
        assert_eq!(&preview[preview.len() - 2..], ["/bin/echo", "hi"]);
    }
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --test backend_macos macos_backend_command_preview_uses_taskpolicy`
Expected on macOS: FAIL with "no function or associated item named `macos_taskpolicy_command_preview_for_test`". On Linux it is gated and absent.

- [ ] **Step 5: Add `MacosTaskpolicyBackend` and the test seam**

In `src/backend/macos_taskpolicy.rs`, after the existing `pub fn detect_macos_capabilities` (around line 140), append:

```rust
use crate::backend::Backend;
use crate::core::{IoMode, RunningHandle, Sample, Signal};
use crate::core::run_loop::{
    command_from_argv, preferred_io_mode_for, spawn_with_bookkeeping, terminate_process_group,
    try_wait_via_registry,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct MacosTaskpolicyBackend;

impl Backend for MacosTaskpolicyBackend {
    fn detect(&self) -> crate::core::CapabilityReport {
        detect_macos_capabilities(probe_macos_host(), InteractiveMode::Auto)
    }

    fn launch(&self, plan: &LaunchPlan) -> anyhow::Result<RunningHandle> {
        let io_mode = preferred_io_mode_for(plan.resource_spec.interactive);
        let include_memory_flag = probe_macos_host().has_memory_support;
        let argv = build_taskpolicy_argv(plan, include_memory_flag)?;
        let command = command_from_argv(&argv, io_mode)?;
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

/// Test seam: returns the argv that `MacosTaskpolicyBackend.launch` would
/// hand to `command_from_argv`. Used by integration tests so they can
/// assert on the wiring without spawning a real process.
pub fn macos_taskpolicy_command_preview_for_test(
    plan: &LaunchPlan,
    include_memory_flag: bool,
) -> anyhow::Result<Vec<std::ffi::OsString>> {
    build_taskpolicy_argv(plan, include_memory_flag)
}
```

Note on PTY: macOS `taskpolicy` is not a PTY allocator, so when `io_mode == IoMode::Pty` we still spawn `taskpolicy ... -- <target>` and the target inherits the parent's stdio (which `command_from_argv` configures as piped). PTY-mode interactive commands on macOS therefore degrade to pipe relay until a future task adds a `script` shim wrapper. Document this behavior in the warning when the user passes `--interactive always` on macOS — that's deferred to a future task and not in this plan's scope.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --test backend_macos macos_backend_command_preview_uses_taskpolicy`
Expected on macOS: PASS.

- [ ] **Step 7: Run the full macOS backend test suite**

Run: `cargo test --test backend_macos`
Expected on macOS: all tests PASS, including the new `macos_command_omits_memory_flag_when_unsupported`.

- [ ] **Step 8: Stage but do not commit yet**

```bash
git add src/backend/macos_taskpolicy.rs tests/backend_macos.rs
```

We'll commit everything together after Task 7 finishes wiring `lib.rs`.

---

## Task 7: Wire lib.rs through select_backend() + doctor effective_backend line

**Files:**
- Modify: `src/cli/mod.rs`
- Modify: `src/lib.rs`
- Modify: `tests/doctor_cli.rs`

- [ ] **Step 1: Update the doctor renderer signature**

In `src/cli/mod.rs`, find:

```rust
pub fn render_doctor_output(report: &crate::core::CapabilityReport) -> String {
```

Replace with:

```rust
pub fn render_doctor_output(
    report: &crate::core::CapabilityReport,
    effective: crate::core::BackendKind,
) -> String {
```

Inside the function body, find where the core capability lines are constructed:

```rust
    let mut lines = vec![
        format!("platform: {}", report.platform.as_str()),
        format!("backend: {}", report.backend.as_str()),
        format!("backend_state: {}", report.backend_state.as_str()),
        format!("cpu: {}", report.cpu.as_str()),
        format!("memory: {}", report.memory.as_str()),
        format!("interactive: {}", report.interactive.as_str()),
    ];
```

Replace with:

```rust
    let mut lines = vec![
        format!("platform: {}", report.platform.as_str()),
        format!("backend: {}", report.backend.as_str()),
        format!("backend_state: {}", report.backend_state.as_str()),
        format!("cpu: {}", report.cpu.as_str()),
        format!("memory: {}", report.memory.as_str()),
        format!("interactive: {}", report.interactive.as_str()),
        format!("effective_backend: {}", effective.as_str()),
    ];
```

- [ ] **Step 2: Update `lib.rs` to feed the new arg**

In `src/lib.rs`, find the `Command::Doctor` arm:

```rust
        crate::cli::args::Command::Doctor => {
            let report = crate::backend::detect_host_capabilities();
            println!("{}", crate::cli::render_doctor_output(&report));
            Ok(())
        }
```

Replace with:

```rust
        crate::cli::args::Command::Doctor => {
            let report = crate::backend::detect_host_capabilities();
            let effective = crate::backend::effective_backend_kind();
            println!("{}", crate::cli::render_doctor_output(&report, effective));
            Ok(())
        }
```

- [ ] **Step 3: Update `lib.rs` to use `select_backend()` for `Command::Run`**

In the same file, find the `Command::Run(run)` arm:

```rust
        crate::cli::args::Command::Run(run) => {
            let plan = build_launch_plan(run);
            let backend = crate::core::run_loop::PlainFallbackBackend;
            let _signal_bridge = crate::core::run_loop::install_signal_bridge()?;
            let outcome = crate::core::run_loop::execute(plan, &backend)?;
            if let Some(exit_code) = resolved_exit_code(&outcome.exit_status)
                && exit_code != 0
            {
                std::process::exit(exit_code);
            }
            Ok(())
        }
```

Replace with:

```rust
        crate::cli::args::Command::Run(run) => {
            let plan = build_launch_plan(run);
            warn_if_resource_limits_will_be_dropped(&plan);
            let backend = crate::backend::select_backend();
            let _signal_bridge = crate::core::run_loop::install_signal_bridge()?;
            let outcome = crate::core::run_loop::execute(plan, backend.as_ref())?;
            if let Some(exit_code) = resolved_exit_code(&outcome.exit_status)
                && exit_code != 0
            {
                std::process::exit(exit_code);
            }
            Ok(())
        }
```

- [ ] **Step 4: Add the warning helper at the bottom of `src/lib.rs`**

After the `resolved_exit_code` function, add:

```rust
fn warn_if_resource_limits_will_be_dropped(plan: &crate::core::LaunchPlan) {
    let asked_for_limits =
        plan.resource_spec.cpu.is_some() || plan.resource_spec.mem.is_some();
    if !asked_for_limits {
        return;
    }
    if crate::backend::effective_backend_kind() == crate::core::BackendKind::PlainFallback {
        eprintln!(
            "scaler: resource limits NOT being enforced on this host; run `scaler doctor` for details"
        );
    }
}
```

- [ ] **Step 5: Update `tests/doctor_cli.rs` slice indices**

In `tests/doctor_cli.rs`, find the Linux branch of `doctor_prints_capability_states`:

```rust
        assert_line_prefixes(
            &lines[6..9],
            &[
                "prerequisite: systemd_run=",
                "prerequisite: cgroup_v2=",
                "prerequisite: user_manager=",
            ],
        );
        assert_sorted_warning_lines(&lines[9..]);
```

Change `&lines[6..9]` to `&lines[7..10]` and `&lines[9..]` to `&lines[10..]`. The new line 6 is `effective_backend: ...`.

Also assert the new line is present. Insert before the prerequisite slice assertion:

```rust
        assert!(lines[6].starts_with("effective_backend: "));
```

For the macOS branch:

```rust
        assert_line_prefixes(
            &lines[6..8],
            &[
                "prerequisite: taskpolicy=",
                "prerequisite: platform_version=",
            ],
        );
        assert_sorted_warning_lines(&lines[8..]);
```

Change `&lines[6..8]` to `&lines[7..9]` and `&lines[8..]` to `&lines[9..]`. Add:

```rust
        assert!(lines[6].starts_with("effective_backend: "));
```

For the unsupported branch, replace the expected string:

```rust
        let expected = concat!(
            "platform: unsupported\n",
            "backend: unsupported\n",
            "backend_state: unavailable\n",
            "cpu: unavailable\n",
            "memory: unavailable\n",
            "interactive: unavailable\n",
            "prerequisite: no supported backend for this host\n",
        );
```

with:

```rust
        let expected = concat!(
            "platform: unsupported\n",
            "backend: unsupported\n",
            "backend_state: unavailable\n",
            "cpu: unavailable\n",
            "memory: unavailable\n",
            "interactive: unavailable\n",
            "effective_backend: plain_fallback\n",
            "prerequisite: no supported backend for this host\n",
        );
```

In `doctor_uses_only_known_capability_words`, the existing assertion `assert_eq!(capability_values.len(), 4);` still passes because the filter only keys on `backend_state | cpu | memory | interactive`, so `effective_backend` is excluded. No change needed.

In `doctor_renderer_uses_structured_prerequisites_in_declared_order`, find the test body. It calls `render_doctor_output(&report)` — change to `render_doctor_output(&report, BackendKind::PlainFallback)` (since the report has all-unavailable). Update the expected concat string to include the new line:

```rust
    let expected = concat!(
        "platform: linux\n",
        "backend: linux_systemd\n",
        "backend_state: unavailable\n",
        "cpu: unavailable\n",
        "memory: unavailable\n",
        "interactive: unavailable\n",
        "effective_backend: plain_fallback\n",
        "prerequisite: systemd_run=missing\n",
        "prerequisite: cgroup_v2=missing\n",
        "prerequisite: user_manager=skipped\n",
        "warning: a warning\n",
        "warning: z warning",
    );
```

In `assert_core_lines`, the helper currently asserts `lines.len() >= 8`. That still holds (we add a 7th core line, plus prerequisites + warnings). No change.

The `assert_core_lines` helper itself only checks the first 6 lines and is called with a `&[&str; 6]` array — also unchanged.

Add a new renderer test for the enforced-happy-path case:

```rust
#[test]
fn doctor_renderer_emits_effective_backend_line() {
    let report = CapabilityReport {
        platform: Platform::Linux,
        backend: BackendKind::LinuxSystemd,
        backend_state: CapabilityLevel::Enforced,
        cpu: CapabilityLevel::Enforced,
        memory: CapabilityLevel::Enforced,
        interactive: CapabilityLevel::Enforced,
        prerequisites: vec![
            DoctorPrerequisite::check("systemd_run", PrerequisiteStatus::Ok),
            DoctorPrerequisite::check("cgroup_v2", PrerequisiteStatus::Ok),
            DoctorPrerequisite::check("user_manager", PrerequisiteStatus::Ok),
        ],
        warnings: vec![],
    };

    let stdout = render_doctor_output(&report, BackendKind::LinuxSystemd);

    assert!(
        stdout.contains("effective_backend: linux_systemd"),
        "expected effective_backend line, got: {stdout}"
    );
}
```

- [ ] **Step 6: Run the doctor tests to verify they pass**

Run: `cargo test --test doctor_cli`
Expected: PASS.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: PASS on the host platform. Note that on a host where systemd-run / taskpolicy is unavailable (e.g., GitHub Actions ubuntu runner without `systemd --user`), the existing `binary_run_*` tests still pass because `select_backend()` falls back to `PlainFallbackBackend` which is the same code path the tests previously exercised.

- [ ] **Step 8: Run formatter and clippy**

Run:

```bash
cargo fmt -- --check
cargo clippy --tests -- -D warnings
```

Expected: no output (i.e., success).

- [ ] **Step 9: Stage and commit Tasks 4 through 7 as one atomic change**

```bash
git add src/cli/mod.rs src/lib.rs tests/doctor_cli.rs
git commit -m "feat: wire linux_systemd and macos_taskpolicy backends through run loop"
```

The single commit captures: `select_backend()` + LinuxSystemdBackend + MacosTaskpolicyBackend + lib.rs wiring + doctor effective_backend line + macOS `-m` gate fix. Tests for all of these are included in the commit, so the tree is green at every commit boundary.

---

## Task 8: PATH-shim integration tests

**Files:**
- Modify: `tests/backend_linux.rs`
- Modify: `tests/backend_macos.rs`

These tests prove the wiring is correct end-to-end on each OS by spawning the real `scaler` binary against a fake `systemd-run` / `taskpolicy` shim that records its argv.

- [ ] **Step 1: Add the Linux PATH-shim integration test**

Append to the `mod linux_tests { ... }` block in `tests/backend_linux.rs`:

```rust
    use std::{env, fs, os::unix::fs::PermissionsExt};

    #[test]
    fn linux_backend_invokes_systemd_run_with_resource_properties_via_shim() {
        let temp = tempfile::tempdir().unwrap();
        let shim_dir = temp.path().join("bin");
        fs::create_dir_all(&shim_dir).unwrap();
        let log_path = temp.path().join("argv.log");

        let shim_body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{log}'\nwhile [ \"$#\" -gt 0 ]; do\n    arg=\"$1\"; shift\n    [ \"$arg\" = \"--\" ] && break\ndone\nexec \"$@\"\n",
            log = log_path.display()
        );
        let shim_path = shim_dir.join("systemd-run");
        fs::write(&shim_path, shim_body).unwrap();
        let mut perms = fs::metadata(&shim_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim_path, perms).unwrap();

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", shim_dir.display(), original_path);

        let assert = assert_cmd::Command::cargo_bin("scaler")
            .unwrap()
            .env("PATH", &new_path)
            .env("SCALER_FORCE_BACKEND", "linux_systemd")
            .args([
                "run",
                "--cpu",
                "0.5c",
                "--mem",
                "64m",
                "--",
                "/bin/echo",
                "ok",
            ])
            .assert();

        assert.success();

        let recorded = fs::read_to_string(&log_path).unwrap();
        assert!(recorded.contains("--user"), "argv: {recorded}");
        assert!(recorded.contains("--scope"), "argv: {recorded}");
        assert!(
            recorded.contains("--property=CPUQuota=50%"),
            "argv: {recorded}"
        );
        assert!(
            recorded.contains("--property=MemoryMax=67108864"),
            "argv: {recorded}"
        );
        assert!(
            recorded.contains("--property=MemorySwapMax=0"),
            "argv: {recorded}"
        );
    }
```

- [ ] **Step 2: Run the Linux integration test to verify it passes**

Run on Linux: `cargo test --test backend_linux linux_backend_invokes_systemd_run_with_resource_properties_via_shim`
Expected on Linux: PASS.

On macOS, the test is gated and absent. To verify locally on macOS, you'll do the same exercise via Task 8 step 3 below for the macOS shim.

- [ ] **Step 3: Add the macOS PATH-shim integration test**

Append to the `mod macos_tests { ... }` block in `tests/backend_macos.rs`:

```rust
    use std::{env, fs, os::unix::fs::PermissionsExt};

    #[test]
    fn macos_backend_invokes_taskpolicy_with_throttle_class_via_shim() {
        let temp = tempfile::tempdir().unwrap();
        let shim_dir = temp.path().join("bin");
        fs::create_dir_all(&shim_dir).unwrap();
        let log_path = temp.path().join("argv.log");

        let shim_body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{log}'\nwhile [ \"$#\" -gt 0 ]; do\n    arg=\"$1\"; shift\n    [ \"$arg\" = \"--\" ] && break\ndone\nexec \"$@\"\n",
            log = log_path.display()
        );
        let shim_path = shim_dir.join("taskpolicy");
        fs::write(&shim_path, shim_body).unwrap();
        let mut perms = fs::metadata(&shim_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim_path, perms).unwrap();

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", shim_dir.display(), original_path);

        let assert = assert_cmd::Command::cargo_bin("scaler")
            .unwrap()
            .env("PATH", &new_path)
            .env("SCALER_FORCE_BACKEND", "macos_taskpolicy")
            .args(["run", "--cpu", "0.5c", "--", "/bin/echo", "ok"])
            .assert();

        assert.success();

        let recorded = fs::read_to_string(&log_path).unwrap();
        assert!(recorded.contains("-b"), "argv: {recorded}");
        assert!(recorded.contains("throttle"), "argv: {recorded}");
        assert!(recorded.contains("default"), "argv: {recorded}");
    }
```

Note: this test deliberately omits `--mem` because the macOS host's real `taskpolicy` may not support `-m`, and `MacosTaskpolicyBackend.launch` calls the live `probe_macos_host()` to decide whether to include the flag — which would test the probe rather than the wiring. The CPU-only argv is enough to prove the launch path is correct.

- [ ] **Step 4: Run the macOS integration test to verify it passes**

Run on macOS: `cargo test --test backend_macos macos_backend_invokes_taskpolicy_with_throttle_class_via_shim`
Expected on macOS: PASS.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: PASS on both Linux and macOS.

- [ ] **Step 6: Add `tempfile` as a dev-dependency if not already present**

Inspect `Cargo.toml`. The `[dev-dependencies]` section already lists `tempfile = "3"` — no change needed. Skip this step if confirmed.

- [ ] **Step 7: Commit**

```bash
git add tests/backend_linux.rs tests/backend_macos.rs
git commit -m "test: add path-shim integration coverage for platform backends"
```

---

## Task 9: README install section + arm64 TODO + version bump + changelog

**Files:**
- Modify: `README.md`
- Modify: `.github/workflows/release.yml`
- Modify: `Cargo.toml`
- Create: `CHANGELOG.md`

- [ ] **Step 1: Add an Install section to `README.md`**

In `README.md`, find the line `## Supported command forms` (line 5) and insert this section just BEFORE it:

```markdown
## Install

### From a release tarball (Linux x86_64)

```bash
VERSION=v0.2.0
TARGET=x86_64-unknown-linux-gnu
curl -fsSL "https://github.com/reedchan7/scaler/releases/download/${VERSION}/scaler-${VERSION}-${TARGET}.tar.gz" \
  | tar -xz -C /tmp
sudo install -m 0755 "/tmp/scaler-${VERSION}-${TARGET}/scaler" /usr/local/bin/scaler
scaler doctor
```

If `scaler doctor` reports `effective_backend: plain_fallback` on a Linux host, your user systemd manager is not reachable and resource limits will not be enforced. Enable lingering for your user (`sudo loginctl enable-linger "$USER"`), log out and back in, then re-run `scaler doctor`. Once the doctor reports `effective_backend: linux_systemd`, `scaler run --cpu 0.5c --mem 64m -- <cmd>` will actually constrain the command via a transient systemd scope.

### From source

```bash
cargo build --release
./target/release/scaler doctor
```

```

(The triple backticks above are part of the README content. When you paste, ensure they render as a fenced block in the final file.)

- [ ] **Step 2: Add the arm64 TODO comment to `release.yml`**

In `.github/workflows/release.yml`, find the matrix block (around line 65):

```yaml
      matrix:
        include:
          - os: ubuntu-24.04
            target: x86_64-unknown-linux-gnu
            archive_name: scaler-${{ github.ref_name }}-x86_64-unknown-linux-gnu.tar.gz
          - os: macos-14
            target: aarch64-apple-darwin
            archive_name: scaler-${{ github.ref_name }}-aarch64-apple-darwin.tar.gz
```

Insert this comment just above `matrix:`:

```yaml
      # TODO(arm64): add aarch64-unknown-linux-gnu (cross-compile or ARM runner)
      # and x86_64-apple-darwin once we need them. Tracked in README install section.
```

- [ ] **Step 3: Bump the package version**

In `Cargo.toml`, change `version = "0.1.0"` to `version = "0.2.0"`.

- [ ] **Step 4: Create `CHANGELOG.md`**

Create `CHANGELOG.md` with this content:

```markdown
# Changelog

All notable changes to scaler are documented in this file.

## 0.2.0

### Fixed

- `scaler run --cpu` and `--mem` now actually enforce limits on Linux via a
  transient `systemd --user --scope` unit. Previous releases accepted the
  flags but launched the command unconstrained because the run loop was
  hardcoded to a fallback backend.
- macOS runs now wrap the target with `taskpolicy -b -d throttle -g default`
  for best-effort CPU lowering. The `-m` memory flag is now gated on whether
  the host's `taskpolicy` actually supports it, instead of being emitted
  unconditionally and crashing at spawn time.
- `scaler` warns to stderr when the user passes `--cpu` or `--mem` but the
  effective backend is `plain_fallback`, so users can no longer be silently
  unenforced.

### Added

- `scaler doctor` now prints an `effective_backend:` line that names the
  backend `scaler run` will actually use (`linux_systemd`, `macos_taskpolicy`,
  or `plain_fallback`).
- `Backend::sample` now aggregates RSS and CPU across the entire descendant
  process tree of the launched command, not just the root pid.
- New `SCALER_FORCE_BACKEND` test escape hatch (intended for integration
  tests, not user-facing API).

## 0.1.0

- Initial release: CLI, doctor, run loop, monitor, plain fallback.
```

- [ ] **Step 5: Commit**

```bash
git add README.md .github/workflows/release.yml Cargo.toml CHANGELOG.md
git commit -m "docs: install section, arm64 todo, 0.2.0 changelog"
```

---

## Task 10: Final verification and dogfood

**Files:** none

- [ ] **Step 1: Run the full local CI quartet**

Run:

```bash
cargo fmt -- --check
cargo clippy --tests -- -D warnings
cargo test -- --nocapture
cargo build --release
```

Expected: all four PASS. If any fails, do not push — fix and re-verify.

- [ ] **Step 2: Smoke test the binary on the dev host**

Run:

```bash
./target/release/scaler doctor
./target/release/scaler version
./target/release/scaler --cpu 0.5c -- /bin/echo "scaled hi"
```

Expected:
- `doctor` prints the new `effective_backend:` line
- `version` prints `scaler 0.2.0 <os>-<arch>`
- the third command exits with code 0 and prints `scaled hi` followed by the summary block (`exit_status:`, `runtime:`, `peak_memory:`, `samples:`)

If you are on macOS and `doctor` says `effective_backend: macos_taskpolicy`, the third command runs through the real taskpolicy backend. If on Linux without `systemd --user`, you should also see the stderr warning `scaler: resource limits NOT being enforced ...`.

- [ ] **Step 3: Push to GitHub and tag a release**

```bash
git push origin main
```

Then either:
1. Manually run the `Manual Release Tag` workflow from GitHub UI with version bump = `minor` to create the `v0.2.0` tag, OR
2. Locally:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

The `Release` workflow will trigger off the tag and produce the x86_64 Linux + aarch64 macOS artifacts.

- [ ] **Step 4: Verify the release artifact end-to-end on the Linux server**

After the Release workflow completes, on the Linux server:

```bash
VERSION=v0.2.0
TARGET=x86_64-unknown-linux-gnu
curl -fsSL "https://github.com/reedchan7/scaler/releases/download/${VERSION}/scaler-${VERSION}-${TARGET}.tar.gz" \
  | tar -xz -C /tmp
sudo install -m 0755 "/tmp/scaler-${VERSION}-${TARGET}/scaler" /usr/local/bin/scaler
scaler doctor
```

Expected: `doctor` reports `effective_backend: linux_systemd` if your user systemd manager is reachable. If it reports `plain_fallback`, run `sudo loginctl enable-linger "$USER"`, log out and back in, and re-run `doctor`.

Once `doctor` is happy, exercise the resource limits:

```bash
scaler --cpu 0.5c --mem 256m -- bash -c '
  systemctl --user status $(cat /proc/self/cgroup | cut -d: -f3 | sed "s|^/||" | xargs basename)
  echo done
'
```

Expected: the inner `systemctl --user status` line names a transient `.scope` unit and shows `CPUQuota=50%` and `MemoryMax=256.0M` in its properties. That is the proof that scaler's wiring works on a real host.

---

## Self-Review Checklist (run before handing off)

- [x] Every spec requirement from `docs/superpowers/specs/2026-04-02-scaler-v1-design.md` for the run path has a corresponding task. The Linux backend launch + sampling + termination contract is covered by Tasks 4–8.
- [x] No placeholders. Every code block contains the literal text to write or modify.
- [x] Type names are consistent: `LinuxSystemdBackend`, `MacosTaskpolicyBackend`, `BackendKind::PlainFallback`, `select_backend`, `effective_backend_kind`, `command_from_argv`, `spawn_with_bookkeeping`, `preferred_io_mode_for`, `try_wait_via_registry`, `terminate_process_group`, `sample_process_tree`, `linux_systemd_command_preview_for_test`, `macos_taskpolicy_command_preview_for_test`, `SCALER_FORCE_BACKEND`.
- [x] Each task ends in a commit (Tasks 4 through 7 share one atomic commit because the build is intentionally red between Task 4's start and Task 7's end).
- [x] TDD discipline: every behavior change has a failing test written first, then the minimum implementation, then a re-run.
- [x] Out-of-scope items (aarch64 builds, cgroup file sampling, systemctl --user kill termination, macOS PTY shim wrapping) are explicitly listed in the Architecture section and as TODO/comment in `release.yml`.
