use anyhow::Result;
use std::ffi::{OsStr, OsString};

pub mod args;
pub mod status;
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
    let value = value.to_string_lossy();
    matches_flag(&value, "--cpu")
        || matches_flag(&value, "--mem")
        || matches_flag(&value, "--interactive")
        || matches_flag(&value, "--shell")
        || matches_flag(&value, "--monitor")
        || matches_flag(&value, "--detach")
        || value == "-d"
}

fn has_delimiter(values: &[OsString]) -> bool {
    values
        .iter()
        .any(|value| value.as_os_str() == OsStr::new("--"))
}

fn matches_flag(value: &str, flag: &str) -> bool {
    value == flag
        || value
            .strip_prefix(flag)
            .is_some_and(|suffix| suffix.starts_with('='))
}

pub fn parse_from(raw: Vec<OsString>) -> Result<Cli> {
    let normalized = normalize_argv(raw);
    Ok(Cli::try_parse_from(normalized)?)
}

pub fn render_doctor_output(
    report: &crate::core::CapabilityReport,
    effective: crate::core::BackendKind,
) -> String {
    let mut lines = vec![
        format!("platform: {}", report.platform.as_str()),
        format!("backend: {}", report.backend.as_str()),
        format!("backend_state: {}", report.backend_state.as_str()),
        format!("cpu: {}", report.cpu.as_str()),
        format!("memory: {}", report.memory.as_str()),
        format!("interactive: {}", report.interactive.as_str()),
        format!("effective_backend: {}", effective.as_str()),
    ];

    lines.extend(
        report
            .prerequisites
            .iter()
            .map(render_doctor_prerequisite)
            .map(|prerequisite| format!("prerequisite: {prerequisite}")),
    );
    lines.extend(
        sorted_warning_lines(&report.warnings)
            .into_iter()
            .map(|warning| format!("warning: {warning}")),
    );

    lines.join("\n")
}

fn render_doctor_prerequisite(prerequisite: &crate::core::DoctorPrerequisite) -> String {
    match prerequisite {
        crate::core::DoctorPrerequisite::Check { key, status } => {
            format!("{key}={}", render_prerequisite_status(*status))
        }
        crate::core::DoctorPrerequisite::Note(message) => (*message).to_string(),
    }
}

fn render_prerequisite_status(status: crate::core::PrerequisiteStatus) -> &'static str {
    match status {
        crate::core::PrerequisiteStatus::Ok => "ok",
        crate::core::PrerequisiteStatus::Missing => "missing",
        crate::core::PrerequisiteStatus::Unreachable => "unreachable",
        crate::core::PrerequisiteStatus::Unsupported => "unsupported",
        crate::core::PrerequisiteStatus::Skipped => "skipped",
    }
}

fn sorted_warning_lines(warnings: &[String]) -> Vec<&str> {
    let mut warnings = warnings.iter().map(String::as_str).collect::<Vec<_>>();
    warnings.sort_unstable();
    warnings
}
