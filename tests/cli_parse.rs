use clap::error::ErrorKind;
use scaler::cli::values::{CpuLimit, MemoryLimit};
use scaler::cli::{args::Cli, normalize_argv, parse_from};
use std::ffi::OsString;

#[test]
fn parses_decimal_cpu_limit() {
    assert_eq!(CpuLimit::parse("0.5c").unwrap().centi_cores(), 50);
}

#[test]
fn cpu_rounds_half_up_and_rejects_below_minimum() {
    assert_eq!(CpuLimit::parse(".005c").unwrap().centi_cores(), 1);
    assert!(CpuLimit::parse(".004c").is_err());
}

#[test]
fn rejects_zero_cpu_limit() {
    assert!(CpuLimit::parse("0c").is_err());
}

#[test]
fn parses_decimal_memory_limit() {
    assert_eq!(MemoryLimit::parse("1.5g").unwrap().bytes(), 1610612736);
}

#[test]
fn memory_units_are_case_insensitive_and_enforce_minimum() {
    assert_eq!(MemoryLimit::parse("1G").unwrap().bytes(), 1073741824);
    assert!(MemoryLimit::parse("0.5m").is_err());
}

#[test]
fn shorthand_run_rewrites_without_guessing_shell() {
    let cli = parse_from(vec![
        "scaler".into(),
        "--cpu".into(),
        "1c".into(),
        "--".into(),
        "echo".into(),
        "ok".into(),
    ])
    .unwrap();
    assert_eq!(cli.command_name(), "run");
}

#[test]
fn normalize_argv_only_rewrites_literal_delimiter_form() {
    assert_eq!(
        normalize_argv(vec![
            OsString::from("scaler"),
            OsString::from("--"),
            OsString::from("echo"),
            OsString::from("ok"),
        ]),
        vec![
            OsString::from("scaler"),
            OsString::from("run"),
            OsString::from("--"),
            OsString::from("echo"),
            OsString::from("ok"),
        ]
    );
    assert_eq!(
        normalize_argv(vec![OsString::from("scaler"), OsString::from("--help")]),
        vec![OsString::from("scaler"), OsString::from("--help")]
    );
    assert_eq!(
        normalize_argv(vec![OsString::from("scaler"), OsString::from("--version")]),
        vec![OsString::from("scaler"), OsString::from("--version")]
    );
    assert_eq!(
        normalize_argv(vec![OsString::from("scaler"), OsString::from("foo")]),
        vec![OsString::from("scaler"), OsString::from("foo")]
    );
}

#[test]
fn top_level_help_version_and_unknown_command_keep_clap_behavior() {
    let help = parse_from(vec!["scaler".into(), "--help".into()]).unwrap_err();
    assert_eq!(
        help.downcast_ref::<clap::Error>().unwrap().kind(),
        ErrorKind::DisplayHelp
    );

    let version = parse_from(vec!["scaler".into(), "--version".into()]).unwrap_err();
    assert_eq!(
        version.downcast_ref::<clap::Error>().unwrap().kind(),
        ErrorKind::DisplayVersion
    );

    let unknown = parse_from(vec!["scaler".into(), "foo".into()]).unwrap_err();
    assert_eq!(
        unknown.downcast_ref::<clap::Error>().unwrap().kind(),
        ErrorKind::InvalidSubcommand
    );
}

#[test]
fn direct_command_forms_require_the_delimiter() {
    let explicit = parse_from(vec![
        "scaler".into(),
        "run".into(),
        "--".into(),
        "echo".into(),
        "ok".into(),
    ])
    .unwrap();
    let shorthand = parse_from(vec![
        "scaler".into(),
        "--".into(),
        "echo".into(),
        "ok".into(),
    ])
    .unwrap();
    assert_eq!(explicit.command_name(), "run");
    assert_eq!(shorthand.command_name(), "run");
}

#[test]
fn delimiter_allows_dash_prefixed_executables() {
    let cli = parse_from(vec![
        "scaler".into(),
        "run".into(),
        "--".into(),
        "-tool".into(),
        "--flag".into(),
    ])
    .unwrap();
    assert_eq!(cli.command_name(), "run");
}

#[test]
fn parses_interactive_and_monitor_flags() {
    let cli = Cli::try_parse_from([
        "scaler",
        "run",
        "--interactive",
        "never",
        "--no-monitor",
        "--",
        "echo",
        "ok",
    ])
    .unwrap();
    assert_eq!(cli.command_name(), "run");
}

#[test]
fn rejects_invalid_interactive_values() {
    assert!(
        Cli::try_parse_from([
            "scaler",
            "run",
            "--interactive",
            "later",
            "--",
            "echo",
            "ok",
        ])
        .is_err()
    );
}

#[test]
fn rejects_invalid_resource_forms() {
    for raw in ["-1c", "0c", "1e3c", "1_0c", "999999999999999999999999c"] {
        assert!(CpuLimit::parse(raw).is_err(), "{raw} should fail");
    }
    for raw in ["0m", "0.5k", "1e3m", "1_024m", "999999999999999999999999g"] {
        assert!(MemoryLimit::parse(raw).is_err(), "{raw} should fail");
    }
}

#[test]
fn cli_rejects_invalid_resource_flags() {
    assert!(
        parse_from(vec![
            "scaler".into(),
            "run".into(),
            "--cpu".into(),
            "1e3c".into(),
            "--".into(),
            "echo".into(),
        ])
        .is_err()
    );
    assert!(
        parse_from(vec![
            "scaler".into(),
            "run".into(),
            "--mem".into(),
            "0.5m".into(),
            "--".into(),
            "echo".into(),
        ])
        .is_err()
    );
}

#[test]
fn shell_mode_requires_exactly_one_script() {
    assert!(Cli::try_parse_from(["scaler", "run", "--shell", "sh"]).is_err());
    assert!(Cli::try_parse_from(["scaler", "run", "--shell", "fish", "--", "echo ok"]).is_err());
    assert!(Cli::try_parse_from(["scaler", "run", "--shell", "sh", "--", "echo", "ok"]).is_err());
}

#[test]
fn shell_mode_accepts_one_script() {
    let cli = Cli::try_parse_from(["scaler", "run", "--shell", "sh", "--", "echo ok"]).unwrap();
    assert_eq!(cli.command_name(), "run");
}

#[test]
fn bare_invocation_is_a_usage_error() {
    assert!(Cli::try_parse_from(["scaler"]).is_err());
    assert!(Cli::try_parse_from(["scaler", "run"]).is_err());
    assert!(Cli::try_parse_from(["scaler", "run", "--"]).is_err());
}
