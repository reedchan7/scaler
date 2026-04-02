pub mod cli;
pub mod core;

pub fn run() -> anyhow::Result<()> {
    let cli = crate::cli::parse_from(std::env::args_os().collect())?;

    match cli.command {
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
