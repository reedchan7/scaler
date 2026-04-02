use std::time::{Duration, SystemTime};

use crate::core::{RunOutcome, SummarySample};

pub fn render(outcome: &RunOutcome) -> String {
    let peak_memory = outcome
        .peak_memory
        .map(|bytes| format!("{bytes} bytes"))
        .unwrap_or_else(|| "unknown".to_string());

    [
        format!("exit_status: {}", outcome.exit_status),
        format!("runtime: {}", format_duration(outcome.runtime)),
        format!("peak_memory: {peak_memory}"),
        format!("samples: {}", outcome.samples.len()),
    ]
    .join("\n")
}

impl RunOutcome {
    pub fn fixture_for_test() -> Self {
        Self {
            exit_status: success_exit_status(),
            runtime: Duration::from_secs(3),
            peak_memory: Some(4_194_304),
            samples: vec![
                SummarySample {
                    captured_at: SystemTime::UNIX_EPOCH,
                    cpu_percent: 12.5,
                    memory_bytes: 2_097_152,
                },
                SummarySample {
                    captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                    cpu_percent: 25.0,
                    memory_bytes: 4_194_304,
                },
            ],
        }
    }
}

fn format_duration(duration: Duration) -> String {
    format!("{:.3}s", duration.as_secs_f64())
}

#[cfg(unix)]
fn success_exit_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;

    std::process::ExitStatus::from_raw(0)
}

#[cfg(windows)]
fn success_exit_status() -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;

    std::process::ExitStatus::from_raw(0)
}
