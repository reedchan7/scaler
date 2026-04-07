use std::time::Duration;

use crate::core::{CapabilityReport, OutputFrame};

pub use crate::core::summary::format_bytes;
pub use crate::core::summary::format_duration as format_elapsed;

pub mod plain;
pub mod tui;

#[derive(Debug, Clone)]
pub struct UiContext {
    pub command: String,
    pub capabilities: CapabilityReport,
    pub compact: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MonitorSnapshot {
    pub elapsed: Duration,
    pub cpu_percent: Option<f32>,
    pub memory_bytes: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
    pub child_count: Option<u32>,
    pub run_status: String,
}

pub trait Renderer {
    fn render_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()>;
    fn render_snapshot(&mut self, snapshot: &MonitorSnapshot) -> anyhow::Result<()>;
    fn finish(&mut self) -> anyhow::Result<()>;

    fn replay(&mut self, frames: &[OutputFrame]) -> anyhow::Result<()> {
        for frame in frames {
            self.render_frame(frame)?;
        }
        Ok(())
    }
}
