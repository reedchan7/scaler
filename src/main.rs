fn main() {
    if let Err(err) = scaler::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
