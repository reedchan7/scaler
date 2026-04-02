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
    if matches!(first, s if s == OsStr::new("run") || s == OsStr::new("doctor") || s == OsStr::new("version"))
    {
        return raw;
    }

    let mut normalized = raw;
    normalized.insert(1, OsString::from("run"));
    normalized
}

pub fn parse_from(raw: Vec<OsString>) -> Result<Cli> {
    let normalized = normalize_argv(raw);
    Ok(Cli::try_parse_from(normalized)?)
}
