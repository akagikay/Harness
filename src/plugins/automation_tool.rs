//! `manage_automations` meta-tool: lets the agent create, list, run, and remove
//! automations. Automations are first-class: standalone or project-bound, with
//! their own trigger and optional model override.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use super::contract::{OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission};
use super::{SessionToolContext, ToolPlugin};
use crate::automation::{Automation, AutomationTrigger};

pub struct ManageAutomationsPlugin;

fn parse_trigger(args: &Value) -> AutomationTrigger {
    let kind = args
        .get("trigger")
        .and_then(|v| v.as_str())
        .unwrap_or("manual")
        .trim()
        .to_ascii_lowercase();
    match kind.as_str() {
        "interval" => {
            let secs = args
                .get("every_secs")
                .and_then(|v| v.as_u64())
                .or_else(|| {
                    // Accept "30m" / "2h" style values.
                    args.get("every")
                        .and_then(|v| v.as_str())
                        .and_then(parse_duration_secs)
                })
                .unwrap_or(3600);
            AutomationTrigger::Interval { every_secs: secs.max(60) }
        }
        "cron" => AutomationTrigger::Cron {
            expr: args
                .get("cron")
                .and_then(|v| v.as_str())
                .unwrap_or("* * * * *")
                .trim()
                .to_string(),
        },
        "always_on" | "alwayson" | "always-on" => AutomationTrigger::AlwaysOn,
        _ => AutomationTrigger::Manual,
    }
}

fn parse_duration_secs(raw: &str) -> Option<u64> {
    let s = raw.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(v) = s.strip_suffix('s') {
        (v, 1)
    } else if let Some(v) = s.strip_suffix('m') {
        (v, 60)
    } else if let Some(v) = s.strip_suffix('h') {
        (v, 3600)
    } else if let Some(v) = s.strip_suffix('d') {
        (v, 86400)
    } else {
        (s.as_str(), 60)
    };
    num.trim().parse::<u64>().ok().map(|n| n * mult)
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        format!("auto-{}", chrono::Utc::now().timestamp_millis())
    } else {
        out
    }
}

#[async_trait]
impl ToolPlugin for ManageAutomationsPlugin {
    fn name(&self) -> &str {
        "manage_automations"
    }

    fn description(&self) -> &str {
        "Create, inspect, edit, run, enable/disable, or remove automations. An automation is a \
         saved prompt with a trigger (interval, cron, always-on, or manual) that runs as its own \
         detached agent run - the output goes to the automation's run log, NOT a conversation. \
         It can be standalone or bound to a project, and can override the model. \
         Use action=update to edit an existing automation in place (only the fields you pass \
         change). Read-only actions: 'logs' (recent runs), 'status' (is it running, next due, \
         recent summary), 'audit' (roll-up across every automation)."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: "manage_automations".to_string(),
            description: self.description().to_string(),
            category: "meta".to_string(),
            permission_level: ToolPermission::Meta,
            fields: vec![
                ToolFieldSpec::required_string(
                    "action",
                    "Operation: list, create, update (edit in place), run, enable, disable, \
                     remove, logs, status, or audit. The read-only actions (list, logs, \
                     status, audit) never start or change anything.",
                    "Use one of: list, create, update, run, enable, disable, remove, logs, \
                     status, audit.",
                )
                .with_enum([
                    "list",
                    "create",
                    "update",
                    "edit",
                    "run",
                    "enable",
                    "disable",
                    "remove",
                    "logs",
                    "status",
                    "audit",
                ])
                .non_empty(),
                ToolFieldSpec::optional_string("id", "Automation id (for run/enable/disable/remove)."),
                ToolFieldSpec::optional_string(
                    "name",
                    "Display name (required for create). Also used to derive the id.",
                ),
                ToolFieldSpec::optional_string(
                    "prompt",
                    "What the agent should do on each run (required for create).",
                ),
                ToolFieldSpec::optional_string(
                    "trigger",
                    "Trigger type: manual, interval, cron, or always_on.",
                )
                .with_enum(["manual", "interval", "cron", "always_on"])
                .with_default(json!("manual")),
                ToolFieldSpec::optional_int(
                    "every_secs",
                    "Interval in seconds (minimum 60) when trigger=interval.",
                ),
                ToolFieldSpec::optional_string(
                    "every",
                    "Human interval such as '30m' or '2h' (alternative to every_secs).",
                ),
                ToolFieldSpec::optional_string(
                    "cron",
                    "5-field cron expression (minute hour day month weekday) when trigger=cron.",
                ),
                ToolFieldSpec::optional_string(
                    "project_id",
                    "Project to bind to; omit for a standalone automation.",
                ),
                ToolFieldSpec::optional_string("model", "Optional model override."),
                ToolFieldSpec::optional_string("provider_id", "Optional provider id override."),
                ToolFieldSpec::optional_string("description", "Short description."),
            ],
            expected_output: OutputContract::new(
                "json",
                "JSON with the automation list or the created/updated automation.",
                r#"{"status":"created","automation":{"id":"github-repo-watcher","name":"GitHub Repo Watcher"}}"#,
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_field",
                    "create without name or prompt",
                    "Missing required parameter",
                    "Provide both 'name' and 'prompt'.",
                ),
                ToolFailureMode::new(
                    "not_found",
                    "run/enable/disable/remove with an unknown id",
                    "Automation not found",
                    "Call action=list to see valid ids.",
                ),
                ToolFailureMode::new(
                    "invalid_trigger",
                    "cron expression is not 5 fields",
                    "Invalid cron expression",
                    "Use 5 fields, e.g. '*/10 * * * *'.",
                ),
            ],
            timeout_secs: 30,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, _sandbox: &Path) -> Result<String, String> {
        Err(format!(
            "manage_automations requires session context (action: {})",
            args.get("action").and_then(|v| v.as_str()).unwrap_or("?")
        ))
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let state = ctx
            .and_then(|c| c.state.clone())
            .ok_or_else(|| "manage_automations is unavailable without application state.".to_string())?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .trim()
            .to_ascii_lowercase();

        match action.as_str() {
            "list" => {
                let conf = state.config.read().await;
                Ok(serde_json::to_string_pretty(&json!({
                    "automations": conf.automations,
                }))
                .unwrap_or_default())
            }
            "create" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Missing required parameter 'name'.".to_string())?;
                let prompt = args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Missing required parameter 'prompt'.".to_string())?;

                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| slugify(name));

                let automation = Automation {
                    id: id.clone(),
                    name: name.to_string(),
                    description: args
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    prompt: prompt.to_string(),
                    project_id: args
                        .get("project_id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string()),
                    provider_id: args
                        .get("provider_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    model: args
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    enabled: true,
                    trigger: parse_trigger(&args),
                    created_at: chrono::Local::now().to_rfc3339(),
                    last_run_at: None,
                    run_count: 0,
                    last_status: "idle".to_string(),
                };
                crate::automation::validate(&automation)?;

                let replaced = {
                    let mut conf = state.config.write().await;
                    if let Some(existing) = conf.automations.iter_mut().find(|a| a.id == id) {
                        // Preserve run history when updating an existing automation.
                        let last_run = existing.last_run_at.clone();
                        let run_count = existing.run_count;
                        *existing = automation.clone();
                        existing.last_run_at = last_run;
                        existing.run_count = run_count;
                        true
                    } else {
                        conf.automations.push(automation.clone());
                        false
                    }
                };
                state.persist_config().await;
                Ok(serde_json::to_string_pretty(&json!({
                    "status": if replaced { "updated" } else { "created" },
                    "automation": automation,
                    "trigger": automation.trigger.describe(),
                }))
                .unwrap_or_default())
            }
            "run" => {
                let id = require_id(&args)?;
                let automation = {
                    let conf = state.config.read().await;
                    conf.automations
                        .iter()
                        .find(|a| a.id == id)
                        .cloned()
                        .ok_or_else(|| format!("Automation '{id}' not found."))?
                };
                let session_id = state.run_automation(automation).await?;
                Ok(json!({ "status": "started", "session_id": session_id }).to_string())
            }
            "enable" | "disable" => {
                let id = require_id(&args)?;
                let want = action == "enable";
                let found = {
                    let mut conf = state.config.write().await;
                    match conf.automations.iter_mut().find(|a| a.id == id) {
                        Some(a) => {
                            a.enabled = want;
                            true
                        }
                        None => false,
                    }
                };
                if !found {
                    return Err(format!("Automation '{id}' not found."));
                }
                state.persist_config().await;
                Ok(json!({ "status": if want { "enabled" } else { "disabled" }, "id": id }).to_string())
            }
            "remove" => {
                let id = require_id(&args)?;
                let removed = {
                    let mut conf = state.config.write().await;
                    let before = conf.automations.len();
                    conf.automations.retain(|a| a.id != id);
                    conf.automations.len() != before
                };
                if !removed {
                    return Err(format!("Automation '{id}' not found."));
                }
                state.persist_config().await;
                Ok(json!({ "status": "removed", "id": id }).to_string())
            }
            // ---- edit an existing automation in place ----
            "update" | "edit" => {
                let id = require_id(&args)?;
                let edit = crate::automation::AutomationEdit::from_json(&args)
                    .map_err(|e| e)?;
                let updated = {
                    let mut conf = state.config.write().await;
                    let Some(existing) = conf.automations.iter_mut().find(|a| a.id == id) else {
                        return Err(format!("Automation '{id}' not found."));
                    };
                    existing.apply_edit(&edit);
                    existing.clone()
                };
                crate::automation::validate(&updated)?;
                state.persist_config().await;
                Ok(json!({
                    "status": "updated",
                    "automation": updated,
                    "trigger": updated.trigger.describe()
                })
                .to_string())
            }
            // ---- read-only inspection for the agent ----
            "logs" => {
                let id = require_id(&args)?;
                let logs = state.automation_logs_for(&id, 50).await;
                Ok(serde_json::to_string_pretty(&json!({
                    "automation_id": id,
                    "runs": logs,
                    "summary": crate::automation::summarize_logs(&logs)
                }))
                .unwrap_or_default())
            }
            "status" => {
                let id = require_id(&args)?;
                let (automation, running) = {
                    let conf = state.config.read().await;
                    let a = conf
                        .automations
                        .iter()
                        .find(|a| a.id == id)
                        .cloned()
                        .ok_or_else(|| format!("Automation '{id}' not found."))?;
                    (
                        a,
                        state
                            .all_running_run_ids()
                            .await
                            .get(&id)
                            .cloned()
                            .unwrap_or_default(),
                    )
                };
                let logs = state.automation_logs_for(&id, 10).await;
                Ok(json!({
                    "id": id,
                    "enabled": automation.enabled,
                    "trigger": automation.trigger.describe(),
                    "last_status": automation.last_status,
                    "last_run_at": automation.last_run_at,
                    "run_count": automation.run_count,
                    "is_running": !running.is_empty(),
                    "running_run_ids": running,
                    "next_due_at": automation.next_due_at(chrono::Local::now()),
                    "recent": crate::automation::summarize_logs(&logs)
                })
                .to_string())
            }
            "audit" => {
                // Read-only roll-up across every automation.
                let (conf, logs) = {
                    let conf = state.config.read().await.clone();
                    let logs = state.automation_logs(200).await;
                    (conf, logs)
                };
                let per: Vec<Value> = conf
                    .automations
                    .iter()
                    .map(|a| {
                        let own: Vec<_> = logs
                            .iter()
                            .filter(|l| l.automation_id == a.id)
                            .cloned()
                            .collect();
                        json!({
                            "id": a.id,
                            "name": a.name,
                            "enabled": a.enabled,
                            "trigger": a.trigger.describe(),
                            "last_status": a.last_status,
                            "is_running": own.iter().any(|l| l.is_running()),
                            "summary": crate::automation::summarize_logs(&own)
                        })
                    })
                    .collect();
                Ok(serde_json::to_string_pretty(&json!({
                    "automations": per,
                    "overall": crate::automation::summarize_logs(&logs),
                    "sample_size": logs.len()
                }))
                .unwrap_or_default())
            }
            other => Err(format!(
                "Unknown action '{other}'. Use list, create, update, run, enable, disable, \
                 remove, logs, status, or audit."
            )),
        }
    }
}

fn require_id(args: &Value) -> Result<String, String> {
    args.get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing required parameter 'id'.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_parsing() {
        assert_eq!(parse_duration_secs("30m"), Some(1800));
        assert_eq!(parse_duration_secs("2h"), Some(7200));
        assert_eq!(parse_duration_secs("45s"), Some(45));
        assert_eq!(parse_duration_secs("1d"), Some(86400));
    }

    #[test]
    fn trigger_parsing() {
        let t = parse_trigger(&json!({"trigger":"interval","every":"15m"}));
        assert_eq!(t, AutomationTrigger::Interval { every_secs: 900 });
        let t = parse_trigger(&json!({"trigger":"cron","cron":"*/10 * * * *"}));
        assert!(matches!(t, AutomationTrigger::Cron { .. }));
        let t = parse_trigger(&json!({"trigger":"always_on"}));
        assert_eq!(t, AutomationTrigger::AlwaysOn);
        // Below the minimum is clamped up.
        let t = parse_trigger(&json!({"trigger":"interval","every_secs":5}));
        assert_eq!(t, AutomationTrigger::Interval { every_secs: 60 });
    }

    #[test]
    fn slugify_names() {
        assert_eq!(slugify("GitHub Repo Watcher"), "github-repo-watcher");
    }
}
