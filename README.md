# scaler

[![CI](https://github.com/reedchan7/scaler/actions/workflows/ci.yml/badge.svg)](https://github.com/reedchan7/scaler/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macOS-lightgrey)](#how-limits-are-enforced)
[![Rust](https://img.shields.io/badge/rust-2024-orange?logo=rust)](https://www.rust-lang.org/)

> Run any command with CPU and memory caps. Real cgroup v2 limits on Linux, best-effort scheduling on macOS, one CLI.

`scaler` wraps a command with normalized resource flags and a transient enforcement scope so heavy work runs *gently and visibly* instead of locking up the host. On Linux it spawns the target inside a `systemd-run --user --scope` unit with `CPUQuota=` and `MemoryMax=` set; on macOS it falls back to `taskpolicy` for best-effort throttling. Either way the same `--cpu 0.5c --mem 512m` flags work, and `scaler doctor` tells you exactly which guarantees apply on the current host.

## Why

You ran `npm install` and your laptop became a brick. A misbehaving script blew through 16 GB of RAM and the OOM killer ate your editor. A nightly job hogs every core and your SSH session times out. `scaler` is for those situations:

```bash
scaler run --cpu 0.5c --mem 1g -- npm install -g some-heavy-package
```

The install runs on half a core, stops growing at 1 GB resident, and you can still type.

## Install

### From a release tarball

```bash
VERSION=v0.3.0
TARGET=x86_64-unknown-linux-gnu   # pick from the table below
curl -fsSL "https://github.com/reedchan7/scaler/releases/download/${VERSION}/scaler-${VERSION}-${TARGET}.tar.gz" \
  | tar -xz -C /tmp
sudo install -m 0755 "/tmp/scaler-${VERSION}-${TARGET}/scaler" /usr/local/bin/scaler
scaler doctor
```

| Host | `TARGET` |
| --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Linux ARM64 (aarch64) | `aarch64-unknown-linux-gnu` |
| macOS Apple Silicon | `aarch64-apple-darwin` |

### From source

Requires Rust stable (edition 2024).

```bash
git clone https://github.com/reedchan7/scaler.git
cd scaler
make install                  # builds release + copies to /usr/local/bin (uses sudo if needed)
# or:
PREFIX=~/.local make install  # installs to $HOME/.local/bin without sudo
# or:
cargo build --release         # plain cargo build, binary at target/release/scaler
```

## Usage

Two equivalent forms:

```bash
# Explicit subcommand
scaler run [FLAGS] -- <program> [args...]

# Shorthand (the `run` is implicit)
scaler [FLAGS] -- <program> [args...]
```

### Resource flags

| Flag | Example | Meaning |
| --- | --- | --- |
| `--cpu` | `1c`, `0.5c`, `0.25c` | Logical CPU budget. `1c` = one full core, `0.5c` = half a core. |
| `--mem` | `1g`, `512m`, `1.5g` | Memory budget. Units: `b k m g t` (1024-based). Minimum `1m`. |
| `--interactive` | `auto` (default), `always`, `never` | Force PTY or pipe mode. `auto` picks PTY only when stdin/stdout/stderr are all terminals. |
| `--shell` | `sh`, `bash`, `zsh` | Wrap a single inline script with the chosen shell. |
| `--monitor` | — | Opt in to the live TUI dashboard (default: plain streaming). |

### Examples

```bash
# Direct command (no shell)
scaler run --cpu 1c --mem 1g -- npm install
scaler --cpu 0.5c --mem 256m -- python3 train.py --epochs 50

# Inline shell script (must be exactly one quoted token after `--`)
scaler --shell sh -- 'find . -name "*.log" | xargs gzip'

# Interactive program inside the limited scope
scaler --interactive always -- htop

# No limits, just record elapsed time and peak memory
scaler -- make build
```

### Direct command vs shell mode

`scaler -- echo '$HOME'` passes the literal string `$HOME` to `echo`. To get shell expansion, pipes, redirects, or compound commands, use `--shell`:

```bash
scaler --shell sh -- 'echo $HOME && ls | wc -l'
```

Shell mode requires exactly one script token after `--`. Direct mode requires the `--` delimiter so dash-prefixed programs and flags pass through unambiguously.

### Output streams

`scaler` keeps the wrapped command's stdout clean so pipelines stay correct. Everything scaler emits goes to stderr:

| Stream | Contents |
| --- | --- |
| **stdout** | only the wrapped command's stdout |
| **stderr** | scaler's capability banner, the wrapped command's stderr, and the run summary |

```bash
$ scaler run --cpu 0.5c -- echo hello > out.txt
[best-effort] backend: macos_taskpolicy
[best-effort] cpu: best_effort
[best-effort] memory: best_effort
[best-effort] interactive: best_effort

── scaler ─────────────────────────────
┌─────────────── scaler summary ────────────────┐
│  exit     0                                   │
│  elapsed  1.845s                              │
│  memory   max 26.4 MiB (10.3%)                │
│  cpu      avg 0.19c (18.8%), max 0.25c (25.0%)│
└───────────────────────────────────────────────┘

$ cat out.txt
hello
```

## How limits are enforced

| Platform | Backend | CPU | Memory | Guarantee |
| --- | --- | --- | --- | --- |
| **Linux** (cgroup v2 + systemd) | `systemd-run --user --scope` | `CPUQuota=` (hard) | `MemoryMax=` (hard, OOM kill) + `MemoryHigh=` at 90 % (slow under reclaim) + `MemorySwapMax=0` | **enforced** |
| **macOS** (≥ 11) | `taskpolicy -b -d throttle -g default` | scheduling priority lowered (THROTTLE class) | `-m <mib>` if `taskpolicy` supports it, else dropped | **best-effort** |
| Anywhere else | plain spawn fallback | none | none | unenforced (warns to stderr) |

Linux enforcement requires:

- `systemd-run` on `PATH`
- unified cgroup v2 mounted at `/sys/fs/cgroup/cgroup.controllers`
- a reachable user systemd manager (`systemctl --user` works)

If `scaler doctor` reports `effective_backend: plain_fallback` on a Linux host, your user systemd manager isn't running. Fix it with `sudo loginctl enable-linger "$USER"`, log out and back in, then re-run `scaler doctor`.

## `scaler doctor`

Prints a deterministic capability report for the current host:

```text
platform: linux
backend: linux_systemd
backend_state: enforced
cpu: enforced
memory: enforced
interactive: enforced
effective_backend: linux_systemd
prerequisite: systemd_run=ok
prerequisite: cgroup_v2=ok
prerequisite: user_manager=ok
```

Line ordering is stable: capability lines first, then prerequisite lines in declared order, then sorted warning lines. The `effective_backend:` line names the backend `scaler run` will actually pick — `linux_systemd`, `macos_taskpolicy`, or `plain_fallback`. If the wrapped command requested `--cpu` or `--mem` and the effective backend is `plain_fallback`, scaler also prints a warning to stderr before launch.

## Verify enforcement on Linux

Once `scaler doctor` reports `effective_backend: linux_systemd`, you can confirm the cgroup is real:

```bash
scaler run --cpu 0.5c --mem 256m -- bash -c '
  unit=$(cat /proc/self/cgroup | cut -d: -f3 | sed "s|^/||" | xargs basename)
  systemctl --user show -p CPUQuotaPerSecUSec -p MemoryMax -p MemorySwapMax "$unit"
'
```

Expected output:

```
CPUQuotaPerSecUSec=500ms
MemoryMax=268435456
MemorySwapMax=0
```

## Development

```bash
make build       # cargo build --release
make test        # cargo test
make check       # fmt + clippy + test (the local CI quartet)
make doctor      # build + run scaler doctor against the new binary
make clean       # cargo clean

# Bump the crate version (yarn-style)
make version                  # patch +1 (default, e.g. 0.2.0 → 0.2.1)
make version BUMP=minor       # minor +1 (e.g. 0.2.0 → 0.3.0)
make version BUMP=major       # major +1 (e.g. 0.2.0 → 1.0.0)
make version VERSION=1.2.3    # set an explicit version
```

CI runs `cargo fmt -- --check`, `cargo clippy --tests -- -D warnings`, `cargo test`, and `cargo build --release` on Linux x86_64, Linux ARM64, and macOS Apple Silicon. Tag a `vX.Y.Z` to ship a release; the workflow validates the tag matches `Cargo.toml`, builds the three target tarballs, generates checksums, and uploads everything to the matching GitHub Release.

## Detached runs

Long commands can be launched in the background and queried later. `scaler` does **not** become a daemon itself — on Linux the transient `systemd-run` unit is the supervisor, on macOS a double-forked grandchild process runs the command.

```sh
# Launch and return immediately. Prints the run id.
scaler run --cpu 0.8c --mem 600m --detach -- npm install --jobs=1
20260408-143022-a1b2

# List all runs (newest first).
scaler status
20260408-143022-a1b2  exited(0)  52m18s  npm install --jobs=1

# Detail for one run (exact id or unique prefix).
scaler status 20260408-143022
id:       20260408-143022-a1b2
command:  npm install --jobs=1
limits:   cpu=0.80c  mem=600 MiB
backend:  linux_systemd (enforced)
started:  2026-04-08T14:30:22+08:00
ended:    2026-04-08T15:22:40+08:00
state:    exited(0)
memory:   peak 587 MiB
stdout:   ~/.local/state/scaler/runs/20260408-143022-a1b2/stdout.log
stderr:   ~/.local/state/scaler/runs/20260408-143022-a1b2/stderr.log

# Machine-readable output.
scaler status --json
```

### State directory

Runs are stored under `$XDG_STATE_HOME/scaler/runs/` (default `~/.local/state/scaler/runs/` on both Linux and macOS). Each run has its own directory with `meta.json`, `result.json` (after exit), `stdout.log`, and `stderr.log`.

There is no automatic cleanup. Remove stale runs manually:

```sh
find ~/.local/state/scaler/runs -mindepth 1 -maxdepth 1 -mtime +30 -exec rm -rf {} +
```

### Killing a detached run

`scaler` does not provide a `kill` subcommand in v1. Use platform tools:

- **Linux:** `systemctl --user stop scaler-run-<id>.service`
- **macOS:** `pkill -P $(jq -r .pid ~/.local/state/scaler/runs/<id>/meta.json)`
  (kills the wrapped command — `meta.pid` is the scaler grandchild that
  supervises it; killing the grandchild directly would leave the run in
  the `gone` state with no `result.json`.)

### Detached limitations

- `scaler` does not limit disk I/O. On small hosts (e.g. 2c/2g VMs) a command that is I/O-bound (like `npm install`) can still saturate the system even when CPU and memory caps are enforced.
- No push notifications. `scaler status` is pull-only; layer your own notifier on top if needed.
- No crash recovery. If `scaler` or `systemd` dies between "service queued" and "`result.json` written", the run shows as `gone`. Check `stdout.log` / `stderr.log` to piece it together.
- Paths containing spaces (e.g. a home directory with a space) are not escaped in systemd property strings; avoid spaces in `$HOME` on Linux if you use `--detach`.
- `--detach` cannot combine with `--monitor` or `--interactive always`.

## Limitations

- macOS limits are **best-effort only** — `taskpolicy` lowers scheduling priority but cannot hard-cap CPU or memory. `scaler doctor` reports this honestly so you don't get a false sense of safety.
- The live TUI dashboard is **opt-in**: pass `--monitor` for a ratatui-based card that shows CPU / memory / elapsed in real time. Without it, scaler streams the command output plainly and prints the summary card at the end.
- When `--monitor` is active, the TUI buffers child output to a 64 KiB tail per stream; for full streaming of long-running jobs, just omit `--monitor`.
- Windows is not supported.

## License

[MIT](LICENSE) © reedchan7

## Contributing

Issues and pull requests are welcome at <https://github.com/reedchan7/scaler/issues>.
