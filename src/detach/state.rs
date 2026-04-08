//! State directory layout, meta.json / result.json types, atomic writes.
//!
//! Layout under `$XDG_STATE_HOME/scaler/runs/<id>/`:
//! - `meta.json`: written once at run start (static run info)
//! - `result.json`: written once at run end (terminal state + metrics)
//! - `stdout.log`, `stderr.log`: captured child output

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result as AnyResult};
use serde::{Deserialize, Serialize};

use crate::detach::id::RunId;

#[derive(Debug, Clone)]
pub struct StateRoot {
    base: PathBuf,
}

impl StateRoot {
    /// Resolve from `$XDG_STATE_HOME` or `$HOME/.local/state` (both Linux and
    /// macOS — we deliberately don't use `~/Library/Application Support/` on
    /// macOS to keep the layout uniform).
    pub fn from_env() -> AnyResult<Self> {
        let base = if let Some(xdg) = std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
            PathBuf::from(xdg)
        } else {
            let home = std::env::var_os("HOME").context("neither XDG_STATE_HOME nor HOME set")?;
            PathBuf::from(home).join(".local").join("state")
        };
        Ok(Self { base })
    }

    pub fn with_base(base: PathBuf) -> Self {
        Self { base }
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.base.join("scaler").join("runs")
    }

    pub fn run_dir(&self, id: &RunId) -> PathBuf {
        self.runs_dir().join(id.as_str())
    }

    pub fn meta_path(&self, id: &RunId) -> PathBuf {
        self.run_dir(id).join("meta.json")
    }

    pub fn result_path(&self, id: &RunId) -> PathBuf {
        self.run_dir(id).join("result.json")
    }

    pub fn stdout_log_path(&self, id: &RunId) -> PathBuf {
        self.run_dir(id).join("stdout.log")
    }

    pub fn stderr_log_path(&self, id: &RunId) -> PathBuf {
        self.run_dir(id).join("stderr.log")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    pub id: String,
    pub started: String,
    pub command: Vec<String>,
    pub cwd: String,
    pub cpu_limit_centi_cores: Option<u32>,
    pub mem_limit_bytes: Option<u64>,
    pub platform: String,
    pub backend: String,
    pub backend_state: String,
    pub pid: Option<u32>,
    pub unit_name: Option<String>,
    pub scaler_exe: String,
    pub scaler_version: String,
    pub stdout_log: String,
    pub stderr_log: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Exited,
    Killed,
    LaunchFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub version: u32,
    pub id: String,
    pub ended: String,
    pub state: RunState,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    pub cpu_total_nanos: Option<u128>,
    pub memory_peak_bytes: Option<u64>,
    pub launch_error: Option<String>,
}

pub fn write_meta(root: &StateRoot, id: &RunId, meta: &Meta) -> AnyResult<()> {
    ensure_run_dir(root, id)?;
    atomic_write_json(&root.meta_path(id), meta)
}

pub fn write_result(root: &StateRoot, id: &RunId, result: &RunResult) -> AnyResult<()> {
    ensure_run_dir(root, id)?;
    atomic_write_json(&root.result_path(id), result)
}

pub fn read_meta(root: &StateRoot, id: &RunId) -> AnyResult<Meta> {
    let path = root.meta_path(id);
    let bytes = fs::read(&path).with_context(|| format!("read meta.json at {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse meta.json at {}", path.display()))
}

pub fn read_result(root: &StateRoot, id: &RunId) -> AnyResult<RunResult> {
    let path = root.result_path(id);
    let bytes =
        fs::read(&path).with_context(|| format!("read result.json at {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse result.json at {}", path.display()))
}

pub fn list_run_ids(root: &StateRoot) -> AnyResult<Vec<RunId>> {
    let dir = root.runs_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<RunId> = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("list runs dir {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if let Some(id) = RunId::parse(name_str) {
            out.push(id);
        }
    }
    out.sort_by(|a, b| b.as_str().cmp(a.as_str()));
    Ok(out)
}

fn ensure_run_dir(root: &StateRoot, id: &RunId) -> AnyResult<()> {
    let dir = root.run_dir(id);
    fs::create_dir_all(&dir).with_context(|| format!("create run dir {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("chmod run dir {}", dir.display()))?;
    }
    Ok(())
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> AnyResult<()> {
    let parent = path.parent().context("path has no parent")?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("open tmp file in {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(value).context("serialize to json")?;
    tmp.write_all(&bytes).context("write tmp file")?;
    tmp.write_all(b"\n").context("write tmp file newline")?;
    tmp.as_file().sync_all().context("fsync tmp file")?;
    tmp.persist(path)
        .map_err(|e| anyhow::anyhow!("persist tmp to {}: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod {}", path.display()))?;
    }
    Ok(())
}
