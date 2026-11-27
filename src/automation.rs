//! First-class automations: scheduled agent runs, standalone or bound to a project.
//!
//! An automation is a saved prompt plus a trigger and an optional model override.
//! Each run is a **log entry** ([`AutomationLog`]), not a conversation: runs execute on a
//! detached agent loop (see `crate::automation_runner`) and never appear in the session
//! list, so an automation that fires every few minutes cannot flood the UI.
//!
//! Automations are editable after creation through [`Automation::apply_edit`], which is
//! shared by the REST layer and the `manage_automations` tool.

use serde::{Deserialize, Serialize};

/// When an automation fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationTrigger {
    /// Fire every N seconds (minimum 60).
    Interval { every_secs: u64 },
    /// Fire on matching minutes of a 5-field cron expression.
    Cron { expr: String },
    /// Fire whenever the harness is idle and the automation is not already running.
    AlwaysOn,
    /// Only run when explicitly triggered.
    Manual,
}

impl Default for AutomationTrigger {
    fn default() -> Self {
        AutomationTrigger::Manual
    }
}

impl AutomationTrigger {
    /// Human-readable summary for the UI.
    pub fn describe(&self) -> String {
        match self {
            AutomationTrigger::Interval { every_secs } => {
                format!("Every {}", humanize_secs(*every_secs))
            }
            AutomationTrigger::Cron { expr } => format!("Cron: {expr}"),
            AutomationTrigger::AlwaysOn => "Always on".to_string(),
            AutomationTrigger::Manual => "Manual".to_string(),
        }
    }
}

pub fn humanize_secs(secs: u64) -> String {
    if secs % 3600 == 0 && secs >= 3600 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

/// Maximum characters of assistant output kept in a run log entry.
///
/// Run output is a log line, not a transcript: cap it so a chatty automation cannot
/// grow the log table without bound.
pub const AUTOMATION_OUTPUT_CAP: usize = 4000;

/// Ring-buffer caps for persisted run logs: the last [`AUTOMATION_LOG_PER_AUTOMATION_CAP`]
/// runs per automation and the last [`AUTOMATION_LOG_TOTAL_CAP`] runs overall.
pub const AUTOMATION_LOG_PER_AUTOMATION_CAP: usize = 200;
/// Overall cap across every automation (see [`AUTOMATION_LOG_PER_AUTOMATION_CAP`]).
pub const AUTOMATION_LOG_TOTAL_CAP: usize = 1000;

/// Roll-up of recent runs for one automation (used by the UI and the
/// `manage_automations` audit action).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LogSummary {
    /// Runs in the sample.
    pub runs: usize,
    /// Runs that finished successfully.
    pub success: usize,
    /// Runs that errored.
    pub failed: usize,
    /// Runs cancelled before finishing.
    pub stopped: usize,
    /// Runs still executing.
    pub running: usize,
    /// Mean duration of the finished runs, in milliseconds.
    pub avg_duration_ms: u64,
    /// Timestamp of the newest finished run (RFC3339).
    pub last_finished_at: Option<String>,
}

/// Summarize a slice of run logs. Empty input yields a zeroed summary.
pub fn summarize_logs(logs: &[AutomationLog]) -> LogSummary {
    let mut summary = LogSummary {
        runs: logs.len(),
        ..Default::default()
    };
    let mut durations: Vec<u64> = Vec::new();
    for log in logs {
        match log.status {
            AutomationRunStatus::Success => summary.success += 1,
            AutomationRunStatus::Failed => summary.failed += 1,
            AutomationRunStatus::Stopped => summary.stopped += 1,
            AutomationRunStatus::Running => summary.running += 1,
        }
        if let Some(ms) = log.duration_ms {
            durations.push(ms);
        }
        if log.finished_at.is_some() {
            let ts = log
                .finished_at
                .clone()
                .unwrap_or_else(|| log.started_at.clone());
            if summary
                .last_finished_at
                .as_deref()
                .map(|cur| ts > cur.to_string())
                .unwrap_or(true)
            {
                summary.last_finished_at = Some(ts);
            }
        }
    }
    if !durations.is_empty() {
        let total: u64 = durations.iter().sum();
        summary.avg_duration_ms = total / durations.len() as u64;
    }
    summary
}

/// How a run was fired.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunTrigger {
    /// Started by an explicit request (`POST /api/automations/:id/run`, tool `run`).
    Manual,
    /// Fired by the interval scheduler.
    Interval,
    /// Fired by a cron expression.
    Cron,
    /// Fired by the always-on scheduler.
    AlwaysOn,
    /// Reserved for future schedule kinds (kept for forward compatibility).
    Schedule,
}

impl AutomationRunTrigger {
    /// Stable string form used by the API and the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            AutomationRunTrigger::Manual => "manual",
            AutomationRunTrigger::Interval => "interval",
            AutomationRunTrigger::Cron => "cron",
            AutomationRunTrigger::AlwaysOn => "always_on",
            AutomationRunTrigger::Schedule => "schedule",
        }
    }

    /// Derive the trigger label of a run from the automation's own trigger kind.
    pub fn from_trigger(trigger: &AutomationTrigger) -> Self {
        match trigger {
            AutomationTrigger::Interval { .. } => AutomationRunTrigger::Interval,
            AutomationTrigger::Cron { .. } => AutomationRunTrigger::Cron,
            AutomationTrigger::AlwaysOn => AutomationRunTrigger::AlwaysOn,
            AutomationTrigger::Manual => AutomationRunTrigger::Manual,
        }
    }
}

/// Lifecycle state of one automation run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    /// The turn is still executing.
    Running,
    /// The turn finished and produced an answer.
    Success,
    /// The turn errored out (model/tool failure or panic).
    Failed,
    /// The turn was cancelled before it finished.
    Stopped,
}

impl AutomationRunStatus {
    /// Stable string form used by the API and the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            AutomationRunStatus::Running => "running",
            AutomationRunStatus::Success => "success",
            AutomationRunStatus::Failed => "failed",
            AutomationRunStatus::Stopped => "stopped",
        }
    }

    /// True once the run can no longer change state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, AutomationRunStatus::Running)
    }

    /// The `Automation::last_status` string the scheduler and UI expect.
    ///
    /// Mirrors the values the old session-backed implementation copied from a
    /// session's terminal status, so nothing downstream has to change.
    pub fn last_status_str(&self) -> &'static str {
        match self {
            AutomationRunStatus::Running => "running",
            AutomationRunStatus::Success => "completed",
            AutomationRunStatus::Failed => "failed",
            AutomationRunStatus::Stopped => "stopped",
        }
    }
}

/// One automation run: a log record, never a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutomationLog {
    /// Stable run id: `<automation_id>-<unix_ms>`.
    pub run_id: String,
    /// The automation that produced this run.
    pub automation_id: String,
    /// Denormalized name at run time, so logs outlive renames/deletes.
    pub automation_name: String,
    /// `running` | `success` | `failed` | `stopped`.
    pub status: AutomationRunStatus,
    /// `manual` | `interval` | `cron` | `always_on` | `schedule`.
    pub trigger: AutomationRunTrigger,
    /// RFC3339 start timestamp.
    pub started_at: String,
    /// RFC3339 end timestamp (absent while running).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Wall-clock duration of the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Project the run was bound to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Model/provider actually used (empty = harness default).
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider_id: String,
    /// Final assistant answer, truncated to [`AUTOMATION_OUTPUT_CAP`] characters.
    #[serde(default)]
    pub output: String,
    /// Failure reason, when the run failed or was stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Telemetry counters (cheap to collect from the event stream).
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub tokens: u64,
}

impl AutomationLog {
    /// Start a new log entry for `automation`; `started_at` is RFC3339.
    pub fn start(
        automation: &Automation,
        run_id: String,
        trigger: AutomationRunTrigger,
        started_at: String,
    ) -> Self {
        Self {
            run_id,
            automation_id: automation.id.clone(),
            automation_name: automation.name.clone(),
            status: AutomationRunStatus::Running,
            trigger,
            started_at,
            finished_at: None,
            duration_ms: None,
            project_id: automation
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            model: automation.model.trim().to_string(),
            provider_id: automation.provider_id.trim().to_string(),
            output: String::new(),
            error: None,
            tool_calls: 0,
            tokens: 0,
        }
    }

    /// Truncate run output to [`AUTOMATION_OUTPUT_CAP`] characters.
    pub fn truncate_output(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.chars().count() <= AUTOMATION_OUTPUT_CAP {
            return trimmed.to_string();
        }
        let mut out: String = trimmed.chars().take(AUTOMATION_OUTPUT_CAP).collect();
        out.push('…');
        out
    }

    /// True while the run has not reached a terminal state.
    pub fn is_running(&self) -> bool {
        self.status == AutomationRunStatus::Running
    }
}

/// An edit to an existing automation.
///
/// Every field is optional: `None` means "leave as is". `project_id` uses a nested
/// `Option` so it can distinguish "keep" from "make standalone" (`Some(None)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutomationEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub prompt: Option<String>,
    /// `None` = keep, `Some(None)` = standalone, `Some(Some(id))` = bind to project.
    pub project_id: Option<Option<String>>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub enabled: Option<bool>,
    pub trigger: Option<AutomationTrigger>,
}

impl AutomationEdit {
    /// Parse an edit patch from JSON, accepting the same body shape as `create`.
    ///
    /// Unknown keys (including `id`) are ignored; malformed values are rejected with a
    /// user-facing error so create and edit share one code path.
    pub fn from_json(body: &serde_json::Value) -> Result<Self, String> {
        let obj = body
            .as_object()
            .ok_or_else(|| "Automation edit body must be a JSON object.".to_string())?;
        let text = |key: &str| -> Option<String> {
            obj.get(key)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
        };
        let mut edit = AutomationEdit::default();
        if obj.contains_key("name") {
            edit.name = text("name");
        }
        if obj.contains_key("description") {
            edit.description = text("description");
        }
        if obj.contains_key("prompt") {
            edit.prompt = text("prompt");
        }
        if obj.contains_key("provider_id") {
            edit.provider_id = Some(text("provider_id").unwrap_or_default());
        }
        if obj.contains_key("model") {
            edit.model = Some(text("model").unwrap_or_default());
        }
        if let Some(v) = obj.get("enabled") {
            edit.enabled = Some(
                v.as_bool()
                    .ok_or_else(|| "Field 'enabled' must be a boolean.".to_string())?,
            );
        }
        if obj.contains_key("project_id") {
            edit.project_id = Some(match obj.get("project_id") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(s)) => {
                    let t = s.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t.to_string())
                    }
                }
                Some(_) => {
                    return Err("Field 'project_id' must be a string or null.".to_string())
                }
            });
        }
        if let Some(v) = obj.get("trigger") {
            let trigger: AutomationTrigger = serde_json::from_value(v.clone())
                .map_err(|e| format!("Invalid 'trigger': {e}"))?;
            // Same validation the create path uses, so an edit can never save a
            // trigger the scheduler would reject at runtime.
            let probe = Automation {
                name: "probe".to_string(),
                prompt: "probe".to_string(),
                trigger: trigger.clone(),
                ..Default::default()
            };
            validate(&probe)?;
            edit.trigger = Some(trigger);
        }
        Ok(edit)
    }
}

/// A saved automation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Automation {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// What the agent should do each run.
    pub prompt: String,
    /// `None` = standalone (no project workspace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Optional model/provider override; empty = use the harness default.
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub trigger: AutomationTrigger,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub run_count: u64,
    /// `idle`, `running`, `completed`, `failed`, `stopped` — see
    /// [`AutomationRunStatus::last_status_str`].
    #[serde(default)]
    pub last_status: String,
}

fn default_true() -> bool {
    true
}

impl Automation {
    /// Whether this automation should fire at `now`, given whether it is already running.
    pub fn is_due(&self, now: chrono::DateTime<chrono::Local>, running: bool) -> bool {
        if !self.enabled || running {
            return false;
        }
        match &self.trigger {
            AutomationTrigger::Manual => false,
            AutomationTrigger::AlwaysOn => true,
            AutomationTrigger::Interval { every_secs } => {
                let every = (*every_secs).max(60) as i64;
                match self.last_run_at.as_deref().and_then(parse_ts) {
                    None => true,
                    Some(last) => (now - last).num_seconds() >= every,
                }
            }
            AutomationTrigger::Cron { expr } => {
                // Fire at most once per minute.
                if let Some(last) = self.last_run_at.as_deref().and_then(parse_ts) {
                    if (now - last).num_seconds() < 60 {
                        return false;
                    }
                }
                cron_matches(expr, &now)
            }
        }
    }

    /// Apply an edit in place, preserving the automation's identity (`id`,
    /// `created_at`) and its run history (`last_run_at`, `run_count`, `last_status`).
    pub fn apply_edit(&mut self, edit: &AutomationEdit) {
        if let Some(name) = edit.name.as_ref().filter(|n| !n.trim().is_empty()) {
            self.name = name.trim().to_string();
        }
        if let Some(description) = &edit.description {
            self.description = description.trim().to_string();
        }
        if let Some(prompt) = edit.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            self.prompt = prompt.trim().to_string();
        }
        if let Some(project) = &edit.project_id {
            self.project_id = project.clone();
        }
        if let Some(provider) = &edit.provider_id {
            self.provider_id = provider.trim().to_string();
        }
        if let Some(model) = &edit.model {
            self.model = model.trim().to_string();
        }
        if let Some(enabled) = edit.enabled {
            self.enabled = enabled;
        }
        if let Some(trigger) = &edit.trigger {
            self.trigger = trigger.clone();
        }
    }

    /// RFC3339 timestamp of the next scheduled fire, when there is one.
    ///
    /// `None` for disabled automations, manual triggers, and cron expressions that
    /// cannot match within the search horizon. `AlwaysOn` is always due, so it
    /// reports `now`.
    pub fn next_due_at(&self, now: chrono::DateTime<chrono::Local>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        match &self.trigger {
            AutomationTrigger::Manual => None,
            AutomationTrigger::AlwaysOn => Some(now.to_rfc3339()),
            AutomationTrigger::Interval { every_secs } => {
                let every = chrono::Duration::seconds((*every_secs).max(60) as i64);
                let base = match self.last_run_at.as_deref().and_then(parse_ts) {
                    Some(last) => last + every,
                    None => now,
                };
                Some(base.to_rfc3339())
            }
            AutomationTrigger::Cron { expr } => {
                use chrono::TimeZone;
                // Do not re-report the minute that just fired.
                let mut cursor = now;
                if let Some(last) = self.last_run_at.as_deref().and_then(parse_ts) {
                    if (now - last).num_seconds() < 60 {
                        cursor = now + chrono::Duration::minutes(1);
                    }
                }
                // Snap to the top of the minute, then scan up to 8 days forward.
                let secs = cursor.timestamp();
                let mut cursor = chrono::Local
                    .timestamp_opt(secs - (secs % 60), 0)
                    .single()
                    .unwrap_or(cursor);
                for _ in 0..(60 * 24 * 8) {
                    if cron_matches(expr, &cursor) {
                        return Some(cursor.to_rfc3339());
                    }
                    cursor += chrono::Duration::minutes(1);
                }
                None
            }
        }
    }
}

fn parse_ts(s: &str) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Local))
}

/// Match a 5-field cron expression (`min hour dom month dow`) against a time.
///
/// Supports `*`, `*/n`, `a`, `a-b`, `a-b/n` and comma lists. Day-of-week uses
/// 0=Sunday..6=Saturday (7 also accepted as Sunday).
pub fn cron_matches(expr: &str, now: &chrono::DateTime<chrono::Local>) -> bool {
    use chrono::{Datelike, Timelike};
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    let minute = now.minute() as u32;
    let hour = now.hour() as u32;
    let dom = now.day();
    let month = now.month();
    let dow = now.weekday().num_days_from_sunday();

    field_matches(fields[0], minute, 0, 59)
        && field_matches(fields[1], hour, 0, 23)
        && field_matches(fields[2], dom, 1, 31)
        && field_matches(fields[3], month, 1, 12)
        && field_matches(fields[4], dow, 0, 7)
}

fn field_matches(field: &str, value: u32, min: u32, max: u32) -> bool {
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).unwrap_or(1)),
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            match (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                (Ok(a), Ok(b)) => (a, b),
                _ => continue,
            }
        } else {
            match range.trim().parse::<u32>() {
                Ok(v) => (v, v),
                Err(_) => continue,
            }
        };
        if value >= lo && value <= hi && (value - lo) % step == 0 {
            return true;
        }
        // 7 == Sunday for day-of-week.
        if max == 7 && value == 0 && lo == 7 && hi == 7 {
            return true;
        }
    }
    false
}

/// Validate an automation definition, returning a user-facing error.
pub fn validate(automation: &Automation) -> Result<(), String> {
    if automation.name.trim().is_empty() {
        return Err("Automation name cannot be empty.".to_string());
    }
    if automation.prompt.trim().is_empty() {
        return Err("Automation prompt cannot be empty.".to_string());
    }
    match &automation.trigger {
        AutomationTrigger::Cron { expr } => {
            let fields: Vec<&str> = expr.split_whitespace().collect();
            if fields.len() != 5 {
                return Err(
                    "Cron expression must have 5 fields: minute hour day month weekday.".to_string(),
                );
            }
            for (f, (lo, hi)) in fields
                .iter()
                .zip([(0u32, 59u32), (0, 23), (1, 31), (1, 12), (0, 7)])
            {
                if !valid_field(f, lo, hi) {
                    return Err(format!("Invalid cron field '{f}'."));
                }
            }
        }
        AutomationTrigger::Interval { every_secs } => {
            if *every_secs < 60 {
                return Err("Interval must be at least 60 seconds.".to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

fn valid_field(field: &str, min: u32, max: u32) -> bool {
    field.split(',').all(|part| {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => match s.parse::<u32>() {
                Ok(v) if v > 0 => (r, v),
                _ => return false,
            },
            None => (part, 1),
        };
        let _ = step;
        if range == "*" {
            return true;
        }
        if let Some((a, b)) = range.split_once('-') {
            return match (a.parse::<u32>(), b.parse::<u32>()) {
                (Ok(a), Ok(b)) => a <= b && b <= max && a >= min,
                _ => false,
            };
        }
        range.parse::<u32>().map(|v| v >= min && v <= max).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Timelike};

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn cron_star_matches_every_minute() {
        assert!(cron_matches("* * * * *", &at(2026, 11, 27, 13, 37)));
    }

    #[test]
    fn cron_step_and_literal() {
        assert!(cron_matches("*/15 * * * *", &at(2026, 11, 27, 9, 30)));
        assert!(!cron_matches("*/15 * * * *", &at(2026, 11, 27, 9, 31)));
        assert!(cron_matches("0 9 * * *", &at(2026, 11, 27, 9, 0)));
        assert!(!cron_matches("0 9 * * *", &at(2026, 11, 27, 10, 0)));
    }

    #[test]
    fn cron_ranges_and_lists() {
        assert!(cron_matches("0 9-17 * * 1-5", &at(2026, 11, 27, 12, 0)));
        assert!(!cron_matches("0 9-17 * * 1-5", &at(2026, 11, 29, 12, 0)));
        assert!(cron_matches("0 0 1,15 * *", &at(2026, 11, 15, 0, 0)));
    }

    #[test]
    fn invalid_cron_rejected() {
        assert!(!cron_matches("* * *", &at(2026, 11, 27, 0, 0)));
        assert!(!cron_matches("* * * * 9", &at(2026, 11, 27, 0, 0)));
    }

    #[test]
    fn interval_due_logic() {
        let mut a = Automation {
            id: "a".into(),
            name: "A".into(),
            description: String::new(),
            prompt: "p".into(),
            project_id: None,
            provider_id: String::new(),
            model: String::new(),
            enabled: true,
            trigger: AutomationTrigger::Interval { every_secs: 600 },
            created_at: String::new(),
            last_run_at: None,
            run_count: 0,
            last_status: String::new(),
        };
        let now = at(2026, 11, 27, 12, 0);
        assert!(a.is_due(now, false), "never run => due");
        assert!(!a.is_due(now, true), "running => not due");
        a.last_run_at = Some((now - chrono::Duration::seconds(300)).to_rfc3339());
        assert!(!a.is_due(now, false), "too recent");
        a.last_run_at = Some((now - chrono::Duration::seconds(700)).to_rfc3339());
        assert!(a.is_due(now, false), "elapsed");
        a.enabled = false;
        assert!(!a.is_due(now, false), "disabled");
    }

    #[test]
    fn manual_and_always_on() {
        let now = at(2026, 11, 27, 12, 0);
        let mut a = Automation {
            id: "a".into(),
            name: "A".into(),
            description: String::new(),
            prompt: "p".into(),
            project_id: None,
            provider_id: String::new(),
            model: String::new(),
            enabled: true,
            trigger: AutomationTrigger::Manual,
            created_at: String::new(),
            last_run_at: None,
            run_count: 0,
            last_status: String::new(),
        };
        assert!(!a.is_due(now, false));
        a.trigger = AutomationTrigger::AlwaysOn;
        assert!(a.is_due(now, false));
    }

    #[test]
    fn validate_rules() {
        let mut a = Automation {
            id: "a".into(),
            name: "Watcher".into(),
            description: String::new(),
            prompt: "check".into(),
            project_id: None,
            provider_id: String::new(),
            model: String::new(),
            enabled: true,
            trigger: AutomationTrigger::Cron { expr: "*/10 * * * *".into() },
            created_at: String::new(),
            last_run_at: None,
            run_count: 0,
            last_status: String::new(),
        };
        assert!(validate(&a).is_ok());
        a.trigger = AutomationTrigger::Cron { expr: "bad".into() };
        assert!(validate(&a).is_err());
        a.trigger = AutomationTrigger::Interval { every_secs: 10 };
        assert!(validate(&a).is_err());
        a.trigger = AutomationTrigger::Manual;
        a.name = "  ".into();
        assert!(validate(&a).is_err());
    }

    fn sample(id: &str) -> Automation {
        Automation {
            id: id.into(),
            name: "Watcher".into(),
            description: "desc".into(),
            prompt: "check".into(),
            project_id: None,
            provider_id: String::new(),
            model: String::new(),
            enabled: true,
            trigger: AutomationTrigger::Cron { expr: "*/10 * * * *".into() },
            created_at: "2026-01-01T00:00:00+00:00".into(),
            last_run_at: None,
            run_count: 0,
            last_status: "idle".into(),
        }
    }

    #[test]
    fn edit_round_trip_preserves_identity_and_history() {
        let mut a = sample("watcher");
        a.project_id = Some("proj".into());
        a.run_count = 7;
        a.last_run_at = Some("2026-02-02T02:02:02+00:00".into());
        a.last_status = "completed".into();

        let edit = AutomationEdit::from_json(&serde_json::json!({
            "name": "  Renamed Watcher  ",
            "description": "new desc",
            "prompt": "check twice",
            "project_id": null,
            "model": "gpt-x",
            "provider_id": " openai ",
            "enabled": false,
            "trigger": { "kind": "interval", "every_secs": 900 },
            // Unknown/ignored keys must not break an edit.
            "id": "someone-else",
            "run_count": 999
        }))
        .expect("edit parses");
        a.apply_edit(&edit);

        assert_eq!(a.id, "watcher", "identity is preserved");
        assert_eq!(a.created_at, "2026-01-01T00:00:00+00:00");
        assert_eq!(a.name, "Renamed Watcher");
        assert_eq!(a.description, "new desc");
        assert_eq!(a.prompt, "check twice");
        assert_eq!(a.project_id, None, "null clears the project binding");
        assert_eq!(a.model, "gpt-x");
        assert_eq!(a.provider_id, "openai");
        assert!(!a.enabled);
        assert_eq!(a.trigger, AutomationTrigger::Interval { every_secs: 900 });
        assert_eq!(a.run_count, 7, "run history is preserved");
        assert_eq!(a.last_run_at.as_deref(), Some("2026-02-02T02:02:02+00:00"));
        assert_eq!(a.last_status, "completed");
        assert!(validate(&a).is_ok(), "an edited automation must still validate");
    }

    #[test]
    fn edit_absent_fields_keep_existing_values() {
        let mut a = sample("watcher");
        a.project_id = Some("proj".into());
        let edit = AutomationEdit::from_json(&serde_json::json!({ "enabled": false }))
            .expect("partial edit parses");
        a.apply_edit(&edit);
        assert_eq!(a.name, "Watcher");
        assert_eq!(a.prompt, "check");
        assert_eq!(a.project_id.as_deref(), Some("proj"));
        assert_eq!(a.trigger, AutomationTrigger::Cron { expr: "*/10 * * * *".into() });
        assert!(!a.enabled);
    }

    #[test]
    fn edit_rejects_malformed_fields() {
        assert!(AutomationEdit::from_json(&serde_json::json!({"enabled": "yes"})).is_err());
        assert!(AutomationEdit::from_json(&serde_json::json!({"project_id": 5})).is_err());
        assert!(AutomationEdit::from_json(&serde_json::json!({"trigger": {"kind":"cron","expr":"x"}})).is_err());
        assert!(AutomationEdit::from_json(&serde_json::json!([1, 2])).is_err());
    }

    #[test]
    fn edited_trigger_is_validated_by_the_shared_helper() {
        let mut a = sample("watcher");
        // The edit parser validates with the same helper as create, so an
        // out-of-range interval is rejected at parse time rather than later.
        let bad = AutomationEdit::from_json(&serde_json::json!({
            "trigger": { "kind": "interval", "every_secs": 5 }
        }));
        assert!(bad.is_err(), "a 5s interval must be rejected");
        // A valid trigger round-trips through the shared validation.
        let edit = AutomationEdit::from_json(&serde_json::json!({
            "trigger": { "kind": "interval", "every_secs": 120 }
        }))
        .expect("parses");
        a.apply_edit(&edit);
        assert!(validate(&a).is_ok());
        assert_eq!(a.trigger, AutomationTrigger::Interval { every_secs: 120 });
    }

    #[test]
    fn next_due_at_reports_interval_cron_and_manual() {
        let now = at(2026, 11, 27, 12, 0);

        let mut manual = sample("m");
        manual.trigger = AutomationTrigger::Manual;
        assert_eq!(manual.next_due_at(now), None);

        let mut interval = sample("i");
        interval.trigger = AutomationTrigger::Interval { every_secs: 900 };
        assert_eq!(interval.next_due_at(now), Some(now.to_rfc3339()));
        interval.last_run_at = Some((now - chrono::Duration::seconds(120)).to_rfc3339());
        assert_eq!(
            interval.next_due_at(now),
            Some((now + chrono::Duration::seconds(780)).to_rfc3339())
        );

        interval.enabled = false;
        assert_eq!(interval.next_due_at(now), None, "disabled => never due");
        interval.enabled = true;

        let mut cron = sample("c");
        cron.trigger = AutomationTrigger::Cron { expr: "*/15 * * * *".into() };
        cron.last_run_at = Some(now.to_rfc3339());
        let next = cron.next_due_at(now).expect("cron has a next slot");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next).unwrap();
        assert_eq!(parsed.minute(), 15);
        assert_eq!(parsed.second(), 0);

        // An expression that never matches has no next slot.
        cron.trigger = AutomationTrigger::Cron { expr: "0 0 30 2 *".into() };
        cron.last_run_at = None;
        assert_eq!(cron.next_due_at(now), None);
    }

    #[test]
    fn run_log_lifecycle_and_status_strings() {
        let auto = sample("watcher");
        let log = AutomationLog::start(
            &auto,
            "watcher-1700000000000".into(),
            AutomationRunTrigger::Cron,
            "2026-11-27T12:00:00+00:00".into(),
        );
        assert!(log.is_running());
        assert_eq!(log.status.as_str(), "running");
        assert_eq!(log.trigger.as_str(), "cron");
        assert_eq!(log.automation_name, "Watcher");
        assert_eq!(log.duration_ms, None);
        assert_eq!(log.output, "");

        assert_eq!(AutomationRunStatus::Success.last_status_str(), "completed");
        assert_eq!(AutomationRunStatus::Failed.last_status_str(), "failed");
        assert_eq!(AutomationRunStatus::Stopped.last_status_str(), "stopped");
        assert_eq!(AutomationRunStatus::Running.last_status_str(), "running");
        assert!(AutomationRunStatus::Success.is_terminal());
        assert!(!AutomationRunStatus::Running.is_terminal());

        assert_eq!(
            AutomationRunTrigger::from_trigger(&AutomationTrigger::AlwaysOn),
            AutomationRunTrigger::AlwaysOn
        );
        assert_eq!(
            AutomationRunTrigger::from_trigger(&AutomationTrigger::Manual),
            AutomationRunTrigger::Manual
        );
    }

    #[test]
    fn run_log_output_is_capped() {
        assert_eq!(AutomationLog::truncate_output("  hi  "), "hi");
        assert_eq!(AutomationLog::truncate_output(""), "");
        let long = "x".repeat(AUTOMATION_OUTPUT_CAP + 500);
        let capped = AutomationLog::truncate_output(&long);
        assert_eq!(capped.chars().count(), AUTOMATION_OUTPUT_CAP + 1);
        assert!(capped.ends_with('…'));

        // Multi-byte input must not panic on the char boundary.
        let emoji = "🙂".repeat(AUTOMATION_OUTPUT_CAP + 10);
        assert_eq!(
            AutomationLog::truncate_output(&emoji).chars().count(),
            AUTOMATION_OUTPUT_CAP + 1
        );
    }
}
