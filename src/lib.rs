pub fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    if matches!(args.next().as_deref(), Some("version")) {
        println!(
            "scaler {} {}-{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        return Ok(());
    }

    anyhow::bail!("command not implemented yet")
}
