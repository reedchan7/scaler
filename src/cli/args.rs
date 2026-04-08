use clap::{ArgAction, CommandFactory, Parser, error::ErrorKind};

use crate::cli::values::{CpuLimit, MemoryLimit};

/// Top-level long_about kept as a string constant so the derive macro
/// stays readable. Pulled in via `long_about = CLI_LONG_ABOUT`.
const CLI_LONG_ABOUT: &str = "\
Run any command with normalized CPU and memory limits.

scaler wraps a command with normalized resource flags and a transient \
enforcement scope so heavy work runs gently and visibly instead of locking \
up the host. On Linux it spawns the target inside a `systemd-run --user --scope` \
unit with `CPUQuota=` and `MemoryMax=` set; on macOS it falls back to \
`taskpolicy` for best-effort throttling.

Quick examples:
  scaler --cpu 0.5c --mem 1g -- npm install
  scaler --shell sh -- 'find . -name \"*.log\" | xargs gzip'
  scaler -- make build               (no limits, just record stats)
  scaler doctor                      (check enforcement on this host)

Run `scaler run --help` for the full flag reference.";

const RUN_AFTER_LONG_HELP: &str = "\
Examples:
  # Half a core, 1 GiB memory; real cgroup v2 limits on Linux
  scaler run --cpu 0.5c --mem 1g -- npm install

  # Same thing, shorthand (the `run` is implicit when `--` is present)
  scaler --cpu 0.5c --mem 1g -- npm install

  # Inline shell script — must be exactly one quoted token after `--`
  scaler --shell sh -- 'find . -name \"*.log\" | xargs gzip'

  # No limits, just record elapsed / peak memory / CPU usage
  scaler -- make build

  # Live TUI dashboard for long-running jobs
  scaler --monitor --cpu 2c -- cargo build --release

Detached examples:
  scaler run --cpu 0.8c --mem 600m -d -- npm install --jobs=1
      Launch in the background, print a run id, return immediately.

  scaler status
      List all runs (newest first).

  scaler status 20260408-143022-a1b2
      Show detail for one run (accepts unique id prefixes).

  scaler status --json
      Machine-readable output for scripting.

Detached notes:
  - Linux uses `systemd-run --no-block`; the unit continues after scaler exits.
  - macOS uses double-fork; a grandchild process runs the command.
  - State lives under $XDG_STATE_HOME/scaler/runs/ (default ~/.local/state/scaler/runs/).
  - No automatic cleanup; remove stale runs with:
      find ~/.local/state/scaler/runs -mindepth 1 -maxdepth 1 -mtime +30 -exec rm -rf {} +
  - To kill a running detached run:
      systemctl --user stop scaler-run-<id>.service   # Linux
      kill $(jq -r .pid ~/.local/state/scaler/runs/<id>/meta.json)   # macOS
  - scaler does not limit disk I/O; I/O-bound workloads on small hosts may still saturate.
  - Detached mode cannot combine with --monitor or --interactive always.

See `scaler doctor` to check what limits your host can actually enforce.";

#[derive(Parser, Debug)]
#[command(
    name = "scaler",
    version,
    about = "Run any command with normalized CPU and memory limits.",
    long_about = CLI_LONG_ABOUT,
    after_help = "See `scaler doctor` to check what limits your host can actually enforce.",
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveModeArg {
    Auto,
    Always,
    Never,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellArg {
    Sh,
    Bash,
    Zsh,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Run a command under scaler with optional CPU and memory limits.
    Run(RunCommand),

    /// Print a deterministic capability report for the current host.
    #[command(long_about = "\
Print a deterministic capability report for the current host. Output line \
ordering is stable: capability lines first (`platform`, `backend`, `backend_state`, \
`cpu`, `memory`, `interactive`, `effective_backend`), then prerequisite lines \
in their declared order, then warning lines sorted alphabetically. Use this \
to confirm whether `scaler run --cpu` and `--mem` will be enforced or merely \
best-effort on this host.")]
    Doctor,

    /// Print scaler version with target triple.
    #[command(long_about = "\
Print the scaler version and the OS/architecture target triple this binary \
was built for, e.g. `scaler 0.4.0 macos-aarch64`. Useful when filing bug \
reports.")]
    Version,

    /// List detached runs or show detail for a single run.
    Status(StatusCommand),

    /// Internal finalize hook invoked by systemd ExecStopPost. Not for human use.
    #[command(hide = true, name = "__finalize")]
    Finalize {
        /// Run id whose result.json should be written.
        id: String,
    },
}

#[derive(clap::Args, Debug)]
#[command(
    after_help = "See `scaler run --help` for usage examples.",
    after_long_help = RUN_AFTER_LONG_HELP,
)]
pub struct RunCommand {
    /// CPU budget in logical cores: `1c`, `0.5c`, `0.25c`. Minimum `0.01c`.
    /// On Linux this maps to `CPUQuota=`. On macOS it lowers scheduling
    /// priority via `taskpolicy` (best-effort, not a hard cap).
    #[arg(long, value_parser = crate::cli::values::parse_cpu_limit)]
    pub cpu: Option<CpuLimit>,

    /// Memory budget in 1024-based units: `1g`, `512m`, `1.5g`. Minimum `1m`.
    /// On Linux this maps to `MemoryMax=` plus `MemoryHigh=` at 90 % plus
    /// `MemorySwapMax=0`. On macOS it is best-effort (`taskpolicy -m <mib>`
    /// when supported).
    #[arg(long, value_parser = crate::cli::values::parse_memory_limit)]
    pub mem: Option<MemoryLimit>,

    /// Force PTY (`always`) or pipe (`never`) IO mode. `auto` (the default)
    /// picks PTY only when stdin/stdout/stderr are all terminals.
    #[arg(long, value_enum, default_value_t = InteractiveModeArg::Auto)]
    pub interactive: InteractiveModeArg,

    /// Wrap a single inline script with the chosen shell. Requires exactly
    /// one quoted token after `--`.
    #[arg(long, value_enum)]
    pub shell: Option<ShellArg>,

    /// Enable the live TUI dashboard (opt-in). Without this flag, scaler
    /// streams command output plainly and prints the summary card at the end.
    #[arg(long = "monitor", default_value_t = false, action = ArgAction::SetTrue)]
    pub monitor: bool,

    /// Run the command in the background and return a run id immediately.
    /// Use `scaler status <id>` to check progress. Incompatible with
    /// `--monitor` and `--interactive always`.
    #[arg(long, short = 'd', default_value_t = false, action = ArgAction::SetTrue)]
    pub detach: bool,

    #[arg(last = true)]
    pub trailing: Vec<String>,
}

#[derive(clap::Args, Debug)]
#[command(
    about = "List detached runs or show detail for a single run.",
    long_about = "\
List all detached runs under $XDG_STATE_HOME/scaler/runs/, or show detail \
for a single run by id (exact match or unique prefix). Runs that are still \
live are queried from systemd (Linux) or ps (macOS) on demand."
)]
pub struct StatusCommand {
    /// Show detail for this run id (exact match or unique prefix).
    pub id: Option<String>,

    /// Machine-readable JSON output.
    #[arg(long, default_value_t = false, action = ArgAction::SetTrue)]
    pub json: bool,
}

impl Cli {
    pub fn try_parse_from<I, T>(itr: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let cli = <Self as Parser>::try_parse_from(itr)?;
        cli.validate()?;
        Ok(cli)
    }

    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Run(_) => "run",
            Command::Doctor => "doctor",
            Command::Version => "version",
            Command::Status(_) => "status",
            Command::Finalize { .. } => "__finalize",
        }
    }

    fn validate(&self) -> Result<(), clap::Error> {
        if let Command::Run(run) = &self.command {
            run.validate()?;
        }

        Ok(())
    }
}

impl RunCommand {
    fn validate(&self) -> Result<(), clap::Error> {
        if self.detach && self.monitor {
            return Err(validation_error(
                ErrorKind::ArgumentConflict,
                "--detach cannot combine with --monitor",
            ));
        }
        if self.detach && matches!(self.interactive, InteractiveModeArg::Always) {
            return Err(validation_error(
                ErrorKind::ArgumentConflict,
                "--detach cannot combine with --interactive always",
            ));
        }
        match self.shell {
            Some(_) if self.trailing.len() != 1 => Err(validation_error(
                ErrorKind::WrongNumberOfValues,
                "shell mode requires exactly one script token after `--`",
            )),
            None if self.trailing.is_empty() => Err(validation_error(
                ErrorKind::MissingRequiredArgument,
                "run requires at least one command token after `--`",
            )),
            _ => Ok(()),
        }
    }
}

fn validation_error(kind: ErrorKind, message: &str) -> clap::Error {
    let mut command = Cli::command();
    command
        .find_subcommand_mut("run")
        .expect("run subcommand must exist")
        .error(kind, message)
}
