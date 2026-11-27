//! Running automations: turn a due automation into a live agent session.

use std::sync::Arc;

use crate::automation::{Automation, AutomationTrigger};
use crate::engine::context::ChatMessage;
use crate::state::AppState;

/// Automation ids currently executing, so `AlwaysOn` and slow runs cannot stack.
#[derive(Default)]
pub struct AutomationRuns {
    pub running: std::collections::HashSet<String>,
}

impl AppState {
    /// Run one automation now, creating a session for it.
    pub async fn run_automation(&self, automation: Automation) -> Result<String, String> {
        {
            let mut runs = self.automation_runs.lock().await;
            if !runs.running.insert(automation.id.clone()) {
                return Err("Automation is already running.".to_string());
            }
        }

        let session_id = format!(
            "auto-{}-{}",
            automation.id,
            chrono::Utc::now().timestamp_millis()
        );
        let project_id = automation.project_id.clone().unwrap_or_default();
        let title = format!("[{}] {}", automation.name, truncate(&automation.prompt, 60));

        let (bcast_tx, cancel_notify) = self
            .create_session_with_options(
                session_id.clone(),
                title,
                project_id.clone(),
                None,
                None,
                automation.model.clone().into_option(),
                "running",
            )
            .await;

        // Per-automation model override (provider + model).
        if !automation.model.trim().is_empty() || !automation.provider_id.trim().is_empty() {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                if !automation.model.trim().is_empty() {
                    session.model = automation.model.clone();
                }
                if !automation.provider_id.trim().is_empty() {
                    session.provider_id = automation.provider_id.clone();
                }
            }
        }

        let Some((_turn, prior, steer)) = self
            .begin_turn_with_command_and_attachments(&session_id, &automation.prompt, Vec::new(), None, None)
            .await
        else {
            let mut runs = self.automation_runs.lock().await;
            runs.running.remove(&automation.id);
            return Err("Failed to start the automation turn.".to_string());
        };

        crate::web::handlers::spawn_session_turn(
            self.clone(),
            session_id.clone(),
            automation.prompt.clone(),
            Vec::new(),
            project_id,
            prior,
            steer,
            bcast_tx,
            cancel_notify,
            false,
        );

        // Record the run and release the running slot once the session finishes.
        {
            let mut conf = self.config.write().await;
            if let Some(a) = conf.automations.iter_mut().find(|a| a.id == automation.id) {
                a.last_run_at = Some(chrono::Local::now().to_rfc3339());
                a.run_count = a.run_count.saturating_add(1);
                a.last_status = "running".to_string();
            }
        }
        self.persist_config().await;

        let state = self.clone();
        let automation_id = automation.id.clone();
        let watch_session = session_id.clone();
        tokio::spawn(async move {
            // Wait for the session to reach a terminal status, then free the slot.
            for _ in 0..720 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let status = {
                    let sessions = state.sessions.read().await;
                    sessions.get(&watch_session).map(|s| s.status.clone())
                };
                match status.as_deref() {
                    Some("running") | Some("waiting") => continue,
                    Some(done) => {
                        let mut conf = state.config.write().await;
                        if let Some(a) = conf.automations.iter_mut().find(|a| a.id == automation_id) {
                            a.last_status = done.to_string();
                        }
                        break;
                    }
                    None => break,
                }
            }
            let mut runs = state.automation_runs.lock().await;
            runs.running.remove(&automation_id);
        });

        Ok(session_id)
    }

    /// Fire every automation that is due. Called by the scheduler tick.
    pub async fn run_due_automations(&self) -> Vec<(String, String)> {
        let now = chrono::Local::now();
        let (automations, running): (Vec<Automation>, std::collections::HashSet<String>) = {
            let conf = self.config.read().await;
            let runs = self.automation_runs.lock().await;
            (conf.automations.clone(), runs.running.clone())
        };

        let mut started = Vec::new();
        for automation in automations {
            if !automation.is_due(now, running.contains(&automation.id)) {
                continue;
            }
            // AlwaysOn: avoid re-firing while a previous run is still active.
            if matches!(automation.trigger, AutomationTrigger::AlwaysOn)
                && automation.last_status == "running"
            {
                continue;
            }
            match self.run_automation(automation.clone()).await {
                Ok(session_id) => started.push((automation.id, session_id)),
                Err(e) => eprintln!("[harness-automation] '{}' failed: {e}", automation.name),
            }
        }
        started
    }
}

trait IntoOption {
    fn into_option(self) -> Option<String>;
}

impl IntoOption for String {
    fn into_option(self) -> Option<String> {
        if self.trim().is_empty() {
            None
        } else {
            Some(self)
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    let t = s.trim().replace('\n', " ");
    if t.chars().count() <= max {
        t
    } else {
        t.chars().take(max).collect::<String>() + "…"
    }
}

/// Background scheduler: ticks every 30s and fires due automations.
pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Small initial delay so startup work finishes first.
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        loop {
            state.run_due_automations().await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

/// Build a `ChatMessage` list placeholder so the module compiles without unused imports.
#[allow(dead_code)]
fn _unused(_: ChatMessage) {}
