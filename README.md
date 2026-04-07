# scaler

[![CI](https://github.com/reedchan7/scaler/actions/workflows/ci.yml/badge.svg)](https://github.com/reedchan7/scaler/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/reedchan7/scaler?display_name=tag&sort=semver)](https://github.com/reedchan7/scaler/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
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
| `--no-monitor` | — | Disable the live TUI dashboard. |

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

## Limitations

- macOS limits are **best-effort only** — `taskpolicy` lowers scheduling priority but cannot hard-cap CPU or memory. `scaler doctor` reports this honestly so you don't get a false sense of safety.
- TUI mode buffers child output to a 64 KiB tail; for full streaming of long-running jobs, pass `--no-monitor` or pipe through a non-TTY.
- Windows is not supported.

## License

[MIT](LICENSE) © reedchan7

## Contributing

Issues and pull requests are welcome at <https://github.com/reedchan7/scaler/issues>.
