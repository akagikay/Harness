//! First-class automations: scheduled agent runs, standalone or bound to a project.
//!
//! An automation is a saved prompt plus a trigger and an optional model override.
//! When due, the scheduler creates a session, starts a turn, and runs it with the
//! normal agent loop — so automations get the same tools, permissions and
//! persistence as a manual conversation.

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

/// A saved automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// `idle`, `running`, `ok`, `error: ...`
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
    use chrono::TimeZone;

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
}
