use std::io::{self, Stdout, Write};

use anyhow::Context;
use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{CrosstermBackend, TestBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    core::{OutputFrame, OutputStream},
    ui::{MonitorSnapshot, Renderer, UiContext, format_bytes, format_elapsed},
};

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub headless: bool,
    pub fail_on_start: Option<String>,
    pub fail_after_draws: Option<usize>,
}

#[derive(Debug)]
pub struct TuiRenderer {
    terminal: TerminalHandle,
    state: TuiState,
    fail_after_draws: Option<usize>,
}

#[derive(Debug)]
enum TerminalHandle {
    Real(Terminal<CrosstermBackend<Stdout>>),
    Test(Terminal<TestBackend>),
}

/// Cap on the per-stream replay buffer. Long outputs see only the tail of
/// each stream — same trade-off as `trim_output` for the display buffer.
const REPLAY_BUFFER_CAP: usize = 64 * 1024;

#[derive(Debug)]
struct TuiState {
    context: UiContext,
    snapshot: MonitorSnapshot,
    /// Mixed display buffer used to render the dashboard "Output" pane.
    output: String,
    /// Bytes destined for real stdout once the alt-screen tears down.
    /// Per-stream so child stdout and child stderr stay separated.
    stdout_replay: Vec<u8>,
    /// Bytes destined for real stderr once the alt-screen tears down.
    stderr_replay: Vec<u8>,
}

impl TuiRenderer {
    pub fn start(context: UiContext, options: InitOptions) -> anyhow::Result<Self> {
        if let Some(message) = options.fail_on_start {
            anyhow::bail!(message);
        }

        let state = TuiState {
            context,
            snapshot: MonitorSnapshot::default(),
            output: String::new(),
            stdout_replay: Vec::new(),
            stderr_replay: Vec::new(),
        };
        let mut renderer = Self {
            terminal: if options.headless {
                TerminalHandle::Test(Terminal::new(TestBackend::new(120, 40))?)
            } else {
                TerminalHandle::Real(init_real_terminal()?)
            },
            state,
            fail_after_draws: options.fail_after_draws,
        };
        renderer.draw()?;
        Ok(renderer)
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        if let Some(remaining) = self.fail_after_draws.as_mut() {
            if *remaining == 0 {
                anyhow::bail!("simulated monitor draw failure");
            }
            *remaining -= 1;
        }

        self.terminal.draw(&self.state)
    }
}

impl Renderer for TuiRenderer {
    fn render_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()> {
        // Append to the mixed display buffer (used for the dashboard pane).
        self.state
            .output
            .push_str(String::from_utf8_lossy(&frame.bytes).as_ref());
        trim_output(&mut self.state.output);

        // Append to the per-stream replay buffer so we can hand the bytes
        // back to real stdout/stderr in finish() with the original stream
        // identity preserved.
        let target = match frame.stream {
            OutputStream::Stdout | OutputStream::PtyMerged => &mut self.state.stdout_replay,
            OutputStream::Stderr => &mut self.state.stderr_replay,
        };
        target.extend_from_slice(&frame.bytes);
        trim_replay(target);

        self.draw()
    }

    fn render_snapshot(&mut self, snapshot: &MonitorSnapshot) -> anyhow::Result<()> {
        self.state.snapshot = snapshot.clone();
        self.draw()
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        self.terminal.restore()?;
        // After leaving the alt-screen, replay the captured child output to
        // real stdout/stderr — preserving the original stream identity so
        // pipelines like `scaler run -- jq < x.json > out.json` keep
        // `out.json` clean of any stderr noise. Each replay buffer is
        // trimmed to a 64 KiB tail, so very long outputs see only the tail.
        // For full streaming use the plain renderer (which writes frames
        // live as they arrive).
        replay_to(std::io::stdout().lock(), &self.state.stdout_replay)?;
        replay_to(std::io::stderr().lock(), &self.state.stderr_replay)?;
        Ok(())
    }
}

fn replay_to<W: Write>(mut writer: W, buffer: &[u8]) -> anyhow::Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    writer.write_all(buffer)?;
    if !buffer.ends_with(b"\n") {
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn trim_replay(buffer: &mut Vec<u8>) {
    if buffer.len() > REPLAY_BUFFER_CAP {
        let drain_to = buffer.len() - REPLAY_BUFFER_CAP;
        buffer.drain(..drain_to);
    }
}

impl TerminalHandle {
    fn draw(&mut self, state: &TuiState) -> anyhow::Result<()> {
        match self {
            Self::Real(terminal) => {
                terminal
                    .draw(|frame| render_dashboard(frame, state))
                    .context("failed to draw monitor")?;
            }
            Self::Test(terminal) => {
                terminal
                    .draw(|frame| render_dashboard(frame, state))
                    .context("failed to draw headless monitor")?;
            }
        }
        Ok(())
    }

    fn restore(&mut self) -> anyhow::Result<()> {
        if let Self::Real(terminal) = self {
            restore_real_terminal(terminal.backend_mut())?;
            terminal.show_cursor()?;
        }
        Ok(())
    }
}

fn init_real_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    let mut stdout = io::stdout();
    enable_raw_mode().context("failed to enable raw mode for monitor")?;

    if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
        let _ = disable_raw_mode();
        return Err(error).context("failed to enter alternate screen for monitor");
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create monitor terminal")?;
    if let Err(error) = terminal.clear() {
        let _ = restore_real_terminal(terminal.backend_mut());
        return Err(error).context("failed to clear monitor terminal");
    }
    Ok(terminal)
}

fn restore_real_terminal(backend: &mut CrosstermBackend<Stdout>) -> anyhow::Result<()> {
    execute!(backend, Show, LeaveAlternateScreen).context("failed to restore terminal")?;
    disable_raw_mode().context("failed to disable raw mode for monitor")?;
    backend.flush()?;
    Ok(())
}

fn render_dashboard(frame: &mut ratatui::Frame<'_>, state: &TuiState) {
    let vertical = if state.context.compact {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(7),
                Constraint::Min(10),
            ])
            .split(frame.area())
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(warning_height(&state.context.warnings)),
                Constraint::Length(9),
                Constraint::Min(10),
            ])
            .split(frame.area())
    };

    frame.render_widget(header(state), vertical[0]);

    let (metrics_index, output_index) = if state.context.compact {
        (1, 2)
    } else {
        frame.render_widget(warnings(state), vertical[1]);
        (2, 3)
    };

    frame.render_widget(metrics(state), vertical[metrics_index]);
    frame.render_widget(output(state), vertical[output_index]);
}

fn header(state: &TuiState) -> Paragraph<'_> {
    let capabilities = &state.context.capabilities;
    let line = Line::from(vec![
        Span::styled(
            format!("backend: {}", capabilities.backend.as_str()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(format!(
            "backend_state: {}",
            capabilities.backend_state.as_str()
        )),
        Span::raw("  "),
        Span::raw(format!("cpu: {}", capabilities.cpu.as_str())),
        Span::raw("  "),
        Span::raw(format!("memory: {}", capabilities.memory.as_str())),
        Span::raw("  "),
        Span::raw(format!(
            "interactive: {}",
            capabilities.interactive.as_str()
        )),
    ]);

    Paragraph::new(line).block(Block::default().borders(Borders::ALL).title("Monitor"))
}

fn warnings(state: &TuiState) -> Paragraph<'_> {
    let style = Style::default().fg(Color::Yellow);
    let body = if state.context.warnings.is_empty() {
        "No warnings".to_string()
    } else {
        state.context.warnings.join("\n")
    };

    Paragraph::new(body)
        .style(style)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Warnings"))
}

fn metrics(state: &TuiState) -> Paragraph<'_> {
    let snapshot = &state.snapshot;
    let memory = snapshot
        .memory_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "n/a".to_string());
    let peak_memory = snapshot
        .peak_memory_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "n/a".to_string());
    let child_count = snapshot
        .child_count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let cpu = snapshot
        .cpu_percent
        .map(|percent| format!("{percent:.1}%"))
        .unwrap_or_else(|| "n/a".to_string());

    let rows = [
        format!("command: {}", state.context.command),
        format!("backend: {}", state.context.capabilities.backend.as_str()),
        format!("elapsed: {}", format_elapsed(snapshot.elapsed)),
        format!("cpu: {cpu}"),
        format!("memory: {memory}"),
        format!("peak_memory: {peak_memory}"),
        format!("child_count: {child_count}"),
        format!("status: {}", snapshot.run_status),
    ]
    .join("\n");

    Paragraph::new(rows)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Metrics"))
}

fn output(state: &TuiState) -> Paragraph<'_> {
    Paragraph::new(state.output.as_str())
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Output"))
}

fn trim_output(output: &mut String) {
    const MAX_CHARS: usize = 32_000;

    if output.len() > MAX_CHARS {
        let drain_to = output.len() - MAX_CHARS;
        output.drain(..drain_to);
    }
}

fn warning_height(warnings: &[String]) -> u16 {
    if warnings.is_empty() {
        3
    } else {
        warnings.len().min(4) as u16 + 2
    }
}
