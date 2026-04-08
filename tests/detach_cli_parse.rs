use scaler::cli::args::{Cli, Command};

fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("scaler").chain(argv.iter().copied()))
}

#[test]
fn run_accepts_detach_flag() {
    let cli = parse(&["run", "--detach", "--", "echo", "hi"]).unwrap();
    let Command::Run(run) = cli.command else {
        panic!("expected Run")
    };
    assert!(run.detach);
}

#[test]
fn run_accepts_detach_short_flag() {
    let cli = parse(&["run", "-d", "--", "echo", "hi"]).unwrap();
    let Command::Run(run) = cli.command else {
        panic!("expected Run")
    };
    assert!(run.detach);
}

#[test]
fn run_detach_defaults_false() {
    let cli = parse(&["run", "--", "echo", "hi"]).unwrap();
    let Command::Run(run) = cli.command else {
        panic!("expected Run")
    };
    assert!(!run.detach);
}

#[test]
fn run_detach_plus_interactive_always_errors() {
    let err = parse(&["run", "--detach", "--interactive", "always", "--", "echo"])
        .expect_err("should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("--detach") && rendered.contains("--interactive always"),
        "error was: {rendered}"
    );
}

#[test]
fn run_detach_plus_monitor_errors() {
    let err = parse(&["run", "--detach", "--monitor", "--", "echo"]).expect_err("should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("--detach") && rendered.contains("--monitor"),
        "error was: {rendered}"
    );
}

#[test]
fn run_detach_plus_interactive_auto_is_fine() {
    parse(&["run", "--detach", "--interactive", "auto", "--", "echo"]).unwrap();
}

#[test]
fn status_subcommand_no_id_lists_all() {
    let cli = parse(&["status"]).unwrap();
    let Command::Status(s) = cli.command else {
        panic!("expected Status")
    };
    assert!(s.id.is_none());
    assert!(!s.json);
}

#[test]
fn status_subcommand_with_id() {
    let cli = parse(&["status", "20260408-143022-a1b2"]).unwrap();
    let Command::Status(s) = cli.command else {
        panic!("expected Status")
    };
    assert_eq!(s.id.as_deref(), Some("20260408-143022-a1b2"));
}

#[test]
fn status_subcommand_with_json() {
    let cli = parse(&["status", "--json"]).unwrap();
    let Command::Status(s) = cli.command else {
        panic!("expected Status")
    };
    assert!(s.json);
}

#[test]
fn finalize_hidden_subcommand_parses() {
    let cli = parse(&["__finalize", "20260408-143022-a1b2"]).unwrap();
    let Command::Finalize { id } = cli.command else {
        panic!("expected Finalize")
    };
    assert_eq!(id, "20260408-143022-a1b2");
}
