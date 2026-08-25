use crate::budget::BudgetUsage;
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, ffi::OsString};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent<'a> {
    pub timestamp: DateTime<Utc>,
    pub run_id: Uuid,
    pub event: &'a str,
    pub data: Value,
    pub budget: &'a BudgetUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: Uuid,
    pub phase: String,
    pub iteration: u32,
    pub last_action: Option<String>,
    pub budget: BudgetUsage,
}

pub struct RunStore {
    run_id: Uuid,
    dir: PathBuf,
    events: File,
    progress: Option<File>,
    activity: Option<File>,
}

impl RunStore {
    pub fn create(project_root: &Path, run_root: &str, prompt: &str) -> Result<Self> {
        let run_id = Uuid::new_v4();
        let root = if Path::new(run_root).is_absolute() {
            PathBuf::from(run_root)
        } else {
            project_root.join(run_root)
        };
        let dir = root.join(run_id.to_string());
        fs::create_dir_all(dir.join("artifacts"))
            .with_context(|| format!("failed to create run directory: {}", dir.display()))?;
        fs::write(dir.join("prompt.md"), prompt)?;
        let events = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(dir.join("events.jsonl"))?;
        let progress = env::var_os("AICONDUCTOR_PROGRESS_LOG")
            .filter(|value| !value.is_empty())
            .map(open_append_log)
            .transpose()?;
        let activity = env::var_os("AICONDUCTOR_ACTIVITY_LOG")
            .filter(|value| !value.is_empty())
            .map(open_append_log)
            .transpose()?;
        Ok(Self {
            run_id,
            dir,
            events,
            progress,
            activity,
        })
    }

    pub fn id(&self) -> Uuid {
        self.run_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn event(&mut self, event: &str, data: Value, budget: &BudgetUsage) -> Result<()> {
        let record = AuditEvent {
            timestamp: Utc::now(),
            run_id: self.run_id,
            event,
            data,
            budget,
        };
        let encoded = serde_json::to_vec(&record)?;
        self.events.write_all(&encoded)?;
        self.events.write_all(b"\n")?;
        self.events.flush()?;
        if let Some(progress) = &mut self.progress {
            progress.write_all(&encoded)?;
            progress.write_all(b"\n")?;
            progress.flush()?;
        }
        if let Some(activity) = &mut self.activity {
            activity.write_all(format_activity_event(&record).as_bytes())?;
            activity.flush()?;
        }
        Ok(())
    }

    pub fn checkpoint(&self, state: &RunState) -> Result<()> {
        let temporary = self.dir.join(".state.json.tmp");
        let final_path = self.dir.join("state.json");
        let data = serde_json::to_vec_pretty(state)?;
        fs::write(&temporary, data)?;
        fs::rename(&temporary, &final_path)?;
        Ok(())
    }

    pub fn write_result(&self, result: &str) -> Result<()> {
        fs::write(self.dir.join("result.md"), result)?;
        Ok(())
    }
}

fn open_append_log(path: OsString) -> Result<File> {
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open activity log: {}", path.display()))
}

fn format_activity_event(record: &AuditEvent<'_>) -> String {
    let detail = ["action", "profile", "tool", "error"]
        .iter()
        .find_map(|key| record.data.get(key).and_then(Value::as_str))
        .map(|value| truncate(value, 160));
    let detail = detail
        .map(|value| format!(" | {value}"))
        .unwrap_or_default();
    format!(
        "[{}] progress {} (iteration={} model={} mcp={} switch={}){}\n",
        record
            .timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%dT%H:%M:%S%:z"),
        record.event,
        record.budget.iterations,
        record.budget.model_calls,
        record.budget.mcp_calls,
        record.budget.model_switches,
        detail,
    )
}

fn truncate(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let shortened = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}
