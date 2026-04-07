# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`scaler` is a Rust CLI (edition 2024) that runs an existing host command under normalized CPU/memory flags with a deterministic capability report and a compact monitor-oriented run loop. V1 targets Linux (strong enforcement via `systemd-run` + cgroup v2) and macOS (best-effort via `taskpolicy` / `renice`). See `README.md` for the user-facing CLI surface. The original design notes and implementation plans live under `docs/superpowers/` and are gitignored (kept locally for agent-driven workflows); when working in this repo without those notes, treat the README and inline `///` doc comments as the source of truth.

## Commands

The CI matrix in `.github/workflows/ci.yml` runs exactly these checks on Ubuntu and macOS — match them locally before pushing:

```bash
cargo fmt -- --check
cargo clippy --tests -- -D warnings
cargo test -- --nocapture
cargo build --release
```

Useful subsets while iterating:

```bash
# Run one integration test file (files in tests/ are separate crates)
cargo test --test run_loop
cargo test --test cli_parse -- --nocapture

# Run one test by name (substring match across the whole workspace)
cargo test render_doctor_output

# Run only library unit tests
cargo test --lib
```

The `Release` and `Manual Release Tag` workflows are tag-driven; do not invoke them as part of local development.

## Architecture

Four top-level modules under `src/`, with a strict layering rule: **`cli` → `core` → `backend` and `ui`**. `core` is the only layer that orchestrates the run loop; `ui` never owns OS process handles, and `backend` never decides UI/lifecycle policy.

### Entry point and shorthand normalization (`src/lib.rs`, `src/cli/`)

`scaler::run` (called from `main.rs`) parses argv via `cli::parse_from`, which first runs `cli::normalize_argv` to rewrite the shorthand form `scaler [run-flags] -- <cmd>` into the explicit `scaler run [run-flags] -- <cmd>` shape before clap sees it. The shorthand is recognized when argv[1] is either `--` or one of the `run` flags (`--cpu`, `--mem`, `--interactive`, `--shell`, `--no-monitor`) **and** a `--` delimiter is present later. Anything else is left untouched so the unknown-subcommand error path stays intact.

`Cli::try_parse_from` runs `RunCommand::validate` after clap, enforcing two rules that are not expressible in the derive macros:

- `--shell` requires **exactly one** trailing token after `--` (the script string).
- Without `--shell`, at least one trailing token is required.

CPU/memory parsing lives in `src/cli/values.rs`. Both go through `parse_rounded_positive`, which uses `rust_decimal` with `MidpointAwayFromZero`, rejects scientific notation / underscores, and enforces the floors (`>= 1` centi-core, `>= 1 MiB`).

### Core types and the launch plan (`src/core/`)

`core::types` defines the platform-neutral execution intent: `CpuLimit` (centi-cores), `MemoryLimit` (bytes), `ResourceSpec`, `LaunchPlan`, and the capability vocabulary `CapabilityLevel { Enforced, BestEffort, Unavailable }` with `CapabilityReport` and `DoctorPrerequisite`. `lib.rs::build_launch_plan` is the single funnel that converts CLI args + `current_platform()` into a `LaunchPlan`. `core::output`, `core::summary`, and `core::run_loop` consume only those neutral types — never CLI structs.

### The run loop (`src/core/run_loop.rs`)

This is the longest and most subtle file. Key invariants:

- **Backend selection lives in `backend::select_backend` (`src/backend/mod.rs`)**: on Linux it picks `LinuxSystemdBackend` whenever `detect_host_capabilities()` reports `backend_state != Unavailable`, on macOS it picks `MacosTaskpolicyBackend` under the same rule, and otherwise falls back to `PlainFallbackBackend`. So `scaler run` on a healthy Ubuntu host actually goes through `systemd-run`, not the plain fallback. The Linux backend launches the child as a transient `.service` (not `--scope`) via `systemd-run --user --pipe --wait --collect --quiet --unit=scaler-run-<pid>-<nanos>.service`, captures stdio through systemd-run's pipes, resolves the unit's `MainPID` via `systemctl --user show` for sampling, and forwards signals via `systemctl --user kill --kill-whom=all`.
- `execute(plan, backend)` is the only public driver. It clears the execution trace, calls `backend.detect()`, decides `SelectedExecution` (IO mode + TUI vs plain + compact flag) via `select_execution`, then loops on `try_wait` / output drain / sample tick / interrupt escalation. **`core` is the single owner of final-summary timing** — the summary is printed only after `ui.restore_once()`.
- IO mode selection: `InteractiveMode::Always` requires `script` on PATH and a usable platform, otherwise fails before launch; `Never` always uses pipes; `Auto` chooses PTY only when **all three** standard streams are terminals **and** PTY is available. PTY mode launches via `script` (Linux uses `-q -e -c <cmd> /dev/null`; macOS uses `-q /dev/null <cmd...>`).
- UI selection: `use_tui = plan.resource_spec.monitor && all_terminals`; the `compact` flag is set iff PTY mode is active. `UiSession::start` first tries `TuiRenderer`; on failure it records `monitor_unavailable` and downgrades to `PlainRenderer` with an extra warning. After launch, a TUI render error triggers `handle_runtime_failure`, which restores the terminal, builds a `PlainRenderer`, **replays the already-rendered frames**, and continues. Plain renderer errors are not recoverable.
- Signal escalation: `install_signal_bridge()` installs a process-global ctrlc handler exactly once (guarded by `OnceLock`) that flips `INTERRUPT_REQUESTED` only when `SIGNAL_BRIDGE_ACTIVE > 0`. The escalation timing (`SIGINT` → 2s `SIGTERM` → 5s `SIGKILL`) lives in `InterruptPlan::default()`. `PlainFallbackBackend::terminate` shells out to `kill -<SIG> -- -<pid>` to hit the whole process group, which is why `build_local_command` calls `command.process_group(0)` on unix.
- Output is read on dedicated reader threads (`spawn_reader_thread`) into a per-process `OutputCollector`. Frames are appended to both a process-local `pending_frames` queue and the global `execution_trace` (so tests can inspect them). The run loop drains `pending_frames` each tick, and `finalize_process_output` waits for reader threads to drain before reporting exit. Per-stream byte order is preserved; cross-stream global ordering is **not** guaranteed beyond observed arrival.
- The exit code returned by `lib.rs::resolved_exit_code` maps signal-terminated children to `128 + signal` on unix.

### Test seams in `run_loop.rs`

Several `*_for_test` and `set_test_*_for_next_run` helpers exist so integration tests can drive the loop deterministically: poll interval, interrupt plan, terminal-state spoofing, monitor start failure, monitor-fail-after-N-draws, and an `execution_trace` of named events (`launch`, `restore_terminal`, `render_summary`, `monitor_unavailable`, `tui_renderer_active`, etc.). When adding new lifecycle behavior, prefer recording an event with `record_event(...)` and asserting on it from a test rather than introducing new public state. Always call `clear_runtime_overrides()` (via the loop's normal exit path or `reset_test_state()`) so overrides do not leak across tests.

### Backends (`src/backend/`)

`Backend` is a trait with `detect / launch / try_wait / sample / terminate`. `detect_host_capabilities()` is `cfg`-gated per OS — Linux probes `systemd-run`, cgroup v2, and the user manager; macOS probes `taskpolicy`, `renice`, memory support, PTY support, and platform version. Both backends own argv builders (`build_systemd_run_argv`, `build_taskpolicy_argv`) that are unit-tested in `tests/backend_linux.rs` / `tests/backend_macos.rs`. The Linux mapping enforces the design rule that `--mem X` becomes `MemoryMax=X` plus `MemoryHigh=90%·X` plus `MemorySwapMax=0`.

### UI (`src/ui/`)

`Renderer` trait with two implementations: `plain::PlainRenderer` (line-oriented streaming) and `tui::TuiRenderer` (ratatui/crossterm full-screen with a compact mode for PTY children). Both consume `MonitorSnapshot` and `OutputFrame` and never touch process state. `TuiRenderer::finish` (in `src/ui/tui.rs`) replays its captured `state.output` buffer to real stdout after `LeaveAlternateScreen`, so the user sees command output in scrollback (the buffer is trimmed to the tail at 32 KiB — for full streaming use the plain renderer). `format_bytes` / `format_duration` live in `src/core/summary.rs` and `ui` re-exports them as `format_bytes` / `format_elapsed` — reuse them rather than reimplementing.

### Doctor output

`cli::render_doctor_output` is the canonical formatter and the contract is order-sensitive: core capability lines first (`platform`, `backend`, `backend_state`, `cpu`, `memory`, `interactive`), prerequisite lines in their **declared** order, then warning lines **sorted**. Tests in `tests/doctor_cli.rs` lock this in.

## Testing layout

- `src/**/*.rs`: inline `#[cfg(test)]` unit tests live alongside the code.
- `tests/`: each file is an independent integration crate.
  - `cli_parse.rs`, `doctor_cli.rs`, `version_cli.rs`: end-to-end CLI behavior via `assert_cmd`.
  - `backend_linux.rs`, `backend_macos.rs`, `backend_contracts.rs`: argv builders and capability classification.
  - `core_contracts.rs`: type-level invariants.
  - `run_loop.rs`, `signal_handling.rs`: drive `core::run_loop::execute` against `PlainFallbackBackend` using the test seams above.

When changing run-loop behavior, prefer extending `run_loop.rs` / `signal_handling.rs` with a new test that asserts on the `execution_trace` rather than scraping stdout.

## Specs and plans

If `docs/superpowers/` is present locally (it is gitignored), `specs/2026-04-02-scaler-v1-design.md` is the authoritative behavioral contract for v1 (CLI grammar, capability vocabulary, lifecycle, doctor output, signal escalation, monitor fallback) and `plans/2026-04-02-scaler-v1.md` is the implementation plan it was built from. Without those notes the README and the existing tests are the next-best contract.
