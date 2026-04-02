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
            println!("platform: {}", report.platform.as_str());
            println!("backend: {}", report.backend.as_str());
            println!("backend_state: {}", report.backend_state.as_str());
            println!("cpu: {}", report.cpu.as_str());
            println!("memory: {}", report.memory.as_str());
            println!("interactive: {}", report.interactive.as_str());

            if report.warnings.is_empty() {
                if report.platform == crate::core::Platform::Unsupported {
                    println!("prerequisite: no supported backend for this host");
                }
            } else {
                for warning in report.warnings {
                    println!("prerequisite: {warning}");
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
        _ => anyhow::bail!("command not implemented yet"),
    }
}
