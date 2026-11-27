use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};

use crate::config::HarnessConfig;
use crate::db::{DbConversationRecord, HarnessDb};
use crate::engine::context::{ChatAttachment, ChatMessage, ConversationContext};
use crate::engine::events::HarnessEvent;
use crate::engine::swarm::SwarmBoard;
use crate::plugins::custom_tool::CustomToolSpec;
use crate::plugins::{is_builtin_tool_name, PluginRegistry};

static ID_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCheckpoint {
    pub turn_id: usize,
    pub prompt: String,
    pub history_index: usize,
    pub llm_msg_start_idx: usize,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedPrompt {
    pub id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ChatAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash_args: Option<String>,
    pub created_at: String,
}

/// Allocate a conversation id that cannot collide with one already in use.
///
/// `ses_{unix_ms}` is not unique: two browser tabs (or a double-clicked
/// "New Conversation") can mint the same millisecond, and the second writer
/// silently overwrote the first conversation. Retry with a suffix instead.
pub async fn unique_session_id(state: &AppState) -> String {
    let base = format!("ses_{}", chrono::Utc::now().timestamp_millis());
    let mut candidate = base.clone();
    let mut attempt = 0;
    loop {
        let recently_deleted = {
            let deleted = state.recently_deleted_sessions.lock().await;
            should_skip_resurrect(&deleted, &candidate)
        };
        let taken = state.sessions.read().await.contains_key(&candidate) || recently_deleted;
        if !taken {
            return candidate;
        }
        attempt += 1;
        if attempt > 64 {
            // Paranoia fallback: a monotonic counter can always make progress.
            return format!("{base}-{attempt}");
        }
        candidate = format!("{base}-{attempt}");
    }
}

pub fn is_known_builtin_slash_command(cmd: &str) -> bool {
    matches!(
        cmd.trim().trim_start_matches('/').to_ascii_lowercase().as_str(),
        "swarm" | "plan" | "compact" | "debug" | "diagnose" | "search" | "review" | "test" | "browser"
            | "condense"
    )
}

pub fn build_slash_command_directive(
    conf: &HarnessConfig,
    sandbox: &std::path::Path,
    command: &str,
    args: &str,
) -> (String, String, String) {
    let cmd = command.trim().trim_start_matches('/').to_ascii_lowercase();
    let trimmed_args = args.trim();
    let args_suffix = if trimmed_args.is_empty() {
        String::new()
    } else {
        format!("\nCommand Arguments: {}", trimmed_args)
    };

    match cmd.as_str() {
        "swarm" => (
            "swarm".to_string(),
            "Swarm Orchestration".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /swarm] Activate multi-agent Background Swarm Orchestration mode. \
                 1) Call `swarm_board{{action:\"set_title\", title:\"...\"}}` with a concise title for this swarm. \
                 2) Create `(job)` tags (`swarm_board{{action:\"create_job\"}}`) and `(member specialist)` tags (`swarm_board{{action:\"create_specialist\"}}`). \
                 3) Delegate parallel work using `invoke_subagent` (all workers execute asynchronously in the background by default with `wait: false`; pass `job_tag` and `specialist`), including an independent verifier. \
                 4) Monitor progress by sleeping zero-CPU with `swarm_board{{action:\"wait\", since:<seq>}}` — NEVER busy-poll with repeated `status`/`read`. Update finished jobs to `done` and write your final summary only after all background workers finish.{}",
                args_suffix
            ),
        ),
        "plan" => (
            "plan".to_string(),
            "Architecture Plan".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /plan] Formulate a clear, step-by-step architecture and \
                 implementation plan before making edits. Identify affected files, shared interfaces, \
                 edge cases, and verification steps. Optionally save the plan via `create_artifact`.{}",
                args_suffix
            ),
        ),
        "compact" => (
            "compact".to_string(),
            "Compact Context".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /compact] Keep responses ultra-concise, summarize current \
                 progress and essential state cleanly, and avoid redundant or verbose output.{}",
                args_suffix
            ),
        ),
        "condense" => (
            "condense".to_string(),
            "Condense Session into Skill".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /condense] Review this session's conversation and the work \
                 performed. Distill the repeatable workflow into a reusable skill and persist it by calling \
                 the `create_skill` tool with a clear `name`, a one-line `description`, and full Markdown \
                 `content` (goal, inputs, steps, constraints, preferred tools, verification). Write a general \
                 workflow, not a transcript of this session. Confirm the saved skill name.{}",
                args_suffix
            ),
        ),
        "debug" | "diagnose" => (
            "debug".to_string(),
            "Debug & Diagnostics".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /{}] Perform systematic root-cause debugging and diagnostics. \
                 Inspect logs, runtime errors, state transitions, and use `browser_ui` (`debug_audit`, \
                 `snapshot`, `storage`) or tests to reproduce and verify the fix.{}",
                cmd, args_suffix
            ),
        ),
        "search" => (
            "search".to_string(),
            "Grounded Search".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /search] Prioritize grounded search and codebase investigation \
                 (`mcp_google_ai_search_search`, `web_search`, `grep_search`, `read_file`) to gather \
                 verified facts and citations before answering.{}",
                args_suffix
            ),
        ),
        "review" => (
            "review".to_string(),
            "Code Review".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /review] Conduct a senior-level code review checking \
                 correctness, edge cases, security, concurrency, error handling, and test coverage. \
                 Report concrete findings with file paths and line numbers.{}",
                args_suffix
            ),
        ),
        "test" => (
            "test".to_string(),
            "Test & Verify".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /test] Run the relevant unit/integration tests and verification \
                 checks, diagnose any failures, and report exact observed pass/fail results.{}",
                args_suffix
            ),
        ),
        "browser" => (
            "browser".to_string(),
            "Browser UI Audit".to_string(),
            format!(
                "[SYSTEM COMMAND DIRECTIVE /browser] Run headless browser inspection and UI verification \
                 using `browser_ui` (`debug_audit`, `snapshot`, `screenshot`, `storage`, `interact` with \
                 `steps`) to verify layout, console errors, network requests, and interactive behavior.{}",
                args_suffix
            ),
        ),
        _ => {
            let skills = conf.discover_skills_in(sandbox);
            let matched = skills.into_iter().find(|s| {
                if !s.enabled {
                    return false;
                }
                let stem = std::path::Path::new(&s.path)
                    .file_stem()
                    .and_then(|st| st.to_str())
                    .unwrap_or("");
                s.name.eq_ignore_ascii_case(&cmd) || stem.eq_ignore_ascii_case(&cmd)
            });

            if let Some(skill) = matched {
                (
                    "skill".to_string(),
                    format!("Skill: {}", skill.name),
                    format!(
                        "[SYSTEM COMMAND DIRECTIVE /{}] Activate workspace skill '{}' ({}).\n\
                         Description: {}\n\nSkill Instructions:\n{}{}",
                        cmd, skill.name, skill.path, skill.description, skill.content, args_suffix
                    ),
                )
            } else {
                (
                    "command".to_string(),
                    format!("/{}", cmd),
                    format!(
                        "[SYSTEM COMMAND DIRECTIVE /{}] Execute the '/{}' command directive.{}",
                        cmd, cmd, args_suffix
                    ),
                )
            }
        }
    }
}

/// Automatically route a plain user message to matching skills.
///
/// Returns `(command, label, directive)` triples — the same shape as
/// [`build_slash_command_directive`] — so an auto-routed skill is injected as a
/// labelled `[SYSTEM COMMAND DIRECTIVE /<skill>]` system message, exactly like an
/// explicit `/skill` command but without the user typing one.
///
/// Safety contract:
/// - Returns nothing when `auto_skill_router` is disabled (behaviour identical to
///   a build without routing).
/// - Returns nothing for an empty prompt (no signal to route on).
/// - Never more than `skill_router_max_skills` entries, and only skills whose
///   router confidence clears `skill_router_min_confidence`.
/// - Callers must not use this when an explicit `/slash` command is present: an
///   explicit command always wins.
pub fn build_auto_skill_directives(
    conf: &HarnessConfig,
    sandbox: &std::path::Path,
    prompt: &str,
) -> Vec<(String, String, String)> {
    if !conf.auto_skill_router || prompt.trim().is_empty() {
        return Vec::new();
    }

    let skills = conf
        .discover_skills_in(sandbox)
        .into_iter()
        .filter(|s| s.enabled)
        .map(|s| {
            crate::skill_router::RoutableSkill::from_definition_merged(
                &s,
                crate::builtin_skills::BUILTIN_SKILLS,
            )
        })
        .collect::<Vec<_>>();
    if skills.is_empty() {
        return Vec::new();
    }

    crate::skill_router::route_skills(prompt, &skills, conf.skill_router_settings())
        .into_iter()
        .map(|matched| {
            let matched_terms = if matched.matched_terms.is_empty() {
                String::new()
            } else {
                format!(" Matched on: {}.", matched.matched_terms.join(", "))
            };
            let tool_note = if matched.tool_hints.is_empty() {
                String::new()
            } else {
                format!(" Tool hints: {}.", matched.tool_hints.join(", "))
            };
            let directive = format!(
                "[SYSTEM COMMAND DIRECTIVE /{}] Auto-routed skill '{}' ({}) with confidence {:.2}.\n\
                 The user did not invoke this skill explicitly; the router matched this message to it, so \
                 follow it as a system directive and let the user's request take precedence where they conflict.{}{}\n\
                 Description: {}\n\nSkill Instructions:\n{}",
                matched.name,
                matched.name,
                matched.path,
                matched.confidence,
                matched_terms,
                tool_note,
                matched.description,
                matched.content
            );
            (
                matched.name.clone(),
                format!("Skill: {}", matched.name),
                directive,
            )
        })
        .collect()
}

#[derive(Clone)]
pub struct ServerRunHandle {
    pub run_id: String,
    pub session_id: String,
    pub status: String,
    pub cancel_notify: Arc<tokio::sync::Notify>,
    pub steer_inbox: Arc<Mutex<VecDeque<String>>>,
    pub event_tx: broadcast::Sender<HarnessEvent>,
    pub history: Vec<HarnessEvent>,
    pub updated_messages: Option<Vec<ChatMessage>>,
    pub final_answer: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct SessionState {
    pub id: String,
    pub project_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub task: String,
    pub status: String,
    pub model: String,
    pub provider_id: String,
    pub reasoning_effort: String,
    pub server_id: String,
    pub orchestrator_mode: bool,
    pub event_tx: broadcast::Sender<HarnessEvent>,
    pub history: Vec<HarnessEvent>,
    pub cancel_notify: Arc<tokio::sync::Notify>,
    pub swarm: SwarmBoard,
    pub llm_messages: Vec<ChatMessage>,
    pub checkpoints: Vec<TurnCheckpoint>,
    pub next_turn_id: usize,
    pub queued_prompts: VecDeque<QueuedPrompt>,
    pub steer_inbox: Arc<Mutex<VecDeque<String>>>,
}

/// Minimal view of a loaded session used by the running-aware LRU eviction policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionCandidate {
    pub id: String,
    pub updated_at: String,
    pub status: String,
}

/// Statuses that mark a session as actively executing; these must never be evicted from memory.
pub fn is_session_status_active(status: &str) -> bool {
    matches!(status.trim(), "running" | "waiting")
}

/// Pick the session to evict when the in-memory session map is at capacity: the oldest
/// (`updated_at`) NON-active session. Active (`running`/`waiting`) sessions are never chosen
/// because a long turn may not bump `updated_at` and evicting it would silently lose a live
/// conversation. Returns `None` when there is no safe victim (e.g. every session is active).
pub fn pick_eviction_victim<'a, I>(candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a EvictionCandidate>,
{
    let mut victim: Option<&'a EvictionCandidate> = None;
    for cand in candidates {
        if is_session_status_active(&cand.status) {
            continue;
        }
        match victim {
            Some(cur) if cur.updated_at <= cand.updated_at => {}
            _ => victim = Some(cand),
        }
    }
    victim.map(|v| v.id.clone())
}

/// Decision for what to do with the config row stored in the DB at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredConfigMode {
    /// No usable stored config: seed the DB with the incoming (CLI/env) config.
    Seed,
    /// A stored config parsed: use it (still letting operator overrides win).
    UseSaved,
    /// The stored config row is corrupt/unreadable: fall back to the incoming config in
    /// memory ONLY — never overwrite the bad row so it can be recovered by an operator.
    RecoverInMemory,
}

/// Pure classification of `db.load_config()` for `AppState::with_db`.
pub fn classify_stored_config_load(loaded: &Result<Option<HarnessConfig>, String>) -> StoredConfigMode {
    match loaded {
        Ok(Some(_)) => StoredConfigMode::UseSaved,
        Ok(None) => StoredConfigMode::Seed,
        Err(_) => StoredConfigMode::RecoverInMemory,
    }
}

/// Bound the recently-deleted id set so it cannot grow unboundedly.
pub fn note_session_deleted(deleted: &mut HashSet<String>, session_id: &str, cap: usize) {
    if deleted.len() >= cap {
        deleted.clear();
    }
    deleted.insert(session_id.to_string());
}

/// Resurrection guard: a session deleted in-process must not be re-inserted from a stale
/// DB read that raced with the deletion.
pub fn should_skip_resurrect(deleted: &HashSet<String>, session_id: &str) -> bool {
    deleted.contains(session_id)
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<HarnessConfig>>,
    pub registry: Arc<RwLock<PluginRegistry>>,
    pub sessions: Arc<RwLock<HashMap<String, SessionState>>>,
    pub swarm_boards: Arc<RwLock<HashMap<String, SwarmBoard>>>,
    pub pending_permissions: Arc<RwLock<HashMap<String, oneshot::Sender<bool>>>>,
    /// (tool, session) for each pending permission request, so an "always
    /// allow" decision can name the tool it applies to.
    pub pending_permission_meta: Arc<RwLock<HashMap<String, (String, String)>>>,
    /// Tools a user approved for the rest of a session ("always allow").
    pub session_tool_allowlist: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    pub remote_perm_routes: Arc<RwLock<HashMap<String, String>>>,
    pub server_runs: Arc<RwLock<HashMap<String, ServerRunHandle>>>,
    pub active_remote_runs: Arc<RwLock<HashMap<String, (crate::config::ServerNodeConfig, String)>>>,
    pub db: Option<HarnessDb>,
    /// Session ids deleted in-process, used to guard against DB-read/write-lock races
    /// resurrecting a conversation that was deleted between the read and the insert.
    pub recently_deleted_sessions: Arc<Mutex<HashSet<String>>>,
    /// Automation ids currently executing (prevents overlapping scheduled runs).
    pub automation_runs: Arc<Mutex<crate::automation_runner::AutomationRuns>>,
    /// Live auto-update progress, mirrored to the UI (`GET /api/update/status`).
    pub update_status: Arc<RwLock<crate::updater::UpdateStatus>>,
}

impl AppState {
    pub fn new(config: HarnessConfig) -> Self {
        let registry = PluginRegistry::new(&config);
        Self {
            config: Arc::new(RwLock::new(config)),
            registry: Arc::new(RwLock::new(registry)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            swarm_boards: Arc::new(RwLock::new(HashMap::new())),
            pending_permissions: Arc::new(RwLock::new(HashMap::new())),
            pending_permission_meta: Arc::new(RwLock::new(HashMap::new())),
            session_tool_allowlist: Arc::new(RwLock::new(HashMap::new())),
            remote_perm_routes: Arc::new(RwLock::new(HashMap::new())),
            server_runs: Arc::new(RwLock::new(HashMap::new())),
            active_remote_runs: Arc::new(RwLock::new(HashMap::new())),
            db: None,
            recently_deleted_sessions: Arc::new(Mutex::new(HashSet::new())),
            automation_runs: Arc::new(Mutex::new(
                crate::automation_runner::AutomationRuns::default(),
            )),
            update_status: Arc::new(RwLock::new(crate::updater::UpdateStatus::default())),
        }
    }

    pub fn with_db(mut config: HarnessConfig, db: HarnessDb) -> Self {
        // Read the stored config row once and classify the outcome. A corrupt/unreadable row must
        // NOT be treated like "no config" (that would overwrite the user's saved config with
        // defaults — data loss); instead fall back to the incoming config in memory only.
        let loaded = db.load_config();
        match classify_stored_config_load(&loaded) {
            StoredConfigMode::Seed => {
                let _ = db.save_config(&config);
                for proj in &config.projects {
                    let _ = db.upsert_project(proj);
                }
                for srv in &config.servers {
                    let _ = db.upsert_server(srv);
                }
            }
            StoredConfigMode::UseSaved => {
                let mut saved_conf = loaded
                    .expect("UseSaved implies a parsed config")
                    .expect("UseSaved implies Some(config)");
                // Preserve runtime CLI/env host/port/db_path overrides if explicitly modified from default
                saved_conf.host = config.host.clone();
                saved_conf.port = config.port;
                saved_conf.db_path = config.db_path.clone();
                // CLI/env operator fields (sandbox_root, model, api_key, api_base, provider) win over
                // the persisted config when they were explicitly set at startup.
                saved_conf.apply_operator_overrides(&config, &HarnessConfig::default());
                // One-time cleanup of shipped default providers. Earlier builds seeded
// nvidia/deepseek/openrouter/groq/ollama/vllm; the harness now ships none, so
// drop anything that still looks exactly like a preset (same id, base and
// placeholder key). Gated on `onboarded` so a provider the user later adds is
// never removed.
if !saved_conf.onboarded {
    let before = saved_conf.custom_providers.len();
    saved_conf.custom_providers.retain(|p| !crate::config::is_shipped_default_provider(p));
    if saved_conf.custom_providers.len() != before {
        eprintln!(
            "[harness-db] removed {} leftover default provider(s)",
            before - saved_conf.custom_providers.len()
        );
    }
}
                // If the active provider no longer exists, clear the active selection so
                // the UI shows "no provider configured" instead of a dead one.
                if !saved_conf.provider.trim().is_empty()
                    && !saved_conf
                        .custom_providers
                        .iter()
                        .any(|p| p.id == saved_conf.provider)
                {
                    saved_conf.provider.clear();
                    saved_conf.api_base.clear();
                    saved_conf.api_key.clear();
                }
                if saved_conf.servers.is_empty() {
                    saved_conf.servers = crate::config::default_servers();
                }
                if saved_conf.active_server_id.trim().is_empty() {
                    saved_conf.active_server_id = crate::config::default_active_server_id();
                }
                config = saved_conf;
            }
            StoredConfigMode::RecoverInMemory => {
                let err = loaded
                    .as_ref()
                    .err()
                    .cloned()
                    .unwrap_or_else(|| "unknown error".to_string());
                eprintln!(
                    "[harness-db] stored config is unreadable ({}); keeping the bad row and falling back to CLI/env defaults in memory",
                    err
                );
                if config.servers.is_empty() {
                    config.servers = crate::config::default_servers();
                }
                if config.active_server_id.trim().is_empty() {
                    config.active_server_id = crate::config::default_active_server_id();
                }
                // Deliberately NO db.save_config(config) here: preserve the corrupt row for recovery.
            }
        }

        let registry = PluginRegistry::new(&config);
        let mut sessions_map = HashMap::new();
        let mut boards_map = HashMap::new();

        match db.list_conversations() {
            Err(e) => eprintln!("[harness-db] failed to list conversations at startup: {e}"),
            Ok(convs) => for c in convs.into_iter().take(100) {
                let (tx, _) = broadcast::channel(100);
                let swarm = SwarmBoard::new();
                let mut record = DbConversationRecord {
                    id: c.id.clone(),
                    project_id: c.project_id,
                    title: c.title,
                    status: c.status,
                    model: c.model,
                    provider_id: c.provider_id,
                    reasoning_effort: c.reasoning_effort,
                    server_id: if c.server_id.is_empty() { "local".to_string() } else { c.server_id },
                    orchestrator_mode: c.orchestrator_mode,
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                };
                if record.status == "running" {
                    // Recovered zombie rows: correct in memory AND write back so the DB no longer
                    // claims the session is running.
                    record.status = "stopped".to_string();
                    if let Err(e) = db.upsert_conversation(&record) {
                        eprintln!(
                            "[harness-db] failed to persist recovered 'stopped' status for session '{}': {}",
                            record.id, e
                        );
                    }
                }
                boards_map.insert(record.id.clone(), swarm.clone());
                sessions_map.insert(
                    record.id.clone(),
                    SessionState {
                        id: record.id,
                        project_id: record.project_id,
                        created_at: record.created_at,
                        updated_at: record.updated_at,
                        task: record.title,
                        status: record.status,
                        model: record.model,
                        provider_id: record.provider_id,
                        reasoning_effort: record.reasoning_effort,
                        server_id: record.server_id,
                        orchestrator_mode: record.orchestrator_mode,
                        event_tx: tx,
                        history: Vec::new(),
                        cancel_notify: Arc::new(tokio::sync::Notify::new()),
                        swarm,
                        llm_messages: Vec::new(),
                        checkpoints: Vec::new(),
                        next_turn_id: 0,
                        queued_prompts: VecDeque::new(),
                        steer_inbox: Arc::new(Mutex::new(VecDeque::new())),
                    },
                );
            }
        }

        Self {
            config: Arc::new(RwLock::new(config)),
            registry: Arc::new(RwLock::new(registry)),
            sessions: Arc::new(RwLock::new(sessions_map)),
            swarm_boards: Arc::new(RwLock::new(boards_map)),
            pending_permissions: Arc::new(RwLock::new(HashMap::new())),
            pending_permission_meta: Arc::new(RwLock::new(HashMap::new())),
            session_tool_allowlist: Arc::new(RwLock::new(HashMap::new())),
            remote_perm_routes: Arc::new(RwLock::new(HashMap::new())),
            server_runs: Arc::new(RwLock::new(HashMap::new())),
            active_remote_runs: Arc::new(RwLock::new(HashMap::new())),
            db: Some(db),
            recently_deleted_sessions: Arc::new(Mutex::new(HashSet::new())),
            automation_runs: Arc::new(Mutex::new(
                crate::automation_runner::AutomationRuns::default(),
            )),
            update_status: Arc::new(RwLock::new(crate::updater::UpdateStatus::default())),
        }
    }

    pub async fn persist_config(&self) {
        let Some(ref db) = self.db else { return };
        // Hold the config write lock across snapshot + write so two concurrent persists cannot
        // land out of order and silently revert each other's changes.
        let guard = self.config.write().await;
        if let Err(e) = db.save_config(&*guard) {
            eprintln!("[harness-db] failed to persist config: {}", e);
        }
    }

    /// Read a snapshot of the live update pipeline state.
    pub async fn update_status(&self) -> crate::updater::UpdateStatus {
        self.update_status.read().await.clone()
    }

    /// Mutate the live update pipeline state.
    pub async fn set_update_status<F: FnOnce(&mut crate::updater::UpdateStatus)>(
        &self,
        f: F,
    ) -> crate::updater::UpdateStatus {
        let mut guard = self.update_status.write().await;
        f(&mut guard);
        guard.clone()
    }

    /// True while the app has work that must not be interrupted by a restart:
    /// a running conversation, an automation run, a live subagent, or a remote
    /// server run. An update is only installed once this is `false`.
    pub async fn is_idle(&self) -> bool {
        let busy_sessions = {
            let lock = self.sessions.read().await;
            lock.values().any(|s| {
                s.status.trim() == "running"
                    || !s.queued_prompts.is_empty()
            })
        };
        if busy_sessions {
            return false;
        }
        if self.total_active_subagents().await > 0 {
            return false;
        }
        if !self.automation_runs.lock().await.running.is_empty() {
            return false;
        }
        if !self.server_runs.read().await.is_empty() {
            return false;
        }
        if !self.active_remote_runs.read().await.is_empty() {
            return false;
        }
        true
    }

    pub async fn persist_session_record(&self, session_id: &str) {
        let Some(ref db) = self.db else { return };
        // Hold the sessions write lock across snapshot + write, and write everything in ONE db
        // transaction, so concurrent persists cannot tear the record or revert each other.
        let guard = self.sessions.write().await;
        let Some(session) = guard.get(session_id) else { return };
        let rec = DbConversationRecord {
            id: session.id.clone(),
            project_id: session.project_id.clone(),
            title: session.task.clone(),
            status: session.status.clone(),
            model: session.model.clone(),
            provider_id: session.provider_id.clone(),
            reasoning_effort: session.reasoning_effort.clone(),
            server_id: session.server_id.clone(),
            orchestrator_mode: session.orchestrator_mode,
            created_at: session.created_at.clone(),
            updated_at: session.updated_at.clone(),
        };
        let (history, msgs, cps, turn_id) = (
            session.history.clone(),
            session.llm_messages.clone(),
            session.checkpoints.clone(),
            session.next_turn_id,
        );
        // Keep holding the write lock across the DB write so a concurrent persist cannot
        // interleave and write an older snapshot over this one.
        if let Err(e) = db.persist_session_snapshot(&rec, &history, &msgs, &cps, turn_id) {
            eprintln!(
                "[harness-db] failed to persist session snapshot for '{}': {}",
                session_id, e
            );
        }
    }

    pub async fn ensure_session_loaded(&self, session_id: &str) -> bool {
        let already_loaded = {
            let lock = self.sessions.read().await;
            match lock.get(session_id) {
                Some(s) => !s.history.is_empty() || !s.llm_messages.is_empty() || self.db.is_none(),
                None => false,
            }
        };
        if already_loaded {
            return true;
        }
        let Some(ref db) = self.db else {
            return self.sessions.read().await.contains_key(session_id);
        };

        let db_rec = db.get_conversation(session_id).ok().flatten();
        let events = db.load_events(session_id).unwrap_or_default();
        let msgs_tuple = match db.load_messages_and_checkpoints(session_id) {
            // `Ok(None)` = no messages persisted yet; start empty.
            Ok(t) => t.unwrap_or_default(),
            Err(e) => {
                // Propagate: never replace a good in-memory state with an empty failed load.
                // Returning false (rather than inserting empty state) prevents a later
                // persist from DELETE-ing the still-good rows in the DB.
                eprintln!(
                    "[harness-db] skipping message load for session '{}': {}",
                    session_id, e
                );
                return false;
            }
        };

        // Resurrection guard: re-check under the write lock so a delete that races
        // with this load cannot be undone by inserting the stale DB record.
        let mut lock = self.sessions.write().await;
        {
            let deleted = self.recently_deleted_sessions.lock().await;
            if should_skip_resurrect(&deleted, session_id) {
                eprintln!(
                    "[harness-db] not resurrecting deleted session '{}'",
                    session_id
                );
                return false;
            }
        }
        if let Some(session) = lock.get_mut(session_id) {
            if session.history.is_empty() && !events.is_empty() {
                session.history = events;
            }
            if session.llm_messages.is_empty() {
                let (msgs, cps, turn_id) = msgs_tuple;
                session.llm_messages = msgs;
                session.checkpoints = cps;
                session.next_turn_id = turn_id;
            }
            return true;
        }
        if let Some(c) = db_rec {
            let (tx, _) = broadcast::channel(100);
            let swarm = SwarmBoard::new();
            let (llm_messages, checkpoints, next_turn_id) = msgs_tuple;
            let mut recovered = c;
            if recovered.status == "running" {
                // Correct the zombie "running" row in memory AND in the DB.
                recovered.status = "stopped".to_string();
                if let Err(e) = db.upsert_conversation(&recovered) {
                    eprintln!(
                        "[harness-db] failed to persist recovered 'stopped' status for session '{}': {}",
                        recovered.id, e
                    );
                }
            }
            lock.insert(
                recovered.id.clone(),
                SessionState {
                    id: recovered.id,
                    project_id: recovered.project_id,
                    created_at: recovered.created_at,
                    updated_at: recovered.updated_at,
                    task: recovered.title,
                    status: recovered.status,
                    model: recovered.model,
                    provider_id: recovered.provider_id,
                    reasoning_effort: recovered.reasoning_effort,
                    server_id: if recovered.server_id.is_empty() { "local".to_string() } else { recovered.server_id },
                    orchestrator_mode: recovered.orchestrator_mode,
                    event_tx: tx,
                    history: events,
                    cancel_notify: Arc::new(tokio::sync::Notify::new()),
                    swarm,
                    llm_messages,
                    checkpoints,
                    next_turn_id,
                    queued_prompts: VecDeque::new(),
                    steer_inbox: Arc::new(Mutex::new(VecDeque::new())),
                },
            );
            return true;
        }
        false
    }

    pub async fn reload_plugins(&self) {
        let conf = self.config.read().await.clone();
        let mut new_reg = PluginRegistry::new(&conf);
        // Ensure active project's `.harness/tools/*.json` are also loaded if different from sandbox_root
        if let Some(active_proj) = conf.projects.iter().find(|p| p.id == conf.active_project_id) {
            if active_proj.path != conf.sandbox_root {
                new_reg.load_custom_tools_from(&active_proj.path);
            }
        }
        let mut reg = self.registry.write().await;
        *reg = new_reg;
    }

    /// Hot-register (or update) a modular custom tool in the live registry.
    pub async fn register_custom_tool(&self, spec: CustomToolSpec) -> Result<(), String> {
        let mut reg = self.registry.write().await;
        reg.register_custom_tool(spec)
    }

    /// Hot-unregister a custom tool from the live registry while protecting built-in tools.
    pub async fn unregister_custom_tool(&self, name: &str) -> Result<bool, String> {
        let mut reg = self.registry.write().await;
        if is_builtin_tool_name(name) || reg.is_builtin(name) {
            return Err(format!("Cannot delete built-in tool '{}'.", name));
        }
        Ok(reg.unregister(name))
    }

    /// Return a cheap clone of the current `PluginRegistry` snapshot for turn execution.
    pub async fn get_registry_snapshot(&self) -> PluginRegistry {
        self.registry.read().await.clone()
    }

    pub async fn create_session(
        &self,
        id: String,
        task: String,
        project_id: String,
    ) -> (broadcast::Sender<HarnessEvent>, Arc<tokio::sync::Notify>) {
        self.create_session_with_options(id, task, project_id, None, None, None, "running")
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_session_with_options(
        &self,
        id: String,
        task: String,
        project_id: String,
        server_id: Option<String>,
        orchestrator_mode: Option<bool>,
        model_override: Option<String>,
        initial_status: &str,
    ) -> (broadcast::Sender<HarnessEvent>, Arc<tokio::sync::Notify>) {
        let (tx, _) = broadcast::channel(100);
        let now = chrono::Local::now().to_rfc3339();
        let cancel_notify = Arc::new(tokio::sync::Notify::new());
        let swarm = {
            let mut boards = self.swarm_boards.write().await;
            boards.entry(id.clone()).or_insert_with(SwarmBoard::new).clone()
        };

        let (session_model, provider_id, default_effort, default_server_id, default_orch) = {
            let cfg = self.config.read().await;
            let proj_def = cfg
                .projects
                .iter()
                .find(|p| p.id == project_id)
                .map(|p| p.defaults.clone())
                .unwrap_or_default();
            let resolved_orch_flag = orchestrator_mode
                .or(proj_def.orchestrator_mode)
                .unwrap_or(cfg.orchestrator.enabled);
            let m = if cfg.mock_mode {
                "mock".to_string()
            } else if let Some(ref ov) = model_override {
                ov.clone()
            } else if !proj_def.model.trim().is_empty() {
                proj_def.model.clone()
            } else if resolved_orch_flag && !cfg.orchestrator.model.trim().is_empty() {
                cfg.orchestrator.model.clone()
            } else {
                cfg.model.clone()
            };
            let p_id = if !proj_def.provider_id.trim().is_empty() && model_override.is_none() {
                proj_def.provider_id.clone()
            } else if resolved_orch_flag && !cfg.orchestrator.provider_id.trim().is_empty() && model_override.is_none() {
                cfg.orchestrator.provider_id.clone()
            } else {
                cfg.provider.clone()
            };
            let eff = if !proj_def.reasoning_effort.trim().is_empty() {
                proj_def.reasoning_effort.clone()
            } else if resolved_orch_flag && !cfg.orchestrator.thinking_effort.trim().is_empty() {
                cfg.orchestrator.thinking_effort.clone()
            } else {
                cfg.reasoning_effort.clone()
            };
            let srv = if !proj_def.server_id.trim().is_empty() {
                proj_def.server_id
            } else if cfg.active_server_id.trim().is_empty() {
                "local".to_string()
            } else {
                cfg.active_server_id.clone()
            };
            (m, p_id, eff, srv, resolved_orch_flag)
        };

        let resolved_server_id = server_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(default_server_id);
        let resolved_orch = default_orch;

        let state = SessionState {
            id: id.clone(),
            project_id,
            created_at: now.clone(),
            updated_at: now,
            task,
            status: initial_status.to_string(),
            model: session_model,
            provider_id,
            reasoning_effort: default_effort,
            server_id: resolved_server_id,
            orchestrator_mode: resolved_orch,
            event_tx: tx.clone(),
            history: Vec::with_capacity(64),
            cancel_notify: cancel_notify.clone(),
            swarm,
            llm_messages: Vec::new(),
            checkpoints: Vec::new(),
            next_turn_id: 0,
            queued_prompts: VecDeque::new(),
            steer_inbox: Arc::new(Mutex::new(VecDeque::new())),
        };

        {
            let mut lock = self.sessions.write().await;
            if lock.len() >= 100 {
                let candidates: Vec<EvictionCandidate> = lock
                    .values()
                    .map(|s| EvictionCandidate {
                        id: s.id.clone(),
                        updated_at: s.updated_at.clone(),
                        status: s.status.clone(),
                    })
                    .collect();
                if let Some(victim) = pick_eviction_victim(&candidates) {
                    lock.remove(&victim);
                    let mut boards = self.swarm_boards.write().await;
                    boards.remove(&victim);
                }
            }
            lock.insert(id.clone(), state);
        }
        self.persist_session_record(&id).await;
        (tx, cancel_notify)
    }

    pub async fn update_session_status(&self, id: &str, status: &str) {
        let now = chrono::Local::now().to_rfc3339();
        {
            let mut lock = self.sessions.write().await;
            if let Some(session) = lock.get_mut(id) {
                session.status = status.to_string();
                session.updated_at = now;
            }
        }
        // Persist on every terminal transition. Previously `failed`/`panic` were
        // skipped, so a crashed turn left the DB stuck at `running` and the
        // user's prompt/tool results were lost on restart.
        if status != "running" {
            self.persist_session_record(id).await;
        }
    }

    pub async fn update_session_model(&self, id: &str, model: String) {
        let updated = {
            let mut lock = self.sessions.write().await;
            if let Some(session) = lock.get_mut(id) {
                session.model = model;
                session.updated_at = chrono::Local::now().to_rfc3339();
                true
            } else {
                false
            }
        };
        if updated {
            self.persist_session_record(id).await;
        }
    }

    pub async fn append_session_event(&self, id: &str, event: HarnessEvent) {
        let mut lock = self.sessions.write().await;
        if let Some(session) = lock.get_mut(id) {
            // Coalesce streaming text deltas to conserve memory and preserve full session history
            match (&event, session.history.last_mut()) {
                (
                    HarnessEvent::ThoughtDelta { delta: new_d },
                    Some(HarnessEvent::ThoughtDelta { delta: ref mut last_d }),
                ) => {
                    last_d.push_str(new_d);
                    return;
                }
                (
                    HarnessEvent::MessageDelta { delta: new_d },
                    Some(HarnessEvent::MessageDelta { delta: ref mut last_d }),
                ) => {
                    last_d.push_str(new_d);
                    return;
                }
                _ => {}
            }

            if session.history.len() < 1000 {
                session.history.push(event);
            }
        }
    }

    /// Append an event to the session history and broadcast it to live SSE listeners.
    pub async fn emit_session_event(&self, id: &str, event: HarnessEvent) {
        self.append_session_event(id, event.clone()).await;
        let lock = self.sessions.read().await;
        if let Some(session) = lock.get(id) {
            let _ = session.event_tx.send(event);
        }
    }

    /// Get a session's `SwarmBoard` if the session exists.
    pub async fn get_session_swarm(&self, session_id: &str) -> Option<SwarmBoard> {
        {
            let lock = self.sessions.read().await;
            if let Some(session) = lock.get(session_id) {
                return Some(session.swarm.clone());
            }
        }
        let boards = self.swarm_boards.read().await;
        boards.get(session_id).cloned()
    }

    /// Get or lazily create a `SwarmBoard` for `session_id`.
    pub async fn get_or_create_swarm(&self, session_id: &str) -> SwarmBoard {
        if let Some(board) = self.get_session_swarm(session_id).await {
            return board;
        }
        let mut boards = self.swarm_boards.write().await;
        boards
            .entry(session_id.to_string())
            .or_insert_with(SwarmBoard::new)
            .clone()
    }

    /// Cancel the session loop and all active subagents belonging to `session_id`.
    pub async fn stop_session(&self, session_id: &str) -> bool {
        let swarm_opt = {
            let mut lock = self.sessions.write().await;
            if let Some(session) = lock.get_mut(session_id) {
                // notify_one() STORES a permit when there is no parked waiter, so a Stop pressed
                // while the loop is between awaited `notified()` calls (e.g. during a 120s
                // permission wait) still cancels the next loop iteration. notify_waiters() would
                // only wake already-parked waiters and silently swallow the request.
                session.cancel_notify.notify_one();
                session.status = "stopped".to_string();
                Some(session.swarm.clone())
            } else {
                None
            }
        };

        if let Some(swarm) = swarm_opt {
            swarm.cancel_all_subagents().await;
            // Persist the status change so no zombie "running" row survives a stop.
            self.persist_session_record(session_id).await;
            true
        } else {
            false
        }
    }

    /// Cancel a single subagent in a session.
    pub async fn stop_subagent(&self, session_id: &str, subagent_id: &str) -> bool {
        if let Some(swarm) = self.get_session_swarm(session_id).await {
            swarm.cancel_subagent(subagent_id).await
        } else {
            false
        }
    }

    /// Count active (`running` / `waiting`) subagents across all sessions.
    pub async fn total_active_subagents(&self) -> usize {
        let boards: Vec<SwarmBoard> = {
            let lock = self.sessions.read().await;
            lock.values().map(|s| s.swarm.clone()).collect()
        };
        let mut total = 0;
        for board in boards {
            total += board.active_subagent_count().await;
        }
        total
    }

    /// Resolve the sandbox root `PathBuf` for a given session (falling back to global `sandbox_root`).
    pub async fn resolve_session_sandbox(&self, session_id: &str) -> PathBuf {
        let project_id = {
            let lock = self.sessions.read().await;
            lock.get(session_id).map(|s| s.project_id.clone())
        };
        let conf = self.config.read().await;
        if let Some(pid) = project_id {
            if let Some(proj) = conf.projects.iter().find(|p| p.id == pid) {
                return proj.path.clone();
            }
        }
        conf.sandbox_root.clone()
    }

    /// The project id a session belongs to, if known.
    pub async fn session_project_id(&self, session_id: &str) -> Option<String> {
        let lock = self.sessions.read().await;
        lock.get(session_id).map(|s| s.project_id.clone())
    }

    /// Every workspace folder a session may access (project folders, primary first).
    pub async fn resolve_session_roots(&self, session_id: &str) -> Vec<PathBuf> {
        let project_id = self.session_project_id(session_id).await;
        let conf = self.config.read().await;
        if let Some(pid) = project_id {
            if let Some(proj) = conf.projects.iter().find(|p| p.id == pid) {
                let roots = proj.all_paths();
                if !roots.is_empty() {
                    return roots;
                }
            }
        }
        vec![conf.sandbox_root.clone()]
    }

    pub async fn register_permission_waiter(&self, perm_id: String, tx: oneshot::Sender<bool>) {
        let mut lock = self.pending_permissions.write().await;
        lock.insert(perm_id, tx);
    }

    /// Register a permission request together with the (tool, session) it is
    /// asking about, so a later "always allow" decision can name the tool.
    pub async fn register_permission_waiter_with_meta(
        &self,
        perm_id: String,
        session_id: String,
        tool: String,
        tx: oneshot::Sender<bool>,
    ) {
        self.pending_permission_meta
            .write()
            .await
            .insert(perm_id.clone(), (tool, session_id));
        self.register_permission_waiter(perm_id, tx).await;
    }

    /// Drop a pending permission waiter without resolving it (e.g. on cancel or
    /// timeout) so stale entries cannot accumulate for the process lifetime.
    pub async fn cancel_permission_waiter(&self, perm_id: &str) {
        let mut lock = self.pending_permissions.write().await;
        lock.remove(perm_id);
        drop(lock);
        self.pending_permission_meta.write().await.remove(perm_id);
    }

    pub async fn resolve_permission(&self, perm_id: &str, approved: bool) -> bool {
        let sender = {
            let mut lock = self.pending_permissions.write().await;
            lock.remove(perm_id)
        };
        self.pending_permission_meta.write().await.remove(perm_id);
        match sender {
            Some(tx) => {
                let _ = tx.send(approved);
                true
            }
            None => false,
        }
    }

    /// The tool name a pending permission request belongs to, when still awake.
    ///
    /// Used by "Approve & always allow": the decision only names the tool once
    /// the request is already registered, so the tool is captured alongside it.
    pub async fn permission_tool_for(&self, perm_id: &str) -> Option<String> {
        let lock = self.pending_permission_meta.read().await;
        lock.get(perm_id).map(|(t, _)| t.clone())
    }

    /// The session a pending permission request belongs to (see
    /// [`Self::permission_tool_for`]).
    pub async fn permission_session_for(&self, perm_id: &str) -> Option<String> {
        let lock = self.pending_permission_meta.read().await;
        lock.get(perm_id).map(|(_, s)| s.clone())
    }

    /// Remember that `tool` was approved for the rest of `session_id`.
    pub async fn allow_tool_for_session(&self, session_id: &str, tool: &str) {
        self.session_tool_allowlist
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .insert(tool.to_string());
    }

    /// True when this tool was already approved for this session.
    pub async fn is_tool_allowed_for_session(&self, session_id: &str, tool: &str) -> bool {
        let lock = self.session_tool_allowlist.read().await;
        lock.get(session_id)
            .map(|tools| tools.contains(tool))
            .unwrap_or(false)
    }

    /// Forget every "always allow" decision for a session (e.g. on delete).
    pub async fn clear_tool_allowlist(&self, session_id: &str) {
        self.session_tool_allowlist.write().await.remove(session_id);
    }

    /// Replace a session's `cancel_notify` with a fresh `Notify` when resuming an idle/stopped session.
    /// Returns `(event_tx, fresh_cancel_notify, project_id)`.
    pub async fn reset_cancel_notify(
        &self,
        session_id: &str,
    ) -> Option<(broadcast::Sender<HarnessEvent>, Arc<tokio::sync::Notify>, String)> {
        let mut lock = self.sessions.write().await;
        let session = lock.get_mut(session_id)?;
        let fresh = Arc::new(tokio::sync::Notify::new());
        session.cancel_notify = fresh.clone();
        Some((session.event_tx.clone(), fresh, session.project_id.clone()))
    }

    /// Start a new turn on `session_id`: increments `next_turn_id`, records a `TurnCheckpoint`,
    /// appends + broadcasts `HarnessEvent::UserMessage`, sets `status = "running"`, and returns
    /// `(turn_id, prior_llm_messages, steer_inbox)`.
    #[allow(dead_code)]
    pub async fn begin_turn(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> Option<(usize, Vec<ChatMessage>, Arc<Mutex<VecDeque<String>>>)> {
        self.begin_turn_with_command(session_id, prompt, None, None)
            .await
    }

    /// Start a new turn on `session_id`, optionally applying a slash command as a
    /// `HarnessEvent::SystemCommand` directive + `ChatMessage::system(...)` without forcing a
    /// `UserMessage` bubble when `prompt` is empty.
    #[allow(dead_code)]
    pub async fn begin_turn_with_command(
        &self,
        session_id: &str,
        prompt: &str,
        slash_command: Option<&str>,
        slash_args: Option<&str>,
    ) -> Option<(usize, Vec<ChatMessage>, Arc<Mutex<VecDeque<String>>>)> {
        self.begin_turn_with_command_and_attachments(
            session_id,
            prompt,
            Vec::new(),
            slash_command,
            slash_args,
        )
        .await
    }

    /// Start a new turn on `session_id` with optional attachments and slash command directive.
    pub async fn begin_turn_with_command_and_attachments(
        &self,
        session_id: &str,
        prompt: &str,
        attachments: Vec<ChatAttachment>,
        slash_command: Option<&str>,
        slash_args: Option<&str>,
    ) -> Option<(usize, Vec<ChatMessage>, Arc<Mutex<VecDeque<String>>>)> {
        let clean_cmd = slash_command
            .map(|c| c.trim().trim_start_matches('/').to_ascii_lowercase())
            .filter(|c| !c.is_empty());
        let clean_args = slash_args.unwrap_or("").trim().to_string();

        let resolved_cmd = if let Some(cmd) = clean_cmd {
            let sandbox = self.resolve_session_sandbox(session_id).await;
            let conf = self.config.read().await.clone();
            let (icon, label, directive) =
                build_slash_command_directive(&conf, &sandbox, &cmd, &clean_args);
            Some((cmd, clean_args, icon, label, directive))
        } else {
            None
        };

        // No explicit slash command: let the router try to match a skill to the
        // user's own words. Never runs when a `/slash` command is present, never
        // runs for an empty prompt, and never runs when `auto_skill_router` is
        // disabled (the function returns an empty vec in that case).
        let auto_skill_directives: Vec<(String, String, String)> =
            if resolved_cmd.is_none() && !prompt.trim().is_empty() {
                let sandbox = self.resolve_session_sandbox(session_id).await;
                let conf = self.config.read().await.clone();
                build_auto_skill_directives(&conf, &sandbox, prompt)
            } else {
                Vec::new()
            };

        let mut lock = self.sessions.write().await;
        let session = lock.get_mut(session_id)?;

        session.next_turn_id += 1;
        let turn_id = session.next_turn_id;
        let timestamp = chrono::Local::now().to_rfc3339();
        let trimmed_prompt = prompt.trim();

        let checkpoint_prompt = if !trimmed_prompt.is_empty() {
            prompt.to_string()
        } else if !attachments.is_empty() {
            if attachments.len() == 1 {
                format!("[Attachment: {}]", attachments[0].name)
            } else {
                format!("[{} attachments]", attachments.len())
            }
        } else if let Some((ref cmd, ref args, _, _, _)) = resolved_cmd {
            if args.is_empty() {
                format!("/{}", cmd)
            } else {
                format!("/{} {}", cmd, args)
            }
        } else {
            prompt.to_string()
        };

        let checkpoint = TurnCheckpoint {
            turn_id,
            prompt: checkpoint_prompt,
            history_index: session.history.len(),
            llm_msg_start_idx: session.llm_messages.len(),
            timestamp: timestamp.clone(),
        };
        session.checkpoints.push(checkpoint);

        let has_command = resolved_cmd.is_some();
        if let Some((command, args, icon, label, directive)) = resolved_cmd {
            let sys_evt = HarnessEvent::SystemCommand {
                turn_id,
                command,
                args,
                icon,
                label,
                timestamp: timestamp.clone(),
            };
            session.history.push(sys_evt.clone());
            let _ = session.event_tx.send(sys_evt);
            session.llm_messages.push(ChatMessage::system(directive));
        }
        // Auto-routed skills (no explicit slash command) are injected here, one
        // labelled directive per matched skill, at most `skill_router_max_skills`.
        for (command, label, directive) in auto_skill_directives {
            let sys_evt = HarnessEvent::SystemCommand {
                turn_id,
                command,
                args: String::new(),
                icon: "skill".to_string(),
                label,
                timestamp: timestamp.clone(),
            };
            session.history.push(sys_evt.clone());
            let _ = session.event_tx.send(sys_evt);
            session.llm_messages.push(ChatMessage::system(directive));
        }

        if !has_command || !trimmed_prompt.is_empty() || !attachments.is_empty() {
            let user_evt = HarnessEvent::UserMessage {
                turn_id,
                content: prompt.to_string(),
                attachments,
                timestamp,
            };
            session.history.push(user_evt.clone());
            let _ = session.event_tx.send(user_evt);
        }

        session.status = "running".to_string();
        // Bump updated_at so an actively-executing session is never the stale-oldest victim of
        // the running-aware LRU eviction.
        session.updated_at = chrono::Local::now().to_rfc3339();

        Some((
            turn_id,
            session.llm_messages.clone(),
            session.steer_inbox.clone(),
        ))
    }

    /// Persist the updated LLM `ChatMessage` history after a turn completes or stops.
    pub async fn save_llm_messages(&self, session_id: &str, messages: Vec<ChatMessage>) {
        {
            let mut lock = self.sessions.write().await;
            if let Some(session) = lock.get_mut(session_id) {
                session.llm_messages = messages;
                session.updated_at = chrono::Local::now().to_rfc3339();
            }
        }
        self.persist_session_record(session_id).await;
    }

    /// Append a prompt to the session's FIFO queue, returning `(item, 1_based_position, full_queue)`.
    #[allow(dead_code)]
    pub async fn enqueue_prompt(
        &self,
        session_id: &str,
        content: String,
    ) -> Option<(QueuedPrompt, usize, Vec<QueuedPrompt>)> {
        self.enqueue_prompt_with_command(session_id, content, None, None)
            .await
    }

    /// Append a prompt (with optional slash command metadata) to the session's FIFO queue.
    #[allow(dead_code)]
    pub async fn enqueue_prompt_with_command(
        &self,
        session_id: &str,
        content: String,
        slash_command: Option<String>,
        slash_args: Option<String>,
    ) -> Option<(QueuedPrompt, usize, Vec<QueuedPrompt>)> {
        self.enqueue_prompt_with_command_and_attachments(
            session_id,
            content,
            Vec::new(),
            slash_command,
            slash_args,
        )
        .await
    }

    /// Append a prompt (with attachments and optional slash command metadata) to the session's FIFO queue.
    pub async fn enqueue_prompt_with_command_and_attachments(
        &self,
        session_id: &str,
        content: String,
        attachments: Vec<ChatAttachment>,
        slash_command: Option<String>,
        slash_args: Option<String>,
    ) -> Option<(QueuedPrompt, usize, Vec<QueuedPrompt>)> {
        let mut lock = self.sessions.write().await;
        let session = lock.get_mut(session_id)?;

        let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
        let item = QueuedPrompt {
            id: format!("q_{}_{}", chrono::Utc::now().timestamp_millis(), seq),
            content,
            attachments,
            slash_command,
            slash_args,
            created_at: chrono::Local::now().to_rfc3339(),
        };
        session.queued_prompts.push_back(item.clone());
        let position = session.queued_prompts.len();
        let queue: Vec<QueuedPrompt> = session.queued_prompts.iter().cloned().collect();
        Some((item, position, queue))
    }

    /// Pop the oldest queued prompt from the session queue, returning `(popped, remaining_queue)`.
    pub async fn pop_queued_prompt(
        &self,
        session_id: &str,
    ) -> Option<(QueuedPrompt, Vec<QueuedPrompt>)> {
        let mut lock = self.sessions.write().await;
        let session = lock.get_mut(session_id)?;
        let popped = session.queued_prompts.pop_front()?;
        let remaining: Vec<QueuedPrompt> = session.queued_prompts.iter().cloned().collect();
        Some((popped, remaining))
    }

    /// Remove a queued prompt by `queue_id`, returning the remaining queue if found.
    pub async fn remove_queued_prompt(
        &self,
        session_id: &str,
        queue_id: &str,
    ) -> Option<Vec<QueuedPrompt>> {
        let mut lock = self.sessions.write().await;
        let session = lock.get_mut(session_id)?;
        let pos = session.queued_prompts.iter().position(|q| q.id == queue_id)?;
        session.queued_prompts.remove(pos);
        Some(session.queued_prompts.iter().cloned().collect())
    }

    /// Return a snapshot of all currently queued prompts for `session_id`.
    pub async fn list_queued_prompts(&self, session_id: &str) -> Option<Vec<QueuedPrompt>> {
        let lock = self.sessions.read().await;
        let session = lock.get(session_id)?;
        Some(session.queued_prompts.iter().cloned().collect())
    }

    /// Push a steering instruction into the active turn's `steer_inbox`.
    pub async fn push_steer(&self, session_id: &str, content: String) -> bool {
        let inbox = {
            let lock = self.sessions.read().await;
            match lock.get(session_id) {
                Some(session) => session.steer_inbox.clone(),
                None => return false,
            }
        };
        inbox.lock().await.push_back(content);
        true
    }

    /// Remove a queued prompt and push its content into `steer_inbox` immediately.
    pub async fn promote_queued_to_steer(
        &self,
        session_id: &str,
        queue_id: &str,
    ) -> Option<(QueuedPrompt, Vec<QueuedPrompt>)> {
        let (item, remaining, inbox) = {
            let mut lock = self.sessions.write().await;
            let session = lock.get_mut(session_id)?;
            let pos = session.queued_prompts.iter().position(|q| q.id == queue_id)?;
            let item = session.queued_prompts.remove(pos)?;
            let remaining: Vec<QueuedPrompt> = session.queued_prompts.iter().cloned().collect();
            (item, remaining, session.steer_inbox.clone())
        };
        let steer_text = if let Some(ref cmd) = item.slash_command {
            let sandbox = self.resolve_session_sandbox(session_id).await;
            let conf = self.config.read().await.clone();
            let (_, _, directive) = build_slash_command_directive(
                &conf,
                &sandbox,
                cmd,
                item.slash_args.as_deref().unwrap_or(""),
            );
            if item.content.trim().is_empty() {
                directive
            } else {
                format!("{}\n\n{}", directive, item.content)
            }
        } else {
            // No slash command: auto-route the queued prompt exactly like a fresh
            // turn, so a promoted queued prompt gets the same skill guidance.
            let sandbox = self.resolve_session_sandbox(session_id).await;
            let conf = self.config.read().await.clone();
            let auto = build_auto_skill_directives(&conf, &sandbox, &item.content);
            if item.content.trim().is_empty() {
                auto.into_iter()
                    .map(|(_, _, directive)| directive)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else if auto.is_empty() {
                item.content.clone()
            } else {
                let prefix = auto
                    .into_iter()
                    .map(|(_, _, directive)| directive)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                format!("{}\n\n{}", prefix, item.content)
            }
        };
        inbox.lock().await.push_back(steer_text);
        Some((item, remaining))
    }

    /// Cancel any active run, roll back history/LLM messages/checkpoints to before `turn_id`,
    /// clear pending queue/steer state, set `status = "completed"`, and return `(restored_prompt, remaining_history)`.
    pub async fn undo_to_turn(
        &self,
        session_id: &str,
        turn_id: usize,
    ) -> Option<(String, Vec<HarnessEvent>)> {
        let cp = {
            let lock = self.sessions.read().await;
            let session = lock.get(session_id)?;
            session
                .checkpoints
                .iter()
                .find(|c| c.turn_id == turn_id)
                .cloned()?
        };

        self.stop_session(session_id).await;

        let (remaining_history, inbox) = {
            let mut lock = self.sessions.write().await;
            let session = lock.get_mut(session_id)?;
            let cp = session
                .checkpoints
                .iter()
                .find(|c| c.turn_id == turn_id)
                .cloned()?;

            session.history.truncate(cp.history_index);
            session.llm_messages.truncate(cp.llm_msg_start_idx);
            session.checkpoints.retain(|c| c.turn_id < turn_id);
            session.next_turn_id = session.checkpoints.last().map(|c| c.turn_id).unwrap_or(0);
            session.queued_prompts.clear();
            session.cancel_notify = Arc::new(tokio::sync::Notify::new());
            session.status = "completed".to_string();

            (session.history.clone(), session.steer_inbox.clone())
        };

        inbox.lock().await.clear();
        Some((cp.prompt, remaining_history))
    }

    /// Fork `source_id` into a new `ses_fork_<ts>` session (optionally truncated right after `up_to_turn_id`).
    /// Returns `(new_session_id, new_title, project_id)`.
    pub async fn fork_session(
        &self,
        source_id: &str,
        up_to_turn_id: Option<usize>,
    ) -> Option<(String, String, String)> {
        let (project_id, new_title, forked_model, forked_provider, forked_effort, forked_server, forked_orch, forked_history, forked_llm, forked_checkpoints) = {
            let lock = self.sessions.read().await;
            let source = lock.get(source_id)?;
            let new_title = format!("{} (Fork)", source.task);

            let (hist, llm, cps) = match up_to_turn_id {
                Some(tid) => {
                    let pos = source.checkpoints.iter().position(|c| c.turn_id == tid)?;
                    let (hist_end, llm_end) = if let Some(next_cp) = source.checkpoints.get(pos + 1) {
                        (
                            next_cp.history_index.min(source.history.len()),
                            next_cp.llm_msg_start_idx.min(source.llm_messages.len()),
                        )
                    } else {
                        (source.history.len(), source.llm_messages.len())
                    };
                    (
                        source.history[..hist_end].to_vec(),
                        source.llm_messages[..llm_end].to_vec(),
                        source.checkpoints[..=pos].to_vec(),
                    )
                }
                None => (
                    source.history.clone(),
                    source.llm_messages.clone(),
                    source.checkpoints.clone(),
                ),
            };

            (
                source.project_id.clone(),
                new_title,
                source.model.clone(),
                source.provider_id.clone(),
                source.reasoning_effort.clone(),
                source.server_id.clone(),
                source.orchestrator_mode,
                hist,
                llm,
                cps,
            )
        };

        let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
        let ts = chrono::Utc::now().timestamp_millis();
        let now = chrono::Local::now().to_rfc3339();
        let (tx, _) = broadcast::channel(100);
        let cancel_notify = Arc::new(tokio::sync::Notify::new());
        let next_turn_id = forked_checkpoints.last().map(|c| c.turn_id).unwrap_or(0);

        let new_id = {
            let mut lock = self.sessions.write().await;
            let mut new_id = format!("ses_fork_{}", ts);
            if lock.contains_key(&new_id) {
                new_id = format!("ses_fork_{}_{}", ts, seq);
            }

            if lock.len() >= 100 {
                // Evict before insert using the same running-aware LRU policy as session
                // creation so a live (running) session is never dropped from memory.
                let candidates: Vec<EvictionCandidate> = lock
                    .values()
                    .map(|s| EvictionCandidate {
                        id: s.id.clone(),
                        updated_at: s.updated_at.clone(),
                        status: s.status.clone(),
                    })
                    .collect();
                if let Some(victim) = pick_eviction_victim(&candidates) {
                    lock.remove(&victim);
                    let mut boards = self.swarm_boards.write().await;
                    boards.remove(&victim);
                }
            }

            let swarm = {
                let mut boards = self.swarm_boards.write().await;
                boards
                    .entry(new_id.clone())
                    .or_insert_with(SwarmBoard::new)
                    .clone()
            };

            let forked_state = SessionState {
                id: new_id.clone(),
                project_id: project_id.clone(),
                created_at: now.clone(),
                updated_at: now,
                task: new_title.clone(),
                status: "completed".to_string(),
                model: forked_model,
                provider_id: forked_provider,
                reasoning_effort: forked_effort,
                server_id: forked_server,
                orchestrator_mode: forked_orch,
                event_tx: tx,
                history: forked_history,
                cancel_notify,
                swarm,
                llm_messages: forked_llm,
                checkpoints: forked_checkpoints,
                next_turn_id,
                queued_prompts: VecDeque::new(),
                steer_inbox: Arc::new(Mutex::new(VecDeque::new())),
            };

            lock.insert(new_id.clone(), forked_state);
            new_id
        };
        self.persist_session_record(&new_id).await;
        Some((new_id, new_title, project_id))
    }

    /// Rename a session's display task/title.
    pub async fn rename_session(&self, session_id: &str, title: String) -> bool {
        let updated = {
            let mut lock = self.sessions.write().await;
            if let Some(session) = lock.get_mut(session_id) {
                session.task = title;
                session.updated_at = chrono::Local::now().to_rfc3339();
                true
            } else {
                false
            }
        };
        if updated {
            self.persist_session_record(session_id).await;
        }
        updated
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_session_meta(
        &self,
        session_id: &str,
        title: Option<String>,
        project_id: Option<String>,
        server_id: Option<String>,
        orchestrator_mode: Option<bool>,
        model: Option<String>,
        provider_id: Option<String>,
        reasoning_effort: Option<String>,
    ) -> bool {
        let updated = {
            let mut lock = self.sessions.write().await;
            if let Some(session) = lock.get_mut(session_id) {
                if let Some(t) = title {
                    session.task = t;
                }
                if let Some(pid) = project_id {
                    session.project_id = pid;
                }
                if let Some(sid) = server_id {
                    session.server_id = sid;
                }
                if let Some(orch) = orchestrator_mode {
                    session.orchestrator_mode = orch;
                }
                if let Some(m) = model {
                    if !m.trim().is_empty() {
                        session.model = m.trim().to_string();
                    }
                }
                if let Some(p) = provider_id {
                    // An empty value means "unchanged": the UI sends "" for fields it
                    // does not intend to modify, and blanking it here would discard a
                    // provider that was resolved from the project defaults.
                    if !p.trim().is_empty() {
                        session.provider_id = p.trim().to_string();
                    }
                }
                if let Some(eff) = reasoning_effort {
                    if !eff.trim().is_empty() {
                        session.reasoning_effort = eff.trim().to_string();
                    }
                }
                session.updated_at = chrono::Local::now().to_rfc3339();
                true
            } else {
                false
            }
        };
        if updated {
            self.persist_session_record(session_id).await;
        }
        updated
    }

    pub async fn delete_session(&self, session_id: &str) -> bool {
        self.stop_session(session_id).await;
        // Delete from the DB first; only drop in-memory state (and mark the id as
        // deleted) once the row is actually gone. Otherwise a failed DB delete
        // would leave a row that is invisible in the UI but still persisted.
        let removed_db = if let Some(ref db) = self.db {
            match db.delete_conversation(session_id) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "[harness-db] failed to delete conversation '{}': {}",
                        session_id, e
                    );
                    // Abort: keep the session visible so the user can retry.
                    return false;
                }
            }
        } else {
            false
        };
        let removed_mem = {
            let mut lock = self.sessions.write().await;
            lock.remove(session_id).is_some()
        };
        {
            let mut boards = self.swarm_boards.write().await;
            boards.remove(session_id);
        }
        // A deleted session must not carry its "always allow" trust forward.
        self.clear_tool_allowlist(session_id).await;
        // Record the deletion so ensure_session_loaded cannot resurrect this id from a DB read
        // that raced with the delete.
        {
            let mut deleted = self.recently_deleted_sessions.lock().await;
            note_session_deleted(&mut deleted, session_id, 1024);
        }
        removed_mem || removed_db
    }

    /// Compact a session's LLM message context and archive older UI history events while
    /// updating `TurnCheckpoint` indices so Undo/Fork remain valid.
    /// Returns `(events_compacted, before_tokens, after_tokens, messages_folded)`.
    pub async fn compact_session_memory(
        &self,
        session_id: &str,
    ) -> Option<(usize, usize, usize, usize)> {
        let conf = self.config.read().await.clone();
        let mut mem_cfg = conf.memory.clone();
        mem_cfg.max_context_tokens = crate::models::capabilities::effective_max_context_tokens(
            &conf.model,
            mem_cfg.max_context_tokens,
            mem_cfg.auto_detect_context,
            None,
        );
        let mut lock = self.sessions.write().await;
        let session = lock.get_mut(session_id)?;

        let mut ctx = ConversationContext::from_messages(
            session.llm_messages.clone(),
            conf.max_tool_output_bytes,
        )
        .with_memory_config(mem_cfg.clone());

        let before_tokens = ctx.estimate_tokens();
        let report = ctx.force_compact();
        let after_tokens = ctx.estimate_tokens();
        let messages_folded = report.as_ref().map(|r| r.messages_folded).unwrap_or(0);

        if report.is_some() {
            let orig_llm_len = session.llm_messages.len();
            session.llm_messages = ctx.messages;
            let new_llm_len = session.llm_messages.len();
            let llm_delta = orig_llm_len.saturating_sub(new_llm_len);

            let has_sys = session
                .llm_messages
                .first()
                .is_some_and(|m| m.role == "system");
            let has_wm = session
                .llm_messages
                .get(usize::from(has_sys))
                .is_some_and(|m| {
                    m.content.as_deref().is_some_and(|c| {
                        c.starts_with("[COMPRESSED WORKING MEMORY]")
                            || c.starts_with("[COMPACTED WORKING MEMORY]")
                    })
                });
            let min_llm_idx = (usize::from(has_sys) + usize::from(has_wm)).min(new_llm_len);

            for cp in &mut session.checkpoints {
                if cp.llm_msg_start_idx == 0 {
                    continue;
                }
                if cp.llm_msg_start_idx >= llm_delta + min_llm_idx {
                    cp.llm_msg_start_idx -= llm_delta;
                } else {
                    cp.llm_msg_start_idx = min_llm_idx;
                }
                cp.llm_msg_start_idx = cp.llm_msg_start_idx.min(new_llm_len);
            }
        }

        let orig_hist_len = session.history.len();
        let keep_recent = conf.memory.preserve_recent_messages.max(6);
        let mut events_compacted = 0usize;

        let compact_evt = HarnessEvent::MemoryCompacted {
            before_tokens,
            after_tokens,
            messages_folded,
            reason: "manual_compact".to_string(),
        };

        if orig_hist_len > keep_recent + 2 {
            let end_idx = orig_hist_len.saturating_sub(keep_recent);
            session.history.drain(1..end_idx);
            session.history.insert(1, compact_evt.clone());

            let new_hist_len = session.history.len();
            events_compacted = orig_hist_len.saturating_sub(new_hist_len);
            let min_hist_idx = 2usize.min(new_hist_len);

            for cp in &mut session.checkpoints {
                if cp.history_index == 0 {
                    continue;
                }
                if cp.history_index >= end_idx {
                    cp.history_index = cp.history_index.saturating_sub(events_compacted);
                } else {
                    cp.history_index = min_hist_idx;
                }
                cp.history_index = cp.history_index.min(new_hist_len);
            }

            let _ = session.event_tx.send(compact_evt);
        } else if report.is_some() {
            if session.history.len() < 1000 {
                session.history.push(compact_evt.clone());
            }
            let _ = session.event_tx.send(compact_evt);
        }

        Some((events_compacted, before_tokens, after_tokens, messages_folded))
    }

    /// Reassign all sessions owned by a deleted project to a fallback project so conversations are preserved.
    pub async fn reassign_project_sessions(
        &self,
        from_project_id: &str,
        to_project_id: &str,
    ) -> usize {
        if from_project_id == to_project_id {
            return 0;
        }
        // Persist the move first so a DB failure leaves memory and storage
        // consistent (otherwise sessions reappear under a deleted project on restart).
        let mut reassigned = 0usize;
        if let Some(ref db) = self.db {
            match db.reassign_project_conversations(from_project_id, to_project_id) {
                Ok(db_count) => reassigned = db_count,
                Err(e) => {
                    eprintln!(
                        "[harness-db] failed to reassign conversations {} -> {}: {}",
                        from_project_id, to_project_id, e
                    );
                }
            }
        }
        let mem_count = {
            let mut lock = self.sessions.write().await;
            let mut count = 0usize;
            for session in lock.values_mut() {
                if session.project_id == from_project_id {
                    session.project_id = to_project_id.to_string();
                    count += 1;
                }
            }
            count
        };
        reassigned.max(mem_count)
    }
}

#[cfg(test)]
mod core_state_tests {
    use super::*;
    use std::time::Duration;

    // ---------- Auto-update: idle detection gates a self-restart ----------

    #[tokio::test]
    async fn is_idle_is_true_on_a_fresh_state() {
        let state = AppState::new(HarnessConfig::default());
        assert!(state.is_idle().await, "nothing is running on a fresh state");
        assert_eq!(state.update_status().await.phase, crate::updater::UpdatePhase::Idle);
    }

    #[tokio::test]
    async fn is_idle_is_false_while_a_conversation_runs() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let state = AppState::with_db(cfg, db);
        let id = "ses-update-busy".to_string();
        state
            .create_session_with_options(id.clone(), "busy".into(), "project".into(), None, None, None, "running")
            .await;
        assert!(!state.is_idle().await, "a running conversation blocks an update");

        state.update_session_status(&id, "completed").await;
        assert!(state.is_idle().await, "an idle conversation no longer blocks");
    }

    #[tokio::test]
    async fn update_status_is_observable_and_mutable() {
        let state = AppState::new(HarnessConfig::default());
        state
            .set_update_status(|s| {
                s.current_version = "1.0.0".into();
                s.progress_phase(crate::updater::UpdatePhase::Downloading, "Downloading v1.0.1...");
                s.downloaded_bytes = 512;
                s.total_bytes = 1024;
                s.percent = Some(50);
            })
            .await;
        let snap = state.update_status().await;
        assert_eq!(snap.current_version, "1.0.0");
        assert_eq!(snap.phase, crate::updater::UpdatePhase::Downloading);
        assert_eq!(snap.percent, Some(50));
        assert!(snap.updated_at > 0);
    }

    #[tokio::test]
    async fn queued_prompts_also_block_an_update() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let state = AppState::with_db(cfg, db);
        let id = "ses-update-queued".to_string();
        state
            .create_session_with_options(id.clone(), "queued".into(), "project".into(), None, None, None, "completed")
            .await;
        assert!(state.is_idle().await);
        {
            let mut sessions = state.sessions.write().await;
            sessions.get_mut(&id).unwrap().queued_prompts = VecDeque::from(vec![QueuedPrompt {
                id: "q1".into(),
                content: "keep working".into(),
                attachments: Vec::new(),
                slash_command: None,
                slash_args: None,
                created_at: "now".into(),
            }]);
        }
        assert!(!state.is_idle().await, "a queued prompt means a turn is still owed");
    }

    // ---------- Finding 1: stop must cancel a loop that is not currently parked ----------

    #[tokio::test]
    async fn notify_one_without_a_waiter_still_releases_a_later_notified() {
        let notify = Arc::new(tokio::sync::Notify::new());
        // Stop pressed while nobody is parked on `.notified()` (e.g. inter-turn bookkeeping).
        notify.notify_one();
        let n = notify.clone();
        let res = tokio::time::timeout(Duration::from_millis(750), async move {
            n.notified().await;
        })
        .await;
        assert!(
            res.is_ok(),
            "notify_one must store a permit so a later notified() resolves immediately"
        );
    }

    #[tokio::test]
    async fn notify_waiters_without_a_waiter_is_swallowed_documents_old_bug() {
        let notify = Arc::new(tokio::sync::Notify::new());
        // The pre-fix behavior: nothing wakes because there is no parked waiter.
        notify.notify_waiters();
        let n = notify.clone();
        let res = tokio::time::timeout(Duration::from_millis(150), async move {
            n.notified().await;
        })
        .await;
        assert!(
            res.is_err(),
            "notify_waiters() stores no permit — the old stop_session bug"
        );
    }

    #[tokio::test]
    async fn stop_session_persists_stopped_status_and_cancels_a_later_loop() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let db_check = db.clone();
        let state = AppState::with_db(cfg, db);

        let (_tx, cancel_notify) = state
            .create_session_with_options(
                "ses_stop_notify".to_string(),
                "Long running task".to_string(),
                "harness".to_string(),
                Some("local".to_string()),
                Some(false),
                None,
                "running",
            )
            .await;

        // The loop has not parked on `.notified()` yet when Stop arrives.
        assert!(state.stop_session("ses_stop_notify").await);

        // (a) the DB row must not stay a zombie "running" row
        let row = db_check
            .get_conversation("ses_stop_notify")
            .expect("db read")
            .expect("conversation row");
        assert_eq!(row.status, "stopped");

        // (b) the stored permit must still fire for the loop that parks afterwards
        let n = cancel_notify.clone();
        let res = tokio::time::timeout(Duration::from_millis(750), async move {
            n.notified().await;
        })
        .await;
        assert!(
            res.is_ok(),
            "a Stop issued while the loop is unparked must still cancel it"
        );

        // (c) resuming via reset_cancel_notify drops any stale permit so it cannot fire later
        let (_bcast, fresh, _proj) = state
            .reset_cancel_notify("ses_stop_notify")
            .await
            .expect("session exists");
        let res2 = tokio::time::timeout(Duration::from_millis(150), async move {
            fresh.notified().await;
        })
        .await;
        assert!(
            res2.is_err(),
            "reset_cancel_notify must install a fresh Notify with no stale permit"
        );
    }

    // ---------- Finding 2: stored-config load classification ----------

    #[test]
    fn classify_stored_config_load_handles_three_branches() {
        assert_eq!(
            classify_stored_config_load(&Ok(None)),
            StoredConfigMode::Seed
        );
        assert_eq!(
            classify_stored_config_load(&Ok(Some(HarnessConfig::default()))),
            StoredConfigMode::UseSaved
        );
        assert_eq!(
            classify_stored_config_load(&Err::<Option<HarnessConfig>, String>(
                "Failed to deserialize DB config: bad json".to_string()
            )),
            StoredConfigMode::RecoverInMemory
        );
    }

    #[tokio::test]
    async fn with_db_does_not_overwrite_a_corrupt_config_row() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        db.save_config(&HarnessConfig::default()).unwrap();
        // Corrupt the stored row to mimic a failed deserialize on upgrade.
        db.inject_raw_config_for_test("{not json at all");

        let mut incoming = HarnessConfig::default();
        incoming.model = "cli-model".to_string();
        incoming.sandbox_root = PathBuf::from("/cli/workspace");
        let state = AppState::with_db(incoming.clone(), db.clone());

        let conf = state.config.read().await;
        assert_eq!(conf.model, "cli-model");
        assert_eq!(conf.sandbox_root, PathBuf::from("/cli/workspace"));
        drop(conf);

        // The corrupt row must still be there (not silently overwritten with defaults).
        assert!(
            db.load_config().is_err(),
            "corrupt config row must be preserved for recovery"
        );
    }

    #[tokio::test]
    async fn with_db_operator_overrides_win_over_saved_config() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut saved = HarnessConfig::default();
        saved.model = "db-model".to_string();
        saved.sandbox_root = PathBuf::from("/db/workspace");
        saved.temperature = 0.5;
        db.save_config(&saved).unwrap();

        let mut incoming = HarnessConfig::default();
        incoming.model = "cli-model".to_string();
        incoming.sandbox_root = PathBuf::from("/cli/workspace");

        let state = AppState::with_db(incoming.clone(), db.clone());
        let conf = state.config.read().await;
        assert_eq!(conf.model, "cli-model");
        assert_eq!(conf.sandbox_root, PathBuf::from("/cli/workspace"));
        // Non-operator fields keep the persisted value (host/port stay runtime-binding).
        assert!((conf.temperature - 0.5).abs() < f32::EPSILON);
        assert_eq!(conf.port, incoming.port);
    }

    #[tokio::test]
    async fn with_db_recovers_running_rows_and_writes_back_stopped() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let zombie = DbConversationRecord {
            id: "ses_zombie".to_string(),
            project_id: "harness".to_string(),
            title: "Crashed turn".to_string(),
            status: "running".to_string(),
            model: "mock".to_string(),
            provider_id: "nvidia".to_string(),
            reasoning_effort: "high".to_string(),
            server_id: "local".to_string(),
            orchestrator_mode: false,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        };
        db.upsert_conversation(&zombie).unwrap();

        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let db_check = db.clone();
        let state = AppState::with_db(cfg, db);

        let loaded = state
            .sessions
            .read()
            .await
            .get("ses_zombie")
            .map(|s| s.status.clone())
            .expect("recovered session");
        assert_eq!(loaded, "stopped");

        let row = db_check
            .get_conversation("ses_zombie")
            .unwrap()
            .expect("conversation row");
        assert_eq!(row.status, "stopped", "zombie row must be written back");
    }

    // ---------- Findings 4/5: running-aware LRU eviction ----------

    fn candidate(id: &str, updated_at: &str, status: &str) -> EvictionCandidate {
        EvictionCandidate {
            id: id.to_string(),
            updated_at: updated_at.to_string(),
            status: status.to_string(),
        }
    }

    #[test]
    fn pick_eviction_victim_skips_running_sessions() {
        let sessions = vec![
            candidate("ses_old_completed", "2026-01-01T00:00:00+00:00", "completed"),
            candidate("ses_running", "2026-01-02T00:00:00+00:00", "running"),
            candidate("ses_idle", "2026-01-03T00:00:00+00:00", "idle"),
        ];
        assert_eq!(
            pick_eviction_victim(&sessions),
            Some("ses_old_completed".to_string())
        );
    }

    #[test]
    fn pick_eviction_victim_returns_none_when_everything_is_active() {
        let sessions = vec![
            candidate("ses_running_a", "2026-01-01T00:00:00+00:00", "running"),
            candidate("ses_waiting_b", "2026-01-02T00:00:00+00:00", "waiting"),
        ];
        assert_eq!(pick_eviction_victim(&sessions), None);
    }

    #[test]
    fn pick_eviction_victim_empty_map_yields_none() {
        let sessions: Vec<EvictionCandidate> = Vec::new();
        assert_eq!(pick_eviction_victim(&sessions), None);
    }

    #[test]
    fn pick_eviction_victim_picks_oldest_when_all_inactive() {
        let sessions = vec![
            candidate("b", "2026-02-01T00:00:00+00:00", "error"),
            candidate("a", "2026-01-01T00:00:00+00:00", "completed"),
            candidate("c", "2026-03-01T00:00:00+00:00", "stopped"),
        ];
        assert_eq!(pick_eviction_victim(&sessions), Some("a".to_string()));
    }

    // ---------- Finding 6: resurrection guard ----------

    #[test]
    fn resurrection_guard_skips_recently_deleted_ids() {
        let mut deleted: HashSet<String> = HashSet::new();
        assert!(!should_skip_resurrect(&deleted, "ses_gone"));
        note_session_deleted(&mut deleted, "ses_gone", 16);
        assert!(should_skip_resurrect(&deleted, "ses_gone"));
        assert!(!should_skip_resurrect(&deleted, "ses_other"));
        // Bounded growth: inserting past the cap clears older entries.
        for i in 0..16 {
            note_session_deleted(&mut deleted, &format!("ses_{}", i), 16);
        }
        assert!(!should_skip_resurrect(&deleted, "ses_other"));
        assert!(should_skip_resurrect(&deleted, "ses_15"));
    }

    #[tokio::test]
    async fn deleted_session_is_not_resurrected_by_ensure_session_loaded() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let state = AppState::with_db(cfg, db);

        state
            .create_session_with_options(
                "ses_delete_me".to_string(),
                "Temporary".to_string(),
                "harness".to_string(),
                None,
                None,
                None,
                "completed",
            )
            .await;
        assert!(state.delete_session("ses_delete_me").await);
        assert!(
            !state.ensure_session_loaded("ses_delete_me").await,
            "a just-deleted session must not be resurrected from its stale DB row"
        );
    }

    #[tokio::test]
    async fn ensure_session_loaded_does_not_resurrect_a_concurrently_deleted_session() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let state = AppState::with_db(cfg, db);

        // Persist a conversation row (the DB read part of the race).
        state
            .create_session_with_options(
                "ses_race".to_string(),
                "Racing".to_string(),
                "harness".to_string(),
                None,
                None,
                None,
                "completed",
            )
            .await;

        // Simulate the delete landing between the DB read and the map insert: the row is still
        // present in the DB but the session was removed from memory and marked deleted.
        {
            let mut lock = state.sessions.write().await;
            lock.remove("ses_race");
            let mut deleted = state.recently_deleted_sessions.lock().await;
            note_session_deleted(&mut deleted, "ses_race", 1024);
        }

        assert!(
            !state.ensure_session_loaded("ses_race").await,
            "must not insert a session that was deleted between DB read and insert"
        );
    }

    // ---------- Finding 8: snapshot persists through one transaction ----------

    #[tokio::test]
    async fn persist_session_record_writes_snapshot_via_single_transaction() {
        let db = HarnessDb::open_in_memory().expect("open in-memory db");
        let db_check = db.clone();
        let mut cfg = HarnessConfig::default();
        cfg.mock_mode = true;
        let state = AppState::with_db(cfg, db);

        let (_tx, _cancel) = state
            .create_session_with_options(
                "ses_snapshot".to_string(),
                "Snapshot test".to_string(),
                "harness".to_string(),
                None,
                None,
                None,
                "completed",
            )
            .await;

        state
            .append_session_event(
                "ses_snapshot",
                HarnessEvent::IterationStep { step: 1, max_steps: 3 },
            )
            .await;
        state
            .save_llm_messages("ses_snapshot", vec![ChatMessage::user("hello")])
            .await;
        state.update_session_model("ses_snapshot", "updated-model".to_string()).await;

        let cached = db_check
            .get_conversation("ses_snapshot")
            .unwrap()
            .expect("conversation row");
        assert_eq!(cached.model, "updated-model", "model must be persisted");
        let events = db_check.load_events("ses_snapshot").unwrap();
        assert_eq!(events.len(), 1);
        let (msgs, _, _) = db_check
            .load_messages_and_checkpoints("ses_snapshot")
            .unwrap()
            .expect("messages row");
        assert_eq!(msgs.len(), 1);
    }
}


#[cfg(test)]
mod condense_skill_tests {
    #[test]
    fn condense_is_builtin_and_injects_create_skill() {
        assert!(crate::state::is_known_builtin_slash_command("condense"));
        assert!(crate::state::is_known_builtin_slash_command("/CONDENSE"));
        let conf = crate::config::HarnessConfig::default();
        let (kind, _label, directive) = crate::state::build_slash_command_directive(
            &conf,
            std::path::Path::new("."),
            "condense",
            "",
        );
        assert_eq!(kind, "condense");
        assert!(directive.contains("create_skill"), "directive must instruct calling create_skill");
        assert!(directive.contains("SYSTEM COMMAND DIRECTIVE /condense"));
    }
}

#[cfg(test)]
mod auto_skill_router_tests {
    use super::*;

    fn default_conf() -> HarnessConfig {
        HarnessConfig::default()
    }

    /// Text of every system-role message injected into a session.
    fn system_texts(state: &crate::state::AppState, session_id: &str) -> Vec<String> {
        state
            .sessions
            .try_read()
            .ok()
            .and_then(|lock| lock.get(session_id).map(|s| s.llm_messages.clone()))
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role == "system")
            .filter_map(|m| m.content)
            .collect()
    }

    #[test]
    fn routing_message_injects_the_matching_skill_directive() {
        let conf = default_conf();
        let directives = crate::state::build_auto_skill_directives(
            &conf,
            std::path::Path::new("."),
            "Refactor the router and run the tests to verify it",
        );
        assert_eq!(directives.len(), 1, "one clear intent -> exactly one skill");
        let (command, label, directive) = &directives[0];
        assert_eq!(command, "code");
        assert_eq!(label, "Skill: code");
        assert!(directive.starts_with("[SYSTEM COMMAND DIRECTIVE /code] Auto-routed skill"));
        assert!(directive.contains("builtin://code"));
        assert!(directive.contains("Skill Instructions:"));
        assert!(directive.contains("Read before writing."));
    }

    #[test]
    fn low_signal_messages_inject_nothing() {
        let conf = default_conf();
        for msg in ["ok", "thanks", "hi", ""] {
            assert!(
                crate::state::build_auto_skill_directives(&conf, std::path::Path::new("."), msg)
                    .is_empty(),
                "'{msg}' must not inject a skill"
            );
        }
    }

    #[test]
    fn at_mention_hint_still_routes() {
        let conf = default_conf();
        let directives = crate::state::build_auto_skill_directives(
            &conf,
            std::path::Path::new("."),
            "Fix the failing build and refactor the parser, check it with @bash",
        );
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].0, "code");
        assert!(directives[0].2.contains("Tool hints: bash."));
    }

    #[test]
    fn multi_intent_stays_within_the_cap() {
        let msg = "Research and cite the latest sources, then implement and test the fix";
        let mut conf = default_conf();
        conf.skill_router_max_skills = 1;
        let capped = crate::state::build_auto_skill_directives(&conf, std::path::Path::new("."), msg);
        assert_eq!(capped.len(), 1, "the cap must limit injection to one skill");

        conf.skill_router_max_skills = 2;
        let both = crate::state::build_auto_skill_directives(&conf, std::path::Path::new("."), msg);
        assert_eq!(both.len(), 2, "near-tie multi-intent may inject two skills");
        let names: Vec<&str> = both.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(names, vec!["web-research", "code"]);
    }

    #[test]
    fn threshold_gate_blocks_weak_matches() {
        let mut conf = default_conf();
        conf.skill_router_min_confidence = 0.99;
        let directives = crate::state::build_auto_skill_directives(
            &conf,
            std::path::Path::new("."),
            "Refactor the skill router and run the tests to verify it",
        );
        assert!(directives.is_empty(), "a high threshold must block injection");
    }

    #[test]
    fn disabled_flag_produces_no_injection() {
        let mut conf = default_conf();
        conf.auto_skill_router = false;
        assert!(crate::state::build_auto_skill_directives(
            &conf,
            std::path::Path::new("."),
            "Refactor the skill router and run the tests to verify it"
        )
        .is_empty());
        // A clearly routing message must stay silent while disabled.
        assert!(crate::state::build_auto_skill_directives(
            &conf,
            std::path::Path::new("."),
            "Research and cite sources for async runtimes"
        )
        .is_empty());
    }

    #[test]
    fn explicit_slash_command_wins_over_auto_routing() {
        let conf = default_conf();
        // The slash path is separate; the auto path must not fire for the same
        // message that an explicit command covers.
        let (kind, _label, directive) = build_slash_command_directive(
            &conf,
            std::path::Path::new("."),
            "review",
            "src/state.rs",
        );
        assert_eq!(kind, "review");
        assert!(directive.starts_with("[SYSTEM COMMAND DIRECTIVE /review]"));
    }

    #[tokio::test]
    async fn begin_turn_injects_the_routed_skill_for_a_plain_message() {
        let mut cfg = default_conf();
        cfg.mock_mode = true;
        let state = crate::state::AppState::new(cfg);
        let session_id = "ses_auto_router".to_string();
        state
            .create_session_with_options(
                session_id.clone(),
                "Auto router".to_string(),
                "harness".to_string(),
                None,
                None,
                None,
                "idle",
            )
            .await;

        let (turn_id, messages, _steer) = state
            .begin_turn(&session_id, "Refactor the router and run the tests to verify it")
            .await
            .expect("turn starts");
        assert_eq!(turn_id, 1);
        let injected = messages
            .iter()
            .find(|m| {
                m.content
                    .as_deref()
                    .map(|c| c.contains("Auto-routed skill"))
                    .unwrap_or(false)
            })
            .expect("routed skill directive injected");
        assert!(injected
            .content
            .as_deref()
            .unwrap_or("")
            .contains("SYSTEM COMMAND DIRECTIVE /code"));
        assert!(injected
            .content
            .as_deref()
            .unwrap_or("")
            .contains("Skill Instructions:"));

        // The same directive is broadcast to the UI as a SystemCommand event.
        let history = {
            let lock = state.sessions.read().await;
            lock.get(&session_id)
                .map(|s| s.history.clone())
                .unwrap_or_default()
        };
        let commands: Vec<String> = history
            .iter()
            .filter_map(|e| match e {
                crate::engine::events::HarnessEvent::SystemCommand { command, .. } => {
                    Some(command.clone())
                }
                _ => None,
            })
            .collect();
        assert!(commands.contains(&"code".to_string()));
    }

    #[tokio::test]
    async fn begin_turn_with_slash_command_skips_auto_routing() {
        let mut cfg = default_conf();
        cfg.mock_mode = true;
        let state = crate::state::AppState::new(cfg);
        let session_id = "ses_auto_router_slash".to_string();
        state
            .create_session_with_options(
                session_id.clone(),
                "Auto router".to_string(),
                "harness".to_string(),
                None,
                None,
                None,
                "idle",
            )
            .await;

        let (_turn_id, messages, _steer) = state
            .begin_turn_with_command(&session_id, "", Some("review"), Some("src/state.rs"))
            .await
            .expect("turn starts");
        assert!(
            !system_texts(&state, &session_id)
                .iter()
                .any(|c| c.contains("Auto-routed skill")),
            "an explicit slash command must never trigger auto routing"
        );
        assert!(
            messages
                .iter()
                .any(|m| m
                    .content
                    .as_deref()
                    .map(|c| c.contains("SYSTEM COMMAND DIRECTIVE /review"))
                    .unwrap_or(false))
        );
    }

    #[tokio::test]
    async fn begin_turn_with_low_signal_message_injects_nothing() {
        let mut cfg = default_conf();
        cfg.mock_mode = true;
        let state = crate::state::AppState::new(cfg);
        let session_id = "ses_auto_router_low".to_string();
        state
            .create_session_with_options(
                session_id.clone(),
                "Auto router".to_string(),
                "harness".to_string(),
                None,
                None,
                None,
                "idle",
            )
            .await;

        let (_turn_id, messages, _steer) = state
            .begin_turn(&session_id, "thanks!")
            .await
            .expect("turn starts");
        assert!(messages.iter().all(|m| m
            .content
            .as_deref()
            .map(|c| !c.contains("Auto-routed skill"))
            .unwrap_or(true)));
    }
}
