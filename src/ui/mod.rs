use std::time::Duration;

use crate::core::{CapabilityReport, OutputFrame};

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

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_elapsed(duration: Duration) -> String {
    format!("{:.3}s", duration.as_secs_f64())
}
