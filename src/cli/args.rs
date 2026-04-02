use clap::{ArgAction, CommandFactory, Parser, error::ErrorKind};

use crate::cli::values::{CpuLimit, MemoryLimit};

#[derive(Parser, Debug)]
#[command(name = "scaler", version)]
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
    Run(RunCommand),
    Doctor,
    Version,
}

#[derive(clap::Args, Debug)]
pub struct RunCommand {
    #[arg(long, value_parser = crate::cli::values::parse_cpu_limit)]
    pub cpu: Option<CpuLimit>,

    #[arg(long, value_parser = crate::cli::values::parse_memory_limit)]
    pub mem: Option<MemoryLimit>,

    #[arg(long, value_enum, default_value_t = InteractiveModeArg::Auto)]
    pub interactive: InteractiveModeArg,

    #[arg(long, value_enum)]
    pub shell: Option<ShellArg>,

    #[arg(long = "no-monitor", default_value_t = true, action = ArgAction::SetFalse)]
    pub monitor: bool,

    #[arg(last = true)]
    pub trailing: Vec<String>,
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
