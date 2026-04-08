use std::io::Write;

use crate::{
    core::{OutputFrame, OutputStream},
    ui::{MonitorSnapshot, Renderer, UiContext},
};

#[derive(Debug)]
pub struct PlainRenderer;

impl PlainRenderer {
    pub fn new(_context: &UiContext) -> anyhow::Result<Self> {
        // The capability banner and warnings used to be printed here at
        // construction time. They have moved into `core::summary::render`
        // (the context block above the summary card) so the wrapped
        // command's output is never preceded by scaler chrome.
        Ok(Self)
    }
}

impl Renderer for PlainRenderer {
    fn render_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()> {
        match frame.stream {
            OutputStream::Stdout | OutputStream::PtyMerged => {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(&frame.bytes)?;
                stdout.flush()?;
            }
            OutputStream::Stderr => {
                let mut stderr = std::io::stderr().lock();
                stderr.write_all(&frame.bytes)?;
                stderr.flush()?;
            }
        }
        Ok(())
    }

    fn render_snapshot(&mut self, _snapshot: &MonitorSnapshot) -> anyhow::Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Renderer that accepts frames and snapshots and does nothing. Used by
/// `core::run_loop::execute_headless` for detached runs where stdout has
/// been redirected to a log file by the caller before the run loop starts;
/// printing the summary card would corrupt that captured output.
#[derive(Debug, Default)]
pub struct NoopRenderer;

impl NoopRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Renderer for NoopRenderer {
    fn render_frame(&mut self, _frame: &OutputFrame) -> anyhow::Result<()> {
        Ok(())
    }

    fn render_snapshot(&mut self, _snapshot: &MonitorSnapshot) -> anyhow::Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        BackendKind, CapabilityLevel, CapabilityReport, DoctorPrerequisite, Platform,
        PrerequisiteStatus,
    };
    use crate::ui::UiContext;

    #[test]
    fn new_does_not_write_anything_to_stderr() {
        // PlainRenderer::new used to print a 4-line capability banner to
        // stderr before the wrapped command produced any output. That has
        // moved into core::summary::render so the wrapped command can
        // emit its own output uninterrupted. The constructor is now a
        // pure factory.
        let context = UiContext {
            command: "echo hello".to_string(),
            capabilities: CapabilityReport {
                platform: Platform::Macos,
                backend: BackendKind::PlainFallback,
                backend_state: CapabilityLevel::BestEffort,
                cpu: CapabilityLevel::BestEffort,
                memory: CapabilityLevel::BestEffort,
                interactive: CapabilityLevel::BestEffort,
                prerequisites: vec![DoctorPrerequisite::check(
                    "taskpolicy",
                    PrerequisiteStatus::Ok,
                )],
                warnings: vec!["host probe failed".to_string()],
            },
            compact: false,
            warnings: vec!["host probe failed".to_string()],
        };

        // We don't have a way to capture stderr inside a unit test
        // without re-routing the global handle, but we CAN assert that
        // the constructor produces no errors and returns. The strong
        // assertion lives in tests/run_loop.rs (Task 4 step 4 below)
        // where we scrape the actual output.
        let _renderer = PlainRenderer::new(&context).expect("plain renderer must construct");
    }
}
