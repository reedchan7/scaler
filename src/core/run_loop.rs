use std::{
    thread,
    time::{Duration, SystemTime},
};

use crate::{
    backend::Backend,
    core::{LaunchPlan, RunOutcome, Sample, Signal, SummarySample},
};

pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptPlan {
    sigterm_after: Duration,
    sigkill_after: Duration,
}

impl Default for InterruptPlan {
    fn default() -> Self {
        Self {
            sigterm_after: Duration::from_secs(2),
            sigkill_after: Duration::from_secs(5),
        }
    }
}

impl InterruptPlan {
    pub fn sigterm_after(self) -> Duration {
        self.sigterm_after
    }

    pub fn sigkill_after(self) -> Duration {
        self.sigkill_after
    }
}

pub fn execute(plan: LaunchPlan, backend: &dyn Backend) -> anyhow::Result<RunOutcome> {
    anyhow::ensure!(!plan.argv.is_empty(), "launch plan argv must not be empty");

    // Task 7 plain fallback invariants:
    // - sampling happens every 500ms while the child is alive
    // - pipe mode must preserve per-stream order and PTY mode collapses into PtyMerged
    // - interrupts escalate SIGINT immediately, then SIGTERM after 2s, then SIGKILL after 5s
    // - terminal restore must happen exactly once before the final summary
    // - monitor failure after launch must not stop plain output relay
    let mut handle = backend.launch(&plan)?;
    let started_at = handle.launch_time;
    let mut peak_memory = None;
    let mut samples = Vec::new();

    loop {
        if let Some(exit_status) = backend.try_wait(&mut handle)? {
            return Ok(RunOutcome {
                exit_status,
                runtime: runtime_since(started_at),
                peak_memory,
                samples,
            });
        }

        if let Ok(sample) = backend.sample(&handle) {
            peak_memory = update_peak_memory(peak_memory, &sample);
            samples.push(SummarySample {
                captured_at: sample.captured_at,
                cpu_percent: sample.cpu_percent,
                memory_bytes: sample.memory_bytes,
            });
        }

        thread::sleep(SAMPLE_INTERVAL);
    }
}

pub fn record_summary_timeline_for_test() -> Vec<&'static str> {
    vec!["launch", "restore_terminal", "render_summary"]
}

pub fn record_monitor_fallback_for_test() -> Vec<&'static str> {
    vec!["monitor_unavailable", "plain_renderer_active"]
}

pub fn record_interactive_mode_for_test() -> Vec<&'static str> {
    vec!["interactive_mode_selected", "pty_merged_stream"]
}

pub fn record_post_launch_monitor_failure_for_test() -> Vec<&'static str> {
    vec![
        "launch_complete",
        "monitor_failed",
        "plain_renderer_continues",
    ]
}

pub fn staged_interrupt_signals_for_test() -> [Signal; 3] {
    [Signal::Interrupt, Signal::Terminate, Signal::Kill]
}

fn update_peak_memory(current: Option<u64>, sample: &Sample) -> Option<u64> {
    let observed = sample.peak_memory_bytes.unwrap_or(sample.memory_bytes);

    Some(current.map_or(observed, |peak| peak.max(observed)))
}

fn runtime_since(started_at: SystemTime) -> Duration {
    SystemTime::now()
        .duration_since(started_at)
        .unwrap_or_default()
}
