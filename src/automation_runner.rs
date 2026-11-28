//! Running automations: turn a due automation into a **detached agent run**.
//!
//! An automation run is deliberately *not* a conversation. It reuses the normal agent
//! loop (`AgentLoopRunner`) but with a run id instead of a session id, so it never enters
//! `AppState::sessions`, never shows up in `GET /api/sessions`, and never writes a
//! conversation row. What survives a run is an [`AutomationLog`] entry plus the
//! automation's `last_status` / `run_count` bookkeeping.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, Notify};

use crate::automation::{
    Automation, AutomationLog, AutomationRunStatus, AutomationRunTrigger, AutomationTrigger,
    AUTOMATION_LOG_PER_AUTOMATION_CAP, AUTOMATION_LOG_TOTAL_CAP,
};
use crate::config::HarnessConfig;
use crate::engine::events::HarnessEvent;
use crate::engine::loop_runner::AgentLoopRunner;
use crate::models::google_search_client::is_google_search_model;
use crate::models::{mock_client::MockClient, openai_client::OpenAiClient, ModelAdapter};
use crate::state::AppState;

/// Automation ids currently executing, plus the in-memory run-log mirror.
///
/// `AutomationRuns` lives behind `AppState::automation_runs`; the log mirror, live
/// progress broadcasters and cancellation notifiers for detached runs hang off it so a
/// run can be observed and stopped without ever becoming a session.
#[derive(Default)]
pub struct AutomationRuns {
    /// Automation ids currently executing, so `AlwaysOn` and slow runs cannot stack.
    pub running: HashSet<String>,
    /// In-memory mirror of the run log, newest first.
    pub logs: Vec<AutomationLog>,
    /// Cancellation notifier per running run id.
    pub cancel: HashMap<String, Arc<Notify>>,
    /// Live progress broadcaster per run id (dropped when the run finishes).
    pub live: HashMap<String, broadcast::Sender<HarnessEvent>>,
    /// True once the log mirror has been hydrated from SQLite.
    hydrated: bool,
}

impl AutomationRuns {
    /// Reserve a slot for `automation_id`; `false` when a run is already in flight.
    pub fn insert_running(&mut self, automation_id: &str) -> bool {
        self.running.insert(automation_id.to_string())
    }

    /// Release the slot for a finished run.
    pub fn release(&mut self, automation_id: &str, run_id: &str) {
        self.running.remove(automation_id);
        self.cancel.remove(run_id);
        self.live.remove(run_id);
    }

    /// Insert a log entry keeping the list newest-first.
    pub fn push_log(&mut self, log: AutomationLog) {
        self.logs.insert(0, log);
    }

    /// Most recent `limit` entries, optionally filtered by automation.
    pub fn logs(&self, automation_id: Option<&str>, limit: usize) -> Vec<AutomationLog> {
        self.logs
            .iter()
            .filter(|l| automation_id.map_or(true, |id| l.automation_id == id))
            .take(limit.max(1))
            .cloned()
            .collect()
    }

    /// Mutable access to one entry.
    pub fn log_mut(&mut self, run_id: &str) -> Option<&mut AutomationLog> {
        self.logs.iter_mut().find(|l| l.run_id == run_id)
    }

    /// Enforce the ring-buffer caps in memory (mirrors the SQL prune).
    pub fn prune_logs(&mut self) {
        if self.logs.len() > AUTOMATION_LOG_TOTAL_CAP {
            self.logs.truncate(AUTOMATION_LOG_TOTAL_CAP);
        }
        let mut per_automation: HashMap<String, usize> = HashMap::new();
        let mut kept: Vec<AutomationLog> = Vec::with_capacity(self.logs.len());
        for log in self.logs.drain(..) {
            let seen = per_automation.entry(log.automation_id.clone()).or_insert(0);
            if *seen < AUTOMATION_LOG_PER_AUTOMATION_CAP {
                *seen += 1;
                kept.push(log);
            }
        }
        self.logs = kept;
    }

    /// Run ids of the still-running entries of one automation.
    pub fn running_run_ids(&self, automation_id: &str) -> Vec<String> {
        self.logs
            .iter()
            .filter(|l| l.automation_id == automation_id && l.is_running())
            .map(|l| l.run_id.clone())
            .collect()
    }
}

/// Fields an automation-log update may change. `None` = leave untouched.
#[derive(Debug, Clone, Default)]
pub struct AutomationLogUpdate {
    pub status: Option<AutomationRunStatus>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub tool_calls: Option<u64>,
    pub tokens: Option<u64>,
}

impl AppState {
    /// Hydrate the in-memory log mirror from SQLite on first use.
    pub async fn ensure_automation_logs_loaded(&self) {
        let needed = {
            let runs = self.automation_runs.lock().await;
            !runs.hydrated
        };
        if !needed {
            return;
        }
        let loaded = match self.db {
            Some(ref db) => db.list_automation_logs(None, AUTOMATION_LOG_TOTAL_CAP),
            None => Ok(Vec::new()),
        };
        let mut runs = self.automation_runs.lock().await;
        match loaded {
            Ok(rows) => {
                runs.logs = rows;
                runs.hydrated = true;
            }
            Err(e) => {
                eprintln!("[harness-db] failed to load automation logs: {e}");
                runs.hydrated = true;
            }
        }
    }

    /// Record the start of a run: insert the entry (status `running`) and persist it.
    pub async fn record_automation_log_start(&self, log: AutomationLog) {
        self.ensure_automation_logs_loaded().await;
        {
            let mut runs = self.automation_runs.lock().await;
            runs.push_log(log.clone());
        }
        if let Some(ref db) = self.db {
            if let Err(e) = db.upsert_automation_log(&log) {
                eprintln!("[harness-db] failed to persist automation log '{}': {e}", log.run_id);
            }
        }
    }

    /// Update a run entry, stamping `finished_at`/`duration_ms` when it terminates.
    pub async fn update_automation_log(&self, run_id: &str, update: AutomationLogUpdate) {
        self.ensure_automation_logs_loaded().await;
        let updated = {
            let mut runs = self.automation_runs.lock().await;
            let Some(log) = runs.log_mut(run_id) else {
                return;
            };
            if let Some(status) = update.status {
                log.status = status;
            }
            if let Some(output) = update.output {
                log.output = AutomationLog::truncate_output(&output);
            }
            if update.error.is_some() {
                log.error = update.error.clone();
            }
            if let Some(tool_calls) = update.tool_calls {
                log.tool_calls = tool_calls;
            }
            if let Some(tokens) = update.tokens {
                log.tokens = tokens;
            }
            if log.status.is_terminal() && log.finished_at.is_none() {
                log.finished_at = Some(chrono::Local::now().to_rfc3339());
                log.duration_ms = Some(elapsed_ms(&log.started_at));
            }
            log.clone()
        };
        if let Some(ref db) = self.db {
            if let Err(e) = db.upsert_automation_log(&updated) {
                eprintln!(
                    "[harness-db] failed to persist automation log '{}': {e}",
                    updated.run_id
                );
            }
        }
        self.prune_automation_logs().await;
    }

    /// Most recent run logs across every automation, newest first.
    pub async fn automation_logs(&self, limit: usize) -> Vec<AutomationLog> {
        self.ensure_automation_logs_loaded().await;
        let runs = self.automation_runs.lock().await;
        runs.logs(None, limit)
    }

    /// Most recent run logs for one automation, newest first.
    pub async fn automation_logs_for(&self, automation_id: &str, limit: usize) -> Vec<AutomationLog> {
        self.ensure_automation_logs_loaded().await;
        let runs = self.automation_runs.lock().await;
        runs.logs(Some(automation_id), limit)
    }

    /// One run entry by id.
    pub async fn automation_log(&self, run_id: &str) -> Option<AutomationLog> {
        self.ensure_automation_logs_loaded().await;
        let runs = self.automation_runs.lock().await;
        runs.logs.iter().find(|l| l.run_id == run_id).cloned()
    }

    /// Subscribe to a run's live event stream (only while it is running).
    pub async fn automation_run_receiver(
        &self,
        run_id: &str,
    ) -> Option<broadcast::Receiver<HarnessEvent>> {
        self.ensure_automation_logs_loaded().await;
        let runs = self.automation_runs.lock().await;
        runs.live.get(run_id).map(|tx| tx.subscribe())
    }

    /// Whether an automation currently has a run in flight.
    pub async fn automation_is_running(&self, automation_id: &str) -> bool {
        let runs = self.automation_runs.lock().await;
        runs.running.contains(automation_id)
    }

    /// Run ids of the in-flight runs of every automation, in one lock acquisition.
    ///
    /// Lets a whole list be rendered from a single await instead of one per row.
    pub async fn all_running_run_ids(&self) -> std::collections::HashMap<String, Vec<String>> {
        let runs = self.automation_runs.lock().await;
        let mut out: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for log in runs.logs.iter().filter(|l| l.is_running()) {
            out.entry(log.automation_id.clone())
                .or_default()
                .push(log.run_id.clone());
        }
        out
    }

    /// Cancel the in-flight run(s) of one automation, returning the stopped run ids.
    pub async fn stop_automation(&self, automation_id: &str) -> Result<Vec<String>, String> {
        let targets: Vec<(String, Arc<Notify>)> = {
            let runs = self.automation_runs.lock().await;
            runs.running_run_ids(automation_id)
                .into_iter()
                .filter_map(|run_id| {
                    runs.cancel
                        .get(&run_id)
                        .map(|n| (run_id.clone(), n.clone()))
                })
                .collect()
        };
        if targets.is_empty() {
            return Err(format!("Automation '{automation_id}' is not running."));
        }
        for (_, notify) in &targets {
            // notify_one() stores a permit when no waiter is parked, so a Stop issued
            // between loop iterations still cancels the next one.
            notify.notify_one();
        }
        for (run_id, _) in &targets {
            self.update_automation_log(
                run_id,
                AutomationLogUpdate {
                    status: Some(AutomationRunStatus::Stopped),
                    ..Default::default()
                },
            )
            .await;
        }
        Ok(targets.into_iter().map(|(run_id, _)| run_id).collect())
    }

    /// Apply the ring-buffer caps to the in-memory mirror and the database.
    pub async fn prune_automation_logs(&self) {
        if let Some(ref db) = self.db {
            if let Err(e) = db.prune_automation_logs(
                AUTOMATION_LOG_PER_AUTOMATION_CAP,
                AUTOMATION_LOG_TOTAL_CAP,
            ) {
                eprintln!("[harness-db] failed to prune automation logs: {e}");
            }
        }
        let mut runs = self.automation_runs.lock().await;
        runs.prune_logs();
    }

    /// Run one automation now, logging the run instead of creating a conversation.
    pub async fn run_automation(&self, automation: Automation) -> Result<String, String> {
        let trigger = AutomationRunTrigger::from_trigger(&automation.trigger);
        self.run_automation_with_trigger(automation, trigger).await
    }

    /// Run one automation with an explicit trigger label (used by the scheduler).
    pub async fn run_automation_with_trigger(
        &self,
        automation: Automation,
        trigger: AutomationRunTrigger,
    ) -> Result<String, String> {
        {
            let mut runs = self.automation_runs.lock().await;
            if !runs.insert_running(&automation.id) {
                return Err("Automation is already running.".to_string());
            }
        }

        let started_at = chrono::Local::now().to_rfc3339();
        let run_id = format!(
            "auto-{}-{}",
            automation.id,
            chrono::Utc::now().timestamp_millis()
        );
        let log = AutomationLog::start(&automation, run_id.clone(), trigger, started_at.clone());

        // Log + bookkeeping first, so callers see the run before the turn finishes.
        self.record_automation_log_start(log).await;
        {
            let mut conf = self.config.write().await;
            if let Some(a) = conf.automations.iter_mut().find(|a| a.id == automation.id) {
                a.last_run_at = Some(started_at);
                a.run_count = a.run_count.saturating_add(1);
                a.last_status = AutomationRunStatus::Running.last_status_str().to_string();
            }
        }
        self.persist_config().await;

        // Register the run's cancellation + live-progress channels before spawning so
        // `stop_automation` can reach a run that has not reached its first await yet.
        let cancel_notify = Arc::new(Notify::new());
        let (live_tx, _) = broadcast::channel::<HarnessEvent>(256);
        {
            let mut runs = self.automation_runs.lock().await;
            runs.cancel.insert(run_id.clone(), cancel_notify.clone());
            runs.live.insert(run_id.clone(), live_tx.clone());
        }
        Self::spawn_automation_turn(
            self.clone(),
            automation,
            run_id.clone(),
            cancel_notify,
            live_tx,
        );

        Ok(run_id)
    }

    /// Fire every automation that is due. Called by the scheduler tick.
    pub async fn run_due_automations(&self) -> Vec<(String, String)> {
        let now = chrono::Local::now();
        let (automations, running): (Vec<Automation>, HashSet<String>) = {
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
                Ok(run_id) => started.push((automation.id, run_id)),
                Err(e) => eprintln!("[harness-automation] '{}' failed: {e}", automation.name),
            }
        }
        started
    }

    /// Spawn the detached turn for one run: agent loop without a session.
    ///
    /// `cancel_notify` and `live_tx` are pre-registered by the caller so a Stop issued
    /// between spawning and the first loop iteration is still honoured.
    fn spawn_automation_turn(
        state: AppState,
        automation: Automation,
        run_id: String,
        cancel_notify: Arc<Notify>,
        live_tx: broadcast::Sender<HarnessEvent>,
    ) {
        tokio::spawn(async move {
            let config = { state.config.read().await.clone() };
            let use_mock = config.mock_mode;
            let project_id = automation
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let project_path = match project_id.as_deref() {
                Some(pid) => config
                    .projects
                    .iter()
                    .find(|p| p.id == pid)
                    .map(|p| p.path.clone())
                    .unwrap_or_else(|| config.sandbox_root.clone()),
                None => config.sandbox_root.clone(),
            };

            let mut run_config = config.clone();
            let (model, provider) = resolve_run_model_provider(&config, &automation);
            run_config.model = model;
            run_config.provider = provider;
            run_config.sandbox_root = project_path.clone();
            run_config.knowledge_only = match project_id.as_deref() {
                // Standalone automations (no project) run in the workspace root,
                // which is never knowledge-only; the helper also treats an empty
                // id that way.
                Some(pid) => crate::config::is_knowledge_only_project(pid),
                None => false,
            };
            if run_config.knowledge_only {
                run_config.permissions.mode = crate::config::PermissionMode::ReadOnly;
            }
            // A detached run has no session row, so project folders are granted as
            // explicit extra roots instead of being derived from `sessions`.
            if let Some(pid) = project_id.as_deref() {
                if let Some(proj) = config.projects.iter().find(|p| p.id == pid) {
                    for root in proj.all_paths() {
                        if !run_config.extra_access_roots.contains(&root) {
                            run_config.extra_access_roots.push(root);
                        }
                    }
                }
            }

            let registry = state.get_registry_snapshot().await;
            let model_adapter: Arc<dyn ModelAdapter> = if use_mock {
                Arc::new(MockClient)
            } else if is_google_search_model(&run_config.model) {
                Arc::new(
                    crate::models::google_search_client::GoogleSearchClient::new(),
                )
            } else {
                Arc::new(OpenAiClient::new(run_config.clone()))
            };

            let runner = AgentLoopRunner::new(
                run_id.clone(),
                automation.prompt.clone(),
                run_config,
                registry,
                model_adapter,
                Some(state.clone()),
            )
            .with_cancel_notify(cancel_notify.clone());

            let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(100);
            let runner_handle = tokio::spawn(async move { runner.run_with_messages(event_tx).await });

            // Drain events while the loop runs: forward them to the live channel and
            // collect cheap telemetry counters.
            let mut tool_calls = 0u64;
            let mut tokens = 0u64;
            while let Some(event) = event_rx.recv().await {
                match &event {
                    HarnessEvent::ToolCallStarted { .. } => tool_calls += 1,
                    HarnessEvent::TokenUsageUpdate { total_tokens, .. } => {
                        tokens = *total_tokens as u64;
                    }
                    _ => {}
                }
                let _ = live_tx.send(event);
            }

            let (status, output, error) = match runner_handle.await {
                Ok(Ok((answer, _msgs))) => {
                    if answer.trim() == "(Session stopped by user)" {
                        (AutomationRunStatus::Stopped, answer, None)
                    } else {
                        (AutomationRunStatus::Success, answer, None)
                    }
                }
                Ok(Err(err)) => (AutomationRunStatus::Failed, String::new(), Some(err)),
                Err(join_err) => (
                    AutomationRunStatus::Failed,
                    String::new(),
                    Some(format!("Automation run task failed: {join_err}")),
                ),
            };
            // An explicit stop already finalized the entry: keep `stopped`.
            let status = match state.automation_log(&run_id).await {
                Some(log) if log.status == AutomationRunStatus::Stopped => {
                    AutomationRunStatus::Stopped
                }
                _ => status,
            };

            state
                .update_automation_log(
                    &run_id,
                    AutomationLogUpdate {
                        status: Some(status),
                        output: Some(output),
                        error,
                        tool_calls: Some(tool_calls),
                        tokens: Some(tokens),
                    },
                )
                .await;

            {
                let mut conf = state.config.write().await;
                if let Some(a) = conf.automations.iter_mut().find(|a| a.id == automation.id) {
                    a.last_status = status.last_status_str().to_string();
                }
            }
            state.persist_config().await;
            // A detached run keys its SwarmBoard by the run id (there is no
            // conversation). Without this, every scheduled run leaked one board
            // (and its message history) for the life of the process.
            state.remove_swarm_board(&run_id).await;
            {
                let mut runs = state.automation_runs.lock().await;
                runs.release(&automation.id, &run_id);
            }
        });
    }
}

/// Resolve the (model, provider) a run should use.
///
/// Precedence mirrors the session path: automation override > project defaults >
/// orchestrator > harness default. Mock mode always resolves to the mock client.
fn resolve_run_model_provider(config: &HarnessConfig, automation: &Automation) -> (String, String) {
    let project_defaults = automation
        .project_id
        .as_deref()
        .and_then(|pid| config.projects.iter().find(|p| p.id == pid))
        .map(|p| p.defaults.clone())
        .unwrap_or_default();
    let has_model_override = !automation.model.trim().is_empty();
    let has_provider_override = !automation.provider_id.trim().is_empty();
    let orch = config.orchestrator.enabled;

    let model = if has_model_override {
        automation.model.trim().to_string()
    } else if !project_defaults.model.trim().is_empty() {
        project_defaults.model.clone()
    } else if orch && !config.orchestrator.model.trim().is_empty() {
        config.orchestrator.model.clone()
    } else {
        config.model.clone()
    };
    let provider = if has_provider_override {
        automation.provider_id.trim().to_string()
    } else if !project_defaults.provider_id.trim().is_empty() && !has_model_override {
        project_defaults.provider_id.clone()
    } else if orch && !config.orchestrator.provider_id.trim().is_empty() && !has_model_override {
        config.orchestrator.provider_id.clone()
    } else {
        config.provider.clone()
    };
    (model, provider)
}

/// Milliseconds between an RFC3339 timestamp and now (0 when unparseable).
fn elapsed_ms(started_at: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(started_at)
        .map(|start| {
            let now = chrono::Utc::now();
            (now - start.with_timezone(&chrono::Utc))
                .num_milliseconds()
                .max(0) as u64
        })
        .unwrap_or(0)
}

/// Background scheduler: ticks every 30s and fires due automations.
pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Small initial delay so startup work finishes first.
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            state.run_due_automations().await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::AutomationRunStatus as Status;

    fn sample_automation() -> Automation {
        Automation {
            id: "watcher".into(),
            name: "Repo Watcher".into(),
            description: String::new(),
            prompt: "Check the repo.".into(),
            project_id: None,
            provider_id: String::new(),
            model: String::new(),
            enabled: true,
            trigger: AutomationTrigger::Cron { expr: "*/10 * * * *".into() },
            created_at: String::new(),
            last_run_at: None,
            run_count: 0,
            last_status: "idle".into(),
        }
    }

    fn log_with(id: &str, automation_id: &str) -> AutomationLog {
        AutomationLog {
            run_id: id.into(),
            automation_id: automation_id.into(),
            automation_name: automation_id.into(),
            status: Status::Success,
            trigger: crate::automation::AutomationRunTrigger::Manual,
            started_at: "2026-11-27T12:00:00+00:00".into(),
            finished_at: Some("2026-11-27T12:00:05+00:00".into()),
            duration_ms: Some(5000),
            project_id: None,
            model: String::new(),
            provider_id: String::new(),
            output: "ok".into(),
            error: None,
            tool_calls: 1,
            tokens: 10,
        }
    }

    fn mock_state(tag: &str) -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("harness-auto-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        cfg.sandbox_root = dir.clone();
        (AppState::new(cfg), dir)
    }

    #[test]
    fn runs_mirror_keeps_logs_newest_first_and_bounded() {
        let mut runs = AutomationRuns::default();
        runs.push_log(log_with("a", "auto"));
        runs.push_log(log_with("b", "auto"));
        runs.push_log(log_with("c", "other"));
        assert_eq!(
            runs.logs.iter().map(|l| l.run_id.as_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"],
            "newest run is first"
        );

        assert_eq!(runs.logs(Some("auto"), 10).len(), 2);
        assert_eq!(runs.logs(Some("auto"), 1).len(), 1);
        assert_eq!(runs.logs(None, 2).len(), 2);
    }

    #[test]
    fn ring_buffer_caps_per_automation_and_overall() {
        let mut runs = AutomationRuns::default();
        for i in 0..(AUTOMATION_LOG_PER_AUTOMATION_CAP + 25) {
            runs.push_log(log_with(&format!("a{i}"), "auto"));
        }
        runs.prune_logs();
        let kept = runs.logs(Some("auto"), usize::MAX);
        assert_eq!(kept.len(), AUTOMATION_LOG_PER_AUTOMATION_CAP);
        assert_eq!(kept[0].run_id, format!("a{}", AUTOMATION_LOG_PER_AUTOMATION_CAP + 24));
        assert_eq!(
            kept.last().unwrap().run_id,
            format!("a{}", AUTOMATION_LOG_PER_AUTOMATION_CAP + 25 - AUTOMATION_LOG_PER_AUTOMATION_CAP),
            "the oldest surviving entry is the newest of the pruned tail"
        );

        let mut runs = AutomationRuns::default();
        for i in 0..(AUTOMATION_LOG_TOTAL_CAP + 40) {
            let automation = format!("auto-{}", i % 7);
            runs.push_log(log_with(&format!("r{i}"), &automation));
        }
        runs.prune_logs();
        assert!(
            runs.logs.len() <= AUTOMATION_LOG_TOTAL_CAP,
            "overall cap holds: {}",
            runs.logs.len()
        );
    }

    #[test]
    fn release_clears_slot_cancel_and_live_channel() {
        let mut runs = AutomationRuns::default();
        assert!(runs.insert_running("auto"));
        assert!(!runs.insert_running("auto"), "a second run cannot stack");
        runs.cancel.insert("run1".into(), Arc::new(Notify::new()));
        runs.live.insert("run1".into(), broadcast::channel(4).0);
        runs.release("auto", "run1");
        assert!(!runs.running.contains("auto"));
        assert!(!runs.cancel.contains_key("run1"));
        assert!(!runs.live.contains_key("run1"));
        assert!(runs.insert_running("auto"), "the slot is free again");
    }

    #[test]
    fn model_precedence_prefers_automation_then_project() {
        let mut cfg = HarnessConfig::default();
        cfg.projects = vec![crate::config::Project {
            id: "p1".into(),
            name: "P1".into(),
            path: std::path::PathBuf::from("."),
            icon: String::new(),
            created_at: String::new(),
            is_active: true,
            defaults: crate::config::ProjectDefaults {
                model: "project-model".into(),
                provider_id: "project-provider".into(),
                ..Default::default()
            },
            extra_paths: Vec::new(),
        }];

        let mut auto = sample_automation();
        auto.project_id = Some("p1".into());
        let (model, provider) = resolve_run_model_provider(&cfg, &auto);
        assert_eq!(model, "project-model");
        assert_eq!(provider, "project-provider");

        auto.model = "auto-model".into();
        auto.provider_id = "auto-provider".into();
        let (model, provider) = resolve_run_model_provider(&cfg, &auto);
        assert_eq!(model, "auto-model");
        assert_eq!(provider, "auto-provider");

        auto.project_id = None;
        let (model, provider) = resolve_run_model_provider(&cfg, &auto);
        assert_eq!(model, "auto-model");
        assert_eq!(provider, "auto-provider");
    }

    #[tokio::test]
    async fn log_lifecycle_start_then_success() {
        let (state, dir) = mock_state("lifecycle");
        let auto = sample_automation();
        let log = AutomationLog::start(
            &auto,
            "run-1".into(),
            crate::automation::AutomationRunTrigger::Manual,
            chrono::Local::now().to_rfc3339(),
        );
        state.record_automation_log_start(log).await;

        let logs = state.automation_logs(10).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].run_id, "run-1");
        assert!(logs[0].is_running());
        assert_eq!(logs[0].finished_at, None);

        state
            .update_automation_log(
                "run-1",
                AutomationLogUpdate {
                    status: Some(Status::Success),
                    output: Some("done".into()),
                    error: None,
                    tool_calls: Some(2),
                    tokens: Some(42),
                },
            )
            .await;

        let log = state.automation_log("run-1").await.expect("entry survives");
        assert_eq!(log.status, Status::Success);
        assert_eq!(log.output, "done");
        assert_eq!(log.tool_calls, 2);
        assert_eq!(log.tokens, 42);
        assert!(log.finished_at.is_some(), "terminal runs stamp finished_at");
        assert!(log.duration_ms.is_some(), "terminal runs stamp duration_ms");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn log_lifecycle_failure_keeps_error_and_truncates_output() {
        let (state, dir) = mock_state("failure");
        let auto = sample_automation();
        state
            .record_automation_log_start(AutomationLog::start(
                &auto,
                "run-2".into(),
                crate::automation::AutomationRunTrigger::Cron,
                chrono::Local::now().to_rfc3339(),
            ))
            .await;
        state
            .update_automation_log(
                "run-2",
                AutomationLogUpdate {
                    status: Some(Status::Failed),
                    output: Some("x".repeat(crate::automation::AUTOMATION_OUTPUT_CAP + 50)),
                    error: Some("model request failed".into()),
                    ..Default::default()
                },
            )
            .await;
        let log = state.automation_log("run-2").await.expect("entry survives");
        assert_eq!(log.status, Status::Failed);
        assert_eq!(log.error.as_deref(), Some("model request failed"));
        assert_eq!(
            log.output.chars().count(),
            crate::automation::AUTOMATION_OUTPUT_CAP + 1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_automation_in_mock_mode_logs_without_creating_a_session() {
        let (state, dir) = mock_state("mock-run");
        {
            let mut conf = state.config.write().await;
            conf.automations.push(sample_automation());
        }

        let run_id = state
            .run_automation(sample_automation())
            .await
            .expect("run starts");
        assert!(
            run_id.starts_with("auto-watcher-"),
            "the run id keeps the historical conversation-id prefix: {run_id}"
        );

        // A log entry exists immediately, in `running` state.
        let logs = state.automation_logs(10).await;
        assert_eq!(logs.len(), 1, "exactly one log entry, never a conversation");
        assert_eq!(logs[0].run_id, run_id);
        assert!(logs[0].is_running());
        assert_eq!(
            logs[0].trigger,
            crate::automation::AutomationRunTrigger::Cron
        );

        // The automation recorded the run right away.
        {
            let conf = state.config.read().await;
            let stored = conf.automations.iter().find(|a| a.id == "watcher").unwrap();
            assert_eq!(stored.run_count, 1);
            assert!(stored.last_run_at.is_some());
            assert_eq!(stored.last_status, "running");
        }
        assert!(state.automation_is_running("watcher").await);

        // Wait for the detached turn to finish.
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            match state.automation_log(&run_id).await {
                Some(log) if log.status.is_terminal() => break,
                _ => continue,
            }
        }

        let log = state
            .automation_log(&run_id)
            .await
            .expect("the run log survives completion");
        assert_eq!(
            log.status,
            Status::Success,
            "mock run must succeed: {:?}",
            log.error
        );
        assert!(!log.output.is_empty(), "the final answer is logged");
        assert!(log.finished_at.is_some() && log.duration_ms.is_some());
        assert!(log.tool_calls >= 1, "the mock loop calls sys_info");
        assert!(log.tokens > 0);

        // The decisive assertion: no conversation was created for the run. A swarm
        // board keyed by the *run id* is fine (the agent may use swarm tools);
        // what must never happen is a board that masquerades as a conversation.
        assert!(
            state.sessions.read().await.is_empty(),
            "automation runs must not add sessions"
        );
        {
            let sessions = state.sessions.read().await;
            let boards = state.swarm_boards.read().await;
            let orphans: Vec<String> = boards
                .keys()
                .filter(|k| sessions.contains_key(*k))
                .cloned()
                .collect();
            assert!(
                orphans.is_empty(),
                "no swarm board may be keyed by a conversation session: {orphans:?}"
            );
        }

        // Bookkeeping + slot release.
        {
            let conf = state.config.read().await;
            let stored = conf.automations.iter().find(|a| a.id == "watcher").unwrap();
            assert_eq!(stored.last_status, "completed");
        }
        assert!(!state.automation_is_running("watcher").await);

        // A second run is allowed once the first finished.
        let again = state.run_automation(sample_automation()).await;
        assert!(again.is_ok(), "the slot was released: {again:?}");
        assert_eq!(state.automation_logs(10).await.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn stop_automation_rejects_when_not_running() {
        let (state, dir) = mock_state("stop-idle");
        let err = state.stop_automation("watcher").await;
        assert!(err.is_err(), "nothing to stop: {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn overlapping_runs_are_rejected() {
        let (state, dir) = mock_state("overlap");
        {
            let mut conf = state.config.write().await;
            conf.automations.push(sample_automation());
        }
        let first = state.run_automation(sample_automation()).await;
        assert!(first.is_ok());
        let second = state.run_automation(sample_automation()).await;
        assert!(second.is_err(), "a second concurrent run must be rejected");
        // Let the first run finish so the spawned task does not outlive the test.
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if !state.automation_is_running("watcher").await {
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}