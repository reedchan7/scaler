# scaler

`scaler` is a small CLI for launching a command with normalized CPU/memory flags, deterministic capability reporting, and a compact monitor-oriented run flow.

## Supported command forms

Explicit subcommand form:

```bash
scaler run -- <command> [args...]
scaler run --cpu 1c --mem 1g --interactive always -- <command> [args...]
scaler run --shell sh -- 'echo hello from a shell script'
```

Shorthand direct-command form:

```bash
scaler -- <command> [args...]
scaler --cpu 1c --mem 1g -- <command> [args...]
scaler --interactive never -- <command> [args...]
```

Administrative commands:

```bash
scaler doctor
scaler version
```

Direct command mode does not insert a shell. `scaler -- echo '$HOME'` passes the literal `$HOME` argument to `echo`. Use `--shell <sh|bash|zsh>` when you want shell parsing, expansion, pipes, redirects, or compound commands.

## Semantics

Linux is modeled as an enforced backend. `doctor` reports `enforced` only when all Linux prerequisites are ready:

- `systemd-run` is available
- unified cgroup v2 is available
- the user systemd manager is reachable

If any Linux prerequisite is missing, `doctor` marks `backend_state`, `cpu`, `memory`, and `interactive` as `unavailable`, and prints prerequisite-specific warnings.

macOS is modeled as a best-effort backend. When `taskpolicy` and a supported platform version are available, `doctor` reports `best_effort` for the backend and CPU control. Memory and interactive support may degrade independently, with warnings for missing `renice`, missing taskpolicy memory support, or missing PTY support for forced interactive mode.

## Sample `doctor` output

```text
platform: macos
backend: macos_taskpolicy
backend_state: best_effort
cpu: best_effort
memory: best_effort
interactive: best_effort
prerequisite: taskpolicy=ok
prerequisite: platform_version=ok
```

The output format is deterministic:

- core capability lines always come first
- prerequisite lines keep a fixed declared order
- warning lines are sorted

## Sample `run` invocations

Run a direct command without a shell:

```bash
scaler run -- python3 -c 'print("hello")'
```

Use the shorthand form with resource flags:

```bash
scaler --cpu 0.5c --mem 512m --interactive never -- /usr/bin/env true
```

Run exactly one shell script token:

```bash
scaler run --shell bash -- 'echo start && sleep 1 && echo done'
```

Shell mode requires exactly one script token after `--`. Direct command mode requires the `--` delimiter before the executable so dash-prefixed programs and flags are preserved without ambiguity.
