pub mod backend;
pub mod cli;
pub mod core;
pub mod detach;
pub mod ui;

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
            let effective = crate::backend::effective_backend_kind();
            println!("{}", crate::cli::render_doctor_output(&report, effective));
            Ok(())
        }
        crate::cli::args::Command::Run(run) => {
            let plan = build_launch_plan(run);
            let effective = crate::backend::effective_backend_kind();
            warn_if_resource_limits_will_be_dropped(&plan, effective);
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
        crate::cli::args::Command::Version => {
            println!(
                "scaler {} {}-{}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            Ok(())
        }
        crate::cli::args::Command::Status(_) => {
            anyhow::bail!("`scaler status` not wired yet (Task 11)");
        }
        crate::cli::args::Command::Finalize { id } => {
            #[cfg(target_os = "linux")]
            {
                crate::detach::linux::finalize(&id)?;
                return Ok(());
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = id;
                anyhow::bail!("__finalize is only used on Linux");
            }
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

fn warn_if_resource_limits_will_be_dropped(
    plan: &crate::core::LaunchPlan,
    effective: crate::core::BackendKind,
) {
    let asked_for_limits = plan.resource_spec.cpu.is_some() || plan.resource_spec.mem.is_some();
    if !asked_for_limits {
        return;
    }
    if effective == crate::core::BackendKind::PlainFallback {
        eprintln!(
            "scaler: resource limits NOT being enforced on this host; run `scaler doctor` for details"
        );
    }
}

fn resolved_exit_code(status: &std::process::ExitStatus) -> Option<i32> {
    if let Some(code) = status.code() {
        return Some(code);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        status.signal().map(|signal| 128 + signal)
    }

    #[cfg(not(unix))]
    {
        None
    }
}
