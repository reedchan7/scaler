use anyhow::Result;
use std::ffi::{OsStr, OsString};

pub mod args;
pub mod values;

pub use args::Cli;

pub fn normalize_argv(raw: Vec<OsString>) -> Vec<OsString> {
    if raw.len() <= 1 {
        return raw;
    }

    let first = raw[1].as_os_str();
    if first == OsStr::new("--") || (is_run_shorthand_flag(first) && has_delimiter(&raw[2..])) {
        let mut normalized = raw;
        normalized.insert(1, OsString::from("run"));
        return normalized;
    }

    raw
}

fn is_run_shorthand_flag(value: &OsStr) -> bool {
    matches!(
        value,
        s if s == OsStr::new("--cpu")
            || s == OsStr::new("--mem")
            || s == OsStr::new("--interactive")
            || s == OsStr::new("--shell")
            || s == OsStr::new("--no-monitor")
    )
}

fn has_delimiter(values: &[OsString]) -> bool {
    values
        .iter()
        .any(|value| value.as_os_str() == OsStr::new("--"))
}

pub fn parse_from(raw: Vec<OsString>) -> Result<Cli> {
    let normalized = normalize_argv(raw);
    Ok(Cli::try_parse_from(normalized)?)
}
