use std::io::Write;

use crate::{
    core::{CapabilityLevel, OutputFrame, OutputStream},
    ui::{MonitorSnapshot, Renderer, UiContext},
};

#[derive(Debug)]
pub struct PlainRenderer;

impl PlainRenderer {
    pub fn new(context: &UiContext) -> anyhow::Result<Self> {
        let mut stderr = std::io::stderr().lock();

        writeln!(
            stderr,
            "{} backend: {}",
            status_prefix(context.capabilities.backend_state),
            context.capabilities.backend.as_str()
        )?;
        writeln!(
            stderr,
            "{} cpu: {}",
            status_prefix(context.capabilities.cpu),
            context.capabilities.cpu.as_str()
        )?;
        writeln!(
            stderr,
            "{} memory: {}",
            status_prefix(context.capabilities.memory),
            context.capabilities.memory.as_str()
        )?;
        writeln!(
            stderr,
            "{} interactive: {}",
            status_prefix(context.capabilities.interactive),
            context.capabilities.interactive.as_str()
        )?;

        for warning in &context.warnings {
            writeln!(stderr, "warning: {warning}")?;
        }
        stderr.flush()?;

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

fn status_prefix(level: CapabilityLevel) -> &'static str {
    match level {
        CapabilityLevel::Enforced => "[enforced]",
        CapabilityLevel::BestEffort => "[best-effort]",
        CapabilityLevel::Unavailable => "[unavailable]",
    }
}
