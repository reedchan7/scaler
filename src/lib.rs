pub mod backend;
pub mod cli;
pub mod core;

pub fn run() -> anyhow::Result<()> {
    let cli = match crate::cli::parse_from(std::env::args_os().collect()) {
        Ok(cli) => cli,
        Err(err) => match err.downcast::<clap::Error>() {
            Ok(clap_err) => {
                if matches!(
                    clap_err.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) {
                    clap_err.print()?;
                    return Ok(());
                }

                clap_err.print()?;
                std::process::exit(clap_err.exit_code());
            }
            Err(err) => return Err(err),
        },
    };

    match cli.command {
        crate::cli::args::Command::Doctor => {
            let report = crate::backend::detect_host_capabilities();
            println!("{}", crate::cli::render_doctor_output(&report));
            Ok(())
        }
        crate::cli::args::Command::Run(run) => {
            let plan = build_launch_plan(run);
            let backend = crate::core::run_loop::PlainFallbackBackend::default();
            let _signal_bridge = crate::core::run_loop::install_signal_bridge()?;
            let outcome = crate::core::run_loop::execute(plan, &backend)?;
            println!("{}", crate::core::summary::render(&outcome));
            if let Some(exit_code) = resolved_exit_code(&outcome.exit_status) {
                if exit_code != 0 {
                    std::process::exit(exit_code);
                }
            }
            Ok(())
        }
        crate::cli::args::Command::Version => {
            println!(
                "scaler {} {}-{}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            Ok(())
        }
    }
}

fn build_launch_plan(run: crate::cli::args::RunCommand) -> crate::core::LaunchPlan {
    crate::core::LaunchPlan {
        argv: run
            .trailing
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect(),
        resource_spec: crate::core::ResourceSpec {
            cpu: run.cpu,
            mem: run.mem,
            interactive: match run.interactive {
                crate::cli::args::InteractiveModeArg::Auto => crate::core::InteractiveMode::Auto,
                crate::cli::args::InteractiveModeArg::Always => {
                    crate::core::InteractiveMode::Always
                }
                crate::cli::args::InteractiveModeArg::Never => crate::core::InteractiveMode::Never,
            },
            shell: run.shell.map(|shell| match shell {
                crate::cli::args::ShellArg::Sh => crate::core::ShellKind::Sh,
                crate::cli::args::ShellArg::Bash => crate::core::ShellKind::Bash,
                crate::cli::args::ShellArg::Zsh => crate::core::ShellKind::Zsh,
            }),
            monitor: run.monitor,
        },
        platform: current_platform(),
    }
}

fn current_platform() -> crate::core::Platform {
    match std::env::consts::OS {
        "linux" => crate::core::Platform::Linux,
        "macos" => crate::core::Platform::Macos,
        _ => crate::core::Platform::Unsupported,
    }
}

fn resolved_exit_code(status: &std::process::ExitStatus) -> Option<i32> {
    if let Some(code) = status.code() {
        return Some(code);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        return status.signal().map(|signal| 128 + signal);
    }

    #[cfg(not(unix))]
    {
        None
    }
}
