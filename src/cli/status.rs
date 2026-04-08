//! `scaler status` rendering: takes pre-collected run views and writes
//! them as text (default) or JSON (`--json`). Pure rendering — no I/O,
//! no platform calls. Live state collection lives in `crate::detach::*`.

use std::io::Write;

use anyhow::Result;
use serde::Serialize;

use crate::detach::state::{Meta, RunResult, RunState};

/// Aggregated view of one run, merged from `meta.json` plus either
/// `result.json` (terminal) or a live snapshot (running) or neither (gone).
#[derive(Debug, Clone, Serialize)]
pub struct RunView {
    pub meta: Meta,
    /// Present when the run has ended.
    pub result: Option<RunResult>,
    /// Present when the run is still live AND a snapshot was collectable.
    pub live: Option<LiveSnapshot>,
    /// True when the run is neither live nor has result.json.
    pub gone: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveSnapshot {
    pub cpu_total_nanos: Option<u128>,
    pub memory_current_bytes: Option<u64>,
    pub elapsed_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState {
    Running,
    Exited(i32),
    Killed,
    LaunchFailed,
    Gone,
}

impl RunView {
    pub fn display_state(&self) -> DisplayState {
        if let Some(r) = &self.result {
            return match r.state {
                RunState::Exited => DisplayState::Exited(r.exit_code.unwrap_or(0)),
                RunState::Killed => DisplayState::Killed,
                RunState::LaunchFailed => DisplayState::LaunchFailed,
            };
        }
        if self.gone {
            return DisplayState::Gone;
        }
        DisplayState::Running
    }
}

pub fn render_list<W: Write>(out: &mut W, views: &[RunView], json: bool) -> Result<()> {
    if json {
        serde_json::to_writer_pretty(&mut *out, views)?;
        writeln!(out)?;
        return Ok(());
    }
    for v in views {
        writeln!(out, "{}", format_list_row(v))?;
    }
    Ok(())
}

pub fn render_detail<W: Write>(out: &mut W, view: &RunView, json: bool) -> Result<()> {
    if json {
        serde_json::to_writer_pretty(&mut *out, view)?;
        writeln!(out)?;
        return Ok(());
    }
    write_detail_text(out, view)
}

fn format_list_row(v: &RunView) -> String {
    let state = format_state_short(v.display_state());
    let duration = format_duration_from_view(v);
    let cmd = v.meta.command.join(" ");
    format!("{}  {:<14}  {:>8}  {}", v.meta.id, state, duration, cmd)
}

fn format_state_short(s: DisplayState) -> String {
    match s {
        DisplayState::Running => "running".into(),
        DisplayState::Exited(c) => format!("exited({c})"),
        DisplayState::Killed => "killed".into(),
        DisplayState::LaunchFailed => "launch_failed".into(),
        DisplayState::Gone => "gone".into(),
    }
}

fn format_duration_from_view(v: &RunView) -> String {
    if let Some(r) = &v.result
        && let (Some(started), Some(ended)) =
            (parse_rfc3339(&v.meta.started), parse_rfc3339(&r.ended))
    {
        return format_secs((ended - started).max(0) as u64);
    }
    if let Some(live) = &v.live
        && let Some(secs) = live.elapsed_secs
    {
        return format_secs(secs);
    }
    "-".into()
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(|dt| dt.unix_timestamp())
}

fn format_secs(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

fn write_detail_text<W: Write>(out: &mut W, v: &RunView) -> Result<()> {
    let m = &v.meta;
    writeln!(out, "id:       {}", m.id)?;
    writeln!(out, "command:  {}", m.command.join(" "))?;
    let cpu = m
        .cpu_limit_centi_cores
        .map(|c| format!("{:.2}c", c as f64 / 100.0))
        .unwrap_or_else(|| "-".into());
    let mem = m
        .mem_limit_bytes
        .map(format_mib)
        .unwrap_or_else(|| "-".into());
    writeln!(out, "limits:   cpu={cpu}  mem={mem}")?;
    writeln!(out, "backend:  {} ({})", m.backend, m.backend_state)?;
    writeln!(out, "started:  {}", m.started)?;
    match v.display_state() {
        DisplayState::Running => {
            writeln!(out, "state:    running")?;
            if let Some(live) = &v.live {
                if let Some(mc) = live.memory_current_bytes {
                    writeln!(out, "memory:   current {}", format_mib(mc))?;
                }
                if let Some(ns) = live.cpu_total_nanos {
                    writeln!(
                        out,
                        "cpu:      total {}",
                        format_secs((ns / 1_000_000_000) as u64)
                    )?;
                }
                if let Some(e) = live.elapsed_secs {
                    writeln!(out, "elapsed:  {}", format_secs(e))?;
                }
            }
        }
        DisplayState::Exited(c) => {
            let r = v
                .result
                .as_ref()
                .expect("display_state Exited implies result");
            writeln!(out, "ended:    {}", r.ended)?;
            writeln!(out, "state:    exited({c})")?;
            write_final_metrics(out, r)?;
        }
        DisplayState::Killed => {
            let r = v
                .result
                .as_ref()
                .expect("display_state Killed implies result");
            writeln!(out, "ended:    {}", r.ended)?;
            let sig = r.signal.as_deref().unwrap_or("?");
            writeln!(out, "state:    killed({sig})")?;
            write_final_metrics(out, r)?;
        }
        DisplayState::LaunchFailed => {
            let r = v
                .result
                .as_ref()
                .expect("display_state LaunchFailed implies result");
            writeln!(out, "state:    launch_failed")?;
            if let Some(e) = &r.launch_error {
                writeln!(out, "error:    {e}")?;
            }
        }
        DisplayState::Gone => {
            writeln!(
                out,
                "state:    gone (no result.json and process is not running)"
            )?;
        }
    }
    writeln!(out, "stdout:   {}", m.stdout_log)?;
    writeln!(out, "stderr:   {}", m.stderr_log)?;
    Ok(())
}

fn write_final_metrics<W: Write>(out: &mut W, r: &RunResult) -> Result<()> {
    if let Some(ns) = r.cpu_total_nanos {
        writeln!(
            out,
            "cpu:      total {}",
            format_secs((ns / 1_000_000_000) as u64)
        )?;
    }
    if let Some(p) = r.memory_peak_bytes {
        writeln!(out, "memory:   peak {}", format_mib(p))?;
    }
    Ok(())
}

fn format_mib(bytes: u64) -> String {
    let mib = bytes as f64 / 1024.0 / 1024.0;
    if mib >= 1024.0 {
        format!("{:.2} GiB", mib / 1024.0)
    } else {
        format!("{mib:.0} MiB")
    }
}
