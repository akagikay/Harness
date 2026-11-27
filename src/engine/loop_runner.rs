use futures_util::future::join_all;
use futures_util::FutureExt;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};
use tokio::sync::{mpsc, oneshot};

use crate::config::{HarnessConfig, PermissionLevel};
use crate::engine::context::{ChatAttachment, ChatMessage, ConversationContext};
use crate::engine::events::HarnessEvent;
use crate::engine::recovery;
use crate::models::ModelAdapter;
use crate::plugins::contract::{ToolContract, ToolPermission};
use crate::plugins::{PluginRegistry, SessionToolContext};
use crate::state::AppState;

static PERM_SEQ: AtomicU64 = AtomicU64::new(1);

/// Detect the harness's own system prompt so a resumed session can replace it
/// with a freshly built one instead of stacking a duplicate.
///
/// Recognises the current prefix and the historical ones, so sessions persisted
/// by older builds still refresh cleanly.
fn is_harness_system_prompt(content: &str) -> bool {
    const PREFIXES: [&str; 3] = [
        "You are Harness, an autonomous agent runtime",
        "You are an autonomous agent runtime",
        "You are an autonomous agent runtime powered by",
    ];
    PREFIXES.iter().any(|p| content.starts_with(p))
}

/// Best-effort human message from a caught panic payload.
fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn is_transient_model_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if lower.contains("captcha")
        || lower.contains("verification gate")
        || lower.contains("/sorry/")
        || lower.contains("cannot be bypassed")
    {
        return false;
    }
    [
        "stream read error",
        "error decoding response body",
        "timed out",
        "connection reset",
        "status 429",
        "status 502",
        "status 503",
        "status 504",
        "http request error",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn is_concurrent_safe_tool_call(
    tool_name: &str,
    args: &serde_json::Value,
    contract: Option<&ToolContract>,
) -> bool {
    if matches!(
        tool_name,
        "read_file" | "list_dir" | "grep_search" | "sys_info" | "web_search" | "web_fetch"
    ) {
        return true;
    }
    if tool_name == "swarm_board" {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        return matches!(
            action,
            "read" | "status" | "list_agents" | "list_jobs" | "list_specialists"
        );
    }
    contract.is_some_and(|c| c.permission_level == ToolPermission::ReadOnly)
}

fn is_mutating_tool_call(tool_name: &str, args: &serde_json::Value) -> bool {
    !is_concurrent_safe_tool_call(tool_name, args, None) && tool_name != "swarm_board"
}

fn tool_timeout_secs(
    config: &HarnessConfig,
    tool_name: &str,
    contract: Option<&ToolContract>,
) -> u64 {
    let contract_timeout = contract.map(|c| c.timeout_secs).unwrap_or(0);
    match tool_name {
        "invoke_subagent" => config.tool_timeout_secs.max(1800).max(contract_timeout),
        "swarm_board" => config.tool_timeout_secs.max(150).max(contract_timeout),
        "browser_ui" => config.tool_timeout_secs.max(90).max(contract_timeout),
        _ => config.tool_timeout_secs.max(60).max(contract_timeout),
    }
}

enum ToolRetryRunResult {
    Completed {
        duration_ms: u64,
        result: Result<String, (String, recovery::ToolErrorDiagnosis)>,
    },
    Cancelled,
}

#[allow(clippy::too_many_arguments)]
async fn execute_tool_with_retries(
    exec_registry: &PluginRegistry,
    tool_contract: Option<&ToolContract>,
    tool_name: &str,
    call_id: &str,
    parsed_args: &serde_json::Value,
    sandbox: &std::path::Path,
    step_tool_ctx: &SessionToolContext,
    cancel_notify: &tokio::sync::Notify,
    event_tx: &mpsc::Sender<HarnessEvent>,
    timeout_secs: u64,
    max_tool_attempts: usize,
) -> ToolRetryRunResult {
    let tool_start = Instant::now();
    let mut final_exec: Result<String, (String, recovery::ToolErrorDiagnosis)> = Err((
        "Tool execution did not run".to_string(),
        recovery::diagnose_tool_error(tool_name, "Tool execution did not run", tool_contract),
    ));

    for attempt in 1..=max_tool_attempts {
        let exec_result = tokio::select! {
            _ = cancel_notify.notified() => {
                return ToolRetryRunResult::Cancelled;
            }
            res = tokio::time::timeout(
                Duration::from_secs(timeout_secs),
                // Isolate a panicking tool so it fails the call, not the process.
                std::panic::AssertUnwindSafe(
                    exec_registry.execute_with_context(
                        tool_name,
                        parsed_args.clone(),
                        sandbox,
                        Some(step_tool_ctx),
                    ),
                )
                .catch_unwind(),
            ) => {
                match res {
                    Ok(Ok(inner)) => inner,
                    Ok(Err(panic)) => Err(format!(
                        "Tool '{}' panicked: {}",
                        tool_name,
                        panic_payload_message(&panic)
                    )),
                    Err(_) => Err(format!("Tool '{}' execution timed out after {} seconds", tool_name, timeout_secs)),
                }
            }
        };

        match exec_result {
            Ok(out) => {
                final_exec = Ok(out);
                break;
            }
            Err(err) => {
                let diagnosis = recovery::diagnose_tool_error(tool_name, &err, tool_contract);
                if diagnosis.is_retryable && attempt < max_tool_attempts {
                    let _ = event_tx
                        .send(HarnessEvent::ToolRetryScheduled {
                            call_id: call_id.to_string(),
                            tool: tool_name.to_string(),
                            attempt: attempt + 1,
                            max_attempts: max_tool_attempts,
                            reason: diagnosis.summary.clone(),
                        })
                        .await;

                    tokio::select! {
                        _ = cancel_notify.notified() => {
                            return ToolRetryRunResult::Cancelled;
                        }
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    }
                    continue;
                }
                final_exec = Err((err, diagnosis));
                break;
            }
        }
    }

    ToolRetryRunResult::Completed {
        duration_ms: tool_start.elapsed().as_millis() as u64,
        result: final_exec,
    }
}

pub struct AgentLoopRunner {
    pub session_id: String,
    pub task: String,
    pub config: HarnessConfig,
    pub registry: PluginRegistry,
    pub model: Arc<dyn ModelAdapter>,
    pub sandbox: PathBuf,
    pub state: Option<AppState>,
    pub cancel_notify: Arc<tokio::sync::Notify>,
    pub agent_id: String,
    pub role: Option<String>,
    pub parent_id: Option<String>,
    pub depth: usize,
    pub max_depth: usize,
    pub prior_messages: Vec<ChatMessage>,
    pub task_attachments: Vec<ChatAttachment>,
    pub steer_inbox: Option<Arc<tokio::sync::Mutex<VecDeque<String>>>>,
}

impl AgentLoopRunner {
    pub fn new(
        session_id: String,
        task: String,
        config: HarnessConfig,
        registry: PluginRegistry,
        model: Arc<dyn ModelAdapter>,
        state: Option<AppState>,
    ) -> Self {
        let sandbox = config.sandbox_root.clone();
        Self {
            session_id,
            task,
            config,
            registry,
            model,
            sandbox,
            state,
            cancel_notify: Arc::new(tokio::sync::Notify::new()),
            agent_id: "coordinator".to_string(),
            role: None,
            parent_id: None,
            depth: 0,
            max_depth: 2,
            prior_messages: Vec::new(),
            task_attachments: Vec::new(),
            steer_inbox: None,
        }
    }

    pub fn with_cancel_notify(mut self, notify: Arc<tokio::sync::Notify>) -> Self {
        self.cancel_notify = notify;
        self
    }

    pub fn with_sandbox(mut self, sandbox: PathBuf) -> Self {
        self.sandbox = sandbox;
        self
    }

    pub fn with_subagent(
        mut self,
        agent_id: String,
        role: String,
        parent_id: Option<String>,
        depth: usize,
    ) -> Self {
        self.agent_id = agent_id;
        self.role = Some(role);
        self.parent_id = parent_id;
        self.depth = depth;
        self
    }

    pub fn with_prior_messages(mut self, msgs: Vec<ChatMessage>) -> Self {
        self.prior_messages = msgs;
        self
    }

    pub fn with_steer_inbox(
        mut self,
        inbox: Arc<tokio::sync::Mutex<VecDeque<String>>>,
    ) -> Self {
        self.steer_inbox = Some(inbox);
        self
    }

    pub fn with_task_attachments(mut self, attachments: Vec<ChatAttachment>) -> Self {
        self.task_attachments = attachments;
        self
    }

    fn check_permission_policy(&self, tool_name: &str, registry: &PluginRegistry) -> PermissionLevel {
        if let Some(contract) = registry.get_contract(tool_name) {
            self.config
                .permissions
                .policy_for_contract(tool_name, &contract.permission_level)
        } else {
            self.config.permissions.for_tool(tool_name)
        }
    }

    fn tool_permission_description(&self, tool: &str, args: &serde_json::Value) -> String {
        match tool {
            "bash" => {
                let cmd = args.get("command").and_then(|c| c.as_str()).unwrap_or("");
                format!("Execute shell command: `{}`", cmd)
            }
            "write_file" => {
                let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
                format!("Write/overwrite file: `{}`", path)
            }
            "edit_file" => {
                let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
                format!("Edit/patch file: `{}`", path)
            }
            "web_search" => {
                let q = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
                format!("Web search: '{}'", q)
            }
            "web_fetch" => {
                let u = args.get("url").and_then(|u| u.as_str()).unwrap_or("");
                format!("Web fetch: '{}'", u)
            }
            "browser_ui" => {
                let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("inspect");
                let u = args.get("url").and_then(|u| u.as_str()).unwrap_or("");
                format!("Browser UI {} {}", action, u).trim().to_string()
            }
            "invoke_subagent" => {
                let role = args.get("role").and_then(|r| r.as_str()).unwrap_or("batch");
                format!("Spawn subagent(s): {}", role)
            }
            "swarm_board" => {
                let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("read");
                format!("Swarm board: {}", action)
            }
            "create_artifact" => {
                let title = args.get("title").and_then(|t| t.as_str()).unwrap_or("artifact");
                format!("Create artifact: '{}'", title)
            }
            "manage_tools" => {
                let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("list");
                let name = args.get("name").and_then(|n| n.as_str()).unwrap_or("");
                format!("Manage tools ({} {})", action, name).trim().to_string()
            }
            _ => format!("Execute tool `{}`", tool),
        }
    }

    fn build_system_prompt(&self) -> String {
        let skills_snippet = self.config.format_skills_prompt_snippet(&self.sandbox);
        let subagent_note = if self.depth > 0 {
            format!(
                "\nSubagent Identity: id='{}', role='{}', parent='{}', depth={}/{}.\n\
                 Focus strictly on your assigned slice and return a concise, verified result.",
                self.agent_id,
                self.role.as_deref().unwrap_or("worker"),
                self.parent_id.as_deref().unwrap_or("coordinator"),
                self.depth,
                self.max_depth
            )
        } else {
            String::new()
        };

        let delegation_guidance = if self.depth < self.max_depth {
            "- Parallel Subagents (`invoke_subagent`): For complex multi-file edits, independent research, or parallel verification, spawn specialized subagents (single `role`+`prompt` or parallel `tasks` array; optional `files` + `include_board: true`; `read_only: true` for safe inspection; runs asynchronously in background by default with `wait: false`). When coordinating a swarm, call `swarm_board{action:\"set_title\"}`, create `(job)` tags (`create_job`) and `(member specialist)` tags (`create_specialist`), assign work via `invoke_subagent`, then sleep zero-CPU using `swarm_board{action:\"wait\", since:<seq>}` (or `wait_all: true` to wait until the whole wave finishes) and mark jobs done via `complete_job`.\n"
        } else {
            ""
        };

        let has_google_ai_search = self
            .registry
            .get_contract("mcp_google_ai_search_search")
            .is_some();
        let extra_tools_list = if has_google_ai_search {
            ", `mcp_google_ai_search_search`"
        } else {
            ""
        };
        let google_search_guidance = if has_google_ai_search {
            "- Google AI Grounded Search (`mcp_google_ai_search_search`): Use for AI-synthesized grounded search answers with citations (`mode: \"ai_overview\"` or `\"web\"`), especially when `web_search` or `web_fetch` is blocked or lacks synthesis.\n"
        } else {
            ""
        };

        let host_env_line = if cfg!(windows) {
            "Host Environment: Windows (PowerShell 5.1 — use ';' instead of '&&', use PowerShell cmdlets / dir instead of Unix 'ls -la')"
        } else {
            "Host Environment: Unix (sh)"
        };

        let knowledge_section = if self.config.knowledge_only {
            "\nExecution Mode: Standalone Knowledge-Work Conversation (No Project Workspace attached).\n\
             Local filesystem read, write, edit, and shell tools are DISABLED. Do not attempt to inspect or modify local files or run shell commands. Focus on knowledge synthesis, reasoning, planning, and web/search tools if needed."
        } else {
            ""
        };

        let orchestrator_section = if self.depth == 0 && self.config.orchestrator.enabled {
            let prompt_body = if self.config.orchestrator.system_prompt.trim().is_empty() {
                crate::config::default_orchestrator_system_prompt()
            } else {
                self.config.orchestrator.system_prompt.trim().to_string()
            };
            let remote_servers: Vec<String> = self
                .config
                .servers
                .iter()
                .filter(|s| s.enabled && s.url.starts_with("http"))
                .map(|s| format!("{} ({} -> {})", s.id, s.name, s.url))
                .collect();
            let servers_note = if remote_servers.is_empty() {
                String::new()
            } else {
                format!(
                    "\nAvailable Execution Servers for `invoke_subagent` (`server_id` parameter): {}.",
                    remote_servers.join(", ")
                )
            };
            let profiles_note = if self.config.orchestrator.subagent_mode.eq_ignore_ascii_case("multi")
                && !self.config.orchestrator.subagent_profiles.is_empty()
            {
                let lines: Vec<String> = self
                    .config
                    .orchestrator
                    .subagent_profiles
                    .iter()
                    .map(|p| {
                        let m = if p.model.trim().is_empty() { "default" } else { p.model.trim() };
                        let srv = if p.server_id.trim().is_empty() { "local" } else { p.server_id.trim() };
                        format!(
                            "  - `{}` ({}): {} [model: {}, effort: {}, server: {}, read_only: {}]",
                            p.id, p.name, p.description, m, p.thinking_effort, srv, p.read_only
                        )
                    })
                    .collect();
                format!(
                    "\nConfigured Subagent Types (pass `specialist: \"<id>\"` or matching `role` to `invoke_subagent`):\n{}",
                    lines.join("\n")
                )
            } else {
                String::new()
            };
            format!("\n\n{}\n{}{}", prompt_body, servers_note, profiles_note)
        } else {
            String::new()
        };

        format!(
            "You are Harness, an autonomous agent runtime\n\
             Workspace Root: {}\n\
             {}{}{}\n\
             Principles & Capabilities:\n\
             1. 'Everything is a Plugin': Utilize `bash`, `read_file`, `write_file`, `edit_file`, `list_dir`, `grep_search`, `sys_info`, `web_search`, `web_fetch`, `browser_ui`, `swarm_board`, `create_artifact`, `manage_tools`, `invoke_subagent`{}.\n\
             2. Modular Tool Contracts & Custom Tools (`manage_tools`):\n\
             - Every tool enforces a typed Tool Contract (required/optional fields, constraints, permission tier, expected output, failure modes).\n\
             - Use `manage_tools` (`list`, `inspect`) to inspect tool contracts, or (`create`, `remove`) to register/remove modular custom tools in `.harness/tools/` (shell command templates or HTTP requests) when specialized repeatable workflows are needed — newly created tools hot-reload on the next turn.\n\
             3. Collaboration & Orchestration:\n\
             {}- Swarm Message Board (`swarm_board`): Coordinate across background agents (`set_title`, `create_job`, `create_specialist`, `assign_job`, `wait`, `post`, `read`, `list_agents`, `status`, `cancel`) using `@mentions`, `#seq` references, and concise summaries.\n\
             - Artifacts (`create_artifact`): Save plans, walkthroughs, specs, and reports into `.harness_artifacts/` for the UI Artifact Inspector.\n\
             4. Autonomous Browser & Web Verification:\n\
             {}- Whenever you build or modify a web app, HTML page, game, UI, or HTTP server, you MUST interactively verify it in a real browser using `browser_ui` (`navigate`, `snapshot`, `debug_audit`, multi-step `interact` testing UI controls/gameplay/state, and `screenshot`, plus `storage`/`inspect_dom`/`http_check` as needed) with zero console or runtime errors BEFORE finishing — NEVER substitute `node -e` or regex checks and claim browser verification.\n\
             - `browser_ui` directly supports workspace-relative paths (e.g. `\"index.html\"`) and workspace `file://` URLs via automatic local HTTP serving.\n\
             5. Self-Correction & Resource Efficiency:\n\
             - Diagnose tool errors and adapt; avoid unbounded outputs; keep edits minimal and verified.\n\
             6. Completion: When the task is accomplished, provide a clear, comprehensive final summary without invoking further tools.{}{}",
            self.sandbox.display(),
            host_env_line,
            subagent_note,
            knowledge_section,
            extra_tools_list,
            delegation_guidance,
            google_search_guidance,
            orchestrator_section,
            skills_snippet
        )
    }

    async fn drain_steer_inbox(
        &self,
        context: &mut ConversationContext,
        event_tx: &mpsc::Sender<HarnessEvent>,
    ) -> usize {
        let Some(ref inbox) = self.steer_inbox else {
            return 0;
        };
        let drained: Vec<String> = {
            let mut guard = inbox.lock().await;
            guard.drain(..).collect()
        };
        let mut applied = 0usize;
        for s in drained {
            if s.trim().is_empty() {
                continue;
            }
            let _ = event_tx
                .send(HarnessEvent::SteeringApplied { content: s.clone() })
                .await;
            context.messages.push(ChatMessage::user(format!("[USER STEERING]: {}", s)));
            applied += 1;
        }
        applied
    }

    pub async fn run(&self, event_tx: mpsc::Sender<HarnessEvent>) -> Result<String, String> {
        self.run_with_messages(event_tx)
            .await
            .map(|(ans, _)| ans)
    }

    pub async fn run_with_messages(
        &self,
        event_tx: mpsc::Sender<HarnessEvent>,
    ) -> Result<(String, Vec<ChatMessage>), String> {
        let start_time = Instant::now();
        let mut total_tool_calls = 0;
        let mut total_tokens = 0;
        let mut swarm_exit_nudges = 0usize;

        let _ = event_tx.send(HarnessEvent::SessionStarted {
            session_id: self.session_id.clone(),
            task: self.task.clone(),
        }).await;

        let mut effective_mem = self.config.memory.clone();
        effective_mem.max_context_tokens = crate::models::capabilities::effective_max_context_tokens(
            &self.config.model,
            effective_mem.max_context_tokens,
            effective_mem.auto_detect_context,
            None,
        );

        let mut context = if !self.prior_messages.is_empty() {
            let mut ctx = ConversationContext::from_messages(
                self.prior_messages.clone(),
                self.config.max_tool_output_bytes,
            )
            .with_memory_config(effective_mem.clone());
            let has_base_system = ctx.messages.first().is_some_and(|m| {
                m.role == "system"
                    && m.content
                        .as_deref()
                        .is_some_and(|c| is_harness_system_prompt(c))
            });
            if has_base_system {
                ctx.messages[0] = ChatMessage::system(self.build_system_prompt());
            } else {
                ctx.messages.insert(0, ChatMessage::system(self.build_system_prompt()));
            }
            if !self.task.trim().is_empty() || !self.task_attachments.is_empty() {
                ctx.messages.push(ChatMessage::user_with_attachments(
                    &self.task,
                    self.task_attachments.clone(),
                ));
            }
            ctx
        } else {
            let mut ctx = ConversationContext::new(self.config.max_tool_output_bytes)
                .with_memory_config(effective_mem.clone());
            ctx.messages.push(ChatMessage::system(self.build_system_prompt()));
            if !self.task.trim().is_empty() || !self.task_attachments.is_empty() {
                ctx.messages.push(ChatMessage::user_with_attachments(
                    &self.task,
                    self.task_attachments.clone(),
                ));
            }
            ctx
        };

        let swarm = match self.state {
            Some(ref st) => Some(st.get_or_create_swarm(&self.session_id).await),
            None => None,
        };
        let tool_ctx = SessionToolContext {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            parent_id: self.parent_id.clone(),
            depth: self.depth,
            max_depth: self.max_depth,
            state: self.state.clone(),
            swarm: swarm.clone(),
            event_tx: Some(event_tx.clone()),
            config: self.config.clone(),
            registry: self.registry.clone(),
            model: self.model.clone(),
            cancel_notify: self.cancel_notify.clone(),
            sandbox_roots: {
                let mut roots = Vec::new();
                if let Some(ref st) = self.state {
                    roots = st.resolve_session_roots(&self.session_id).await;
                }
                if roots.is_empty() {
                    roots.push(self.sandbox.clone());
                }
                // Folders the user explicitly approved outside the project.
                let approved = self.config.extra_access_roots.clone();
                for root in approved {
                    if !roots.iter().any(|r| r == &root) {
                        roots.push(root);
                    }
                }
                roots
            },
        };

        let mut final_answer = String::new();
        let mut consecutive_sig_failures = 0usize;
        let mut last_failed_sig: Option<String> = None;
        let mut consecutive_tool_failures = 0usize;
        let mut last_failed_tool: Option<String> = None;
        let mut repeated_readonly_sig: Option<(String, usize)> = None;
        let mut empty_answer_nudged = false;
        let mut session_prompt_tokens = 0usize;
        let mut session_completion_tokens = 0usize;
        let mut session_cached_tokens = 0usize;

        // System monitor for live lightweight metrics
        let mut sys = System::new();
        let pid = Pid::from_u32(std::process::id());
        let mut step = 0usize;

        loop {
            step += 1;
            // Enforce the configured iteration cap (0 = unlimited).
            if self.config.max_iterations > 0 && step > self.config.max_iterations {
                let msg = format!(
                    "Iteration limit reached (max_iterations = {}). Stopping to prevent an unbounded loop.",
                    self.config.max_iterations
                );
                let _ = event_tx
                    .send(HarnessEvent::Error {
                        message: msg.clone(),
                        fatal: true,
                    })
                    .await;
                return Err(msg);
            }
            let _ = event_tx.send(HarnessEvent::IterationStep {
                step,
                max_steps: self.config.max_iterations,
            }).await;

            // Drain any mid-run user steering before the LLM turn
            self.drain_steer_inbox(&mut context, &event_tx).await;

            // Hot-reload tool registry each turn so agent-created/removed tools appear immediately
            let active_registry = if let Some(ref st) = self.state {
                st.get_registry_snapshot().await
            } else {
                self.registry.clone()
            };
            let tools_schema = active_registry.get_tools_schema_for_mode(
                self.depth < self.max_depth,
                self.config.knowledge_only,
            );

            // Auto-compact conversation memory before each model call when threshold is exceeded
            if let Some(report) = context.compact_if_needed() {
                let _ = event_tx
                    .send(HarnessEvent::MemoryCompacted {
                        before_tokens: report.before_tokens,
                        after_tokens: report.after_tokens,
                        messages_folded: report.messages_folded,
                        reason: "auto_threshold".into(),
                    })
                    .await;
            }

            // Stream turn from model (with 1 transient retry) or handle user stop cancellation
            let mut model_attempt = 0usize;
            let max_model_retries = 1usize;
            let turn_result = loop {
                let stream_res = tokio::select! {
                    _ = self.cancel_notify.notified() => {
                        if self.depth == 0 {
                            if let Some(ref sw) = swarm {
                                sw.cancel_all_subagents().await;
                            }
                        }
                        let duration_secs = start_time.elapsed().as_secs_f64();
                        let _ = event_tx.send(HarnessEvent::Warning {
                            message: "Session stopped by user request.".to_string(),
                        }).await;
                        let _ = event_tx.send(HarnessEvent::SessionCompleted {
                            session_id: self.session_id.clone(),
                            final_answer: if final_answer.is_empty() { "(Session stopped by user)".to_string() } else { final_answer },
                            total_steps: step,
                            duration_secs,
                        }).await;
                        return Ok(("(Session stopped by user)".to_string(), context.messages));
                    }
                    res = self.model.stream_completion(&context.messages, &tools_schema, &event_tx) => res,
                };

                match stream_res {
                    Ok(res) => break res,
                    Err(err) => {
                        if model_attempt < max_model_retries && is_transient_model_error(&err) {
                            model_attempt += 1;
                            let _ = event_tx.send(HarnessEvent::Warning {
                                message: format!(
                                    "Transient model stream error ({}); retrying in 1.5s (attempt {}/{})...",
                                    err,
                                    model_attempt + 1,
                                    max_model_retries + 1
                                ),
                            }).await;
                            tokio::select! {
                                _ = self.cancel_notify.notified() => {
                                    if self.depth == 0 {
                                        if let Some(ref sw) = swarm {
                                            sw.cancel_all_subagents().await;
                                        }
                                    }
                                    let duration_secs = start_time.elapsed().as_secs_f64();
                                    let _ = event_tx.send(HarnessEvent::Warning {
                                        message: "Session stopped by user request.".to_string(),
                                    }).await;
                                    let _ = event_tx.send(HarnessEvent::SessionCompleted {
                                        session_id: self.session_id.clone(),
                                        final_answer: if final_answer.is_empty() { "(Session stopped by user)".to_string() } else { final_answer },
                                        total_steps: step,
                                        duration_secs,
                                    }).await;
                                    return Ok(("(Session stopped by user)".to_string(), context.messages));
                                }
                                _ = tokio::time::sleep(Duration::from_millis(1500)) => {}
                            }
                            continue;
                        }
                        let _ = event_tx.send(HarnessEvent::Error {
                            message: format!("Model request failed: {}", err),
                            fatal: true,
                        }).await;
                        return Err(err);
                    }
                }
            };

            total_tokens += turn_result.estimated_tokens;

            // Push assistant message into context
            context.messages.push(ChatMessage::assistant(
                turn_result.content.clone(),
                turn_result.reasoning_content.clone(),
                if turn_result.tool_calls.is_empty() { None } else { Some(turn_result.tool_calls.clone()) },
            ));

            if let Some(ref c) = turn_result.content {
                final_answer = c.clone();
            }

            // Emit live metrics
            sys.refresh_process(pid);
            let rss_mb = sys.process(pid)
                .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
                .unwrap_or(0.0);

            let _ = event_tx.send(HarnessEvent::MetricsUpdate {
                rss_memory_mb: rss_mb,
                cpu_usage_pct: 0.0,
                total_tokens_est: total_tokens,
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                tool_invocations: total_tool_calls,
            }).await;

            // Context window & token usage calculation
            let turn_prompt = if turn_result.prompt_tokens > 0 {
                turn_result.prompt_tokens
            } else {
                context.estimate_tokens()
            };
            session_prompt_tokens += turn_prompt;
            session_completion_tokens += turn_result.estimated_tokens;
            let turn_cached = turn_result.cached_tokens.min(turn_prompt);
            session_cached_tokens += turn_cached;

            let total_session_tokens = session_prompt_tokens + session_completion_tokens;
            let current_ctx = context.estimate_tokens();
            let max_ctx = effective_mem.max_context_tokens.max(1);
            let cache_hit_rate = if session_prompt_tokens > 0 {
                (((session_cached_tokens as f32) / (session_prompt_tokens as f32)) * 100.0)
                    .clamp(0.0, 100.0)
            } else {
                0.0
            };
            let cost_usd = (session_prompt_tokens as f64 * 0.00000015) + (session_completion_tokens as f64 * 0.00000060);

            let _ = event_tx.send(HarnessEvent::TokenUsageUpdate {
                current_context_tokens: current_ctx,
                max_context_tokens: max_ctx,
                prompt_tokens: session_prompt_tokens,
                completion_tokens: session_completion_tokens,
                total_tokens: total_session_tokens,
                cached_tokens: session_cached_tokens,
                cache_hit_rate_pct: cache_hit_rate,
                estimated_cost_usd: cost_usd,
            }).await;

            // If no tool calls requested, check for late steering, active background swarm workers, or empty-response recovery
            if turn_result.tool_calls.is_empty() {
                if self.drain_steer_inbox(&mut context, &event_tx).await > 0 {
                    continue;
                }
                if self.depth == 0 && swarm_exit_nudges < 24 {
                    if let Some(ref st) = self.state {
                        if let Some(swarm) = st.get_session_swarm(&self.session_id).await {
                            let active = swarm.active_subagent_count().await;
                            if active > 0 {
                                swarm_exit_nudges += 1;
                                let since_seq = swarm.latest_seq();
                                let wait_res = swarm
                                    .wait_for_activity_or_cancel(
                                        &self.agent_id,
                                        since_seq,
                                        false,
                                        25,
                                        &self.cancel_notify,
                                    )
                                    .await;
                                match wait_res {
                                    None => {
                                        // Session cancelled; allow clean shutdown
                                    }
                                    Some((new_msgs, remaining_active, timed_out)) => {
                                        let sub_list = swarm.list_subagents().await;
                                        let active_names: Vec<String> = sub_list
                                            .iter()
                                            .filter(|s| matches!(s.status.as_str(), "running" | "waiting"))
                                            .map(|s| format!("@{} ({})", s.id, s.status))
                                            .collect();
                                        let failed_names: Vec<String> = sub_list
                                            .iter()
                                            .filter(|s| matches!(s.status.as_str(), "failed" | "stopped"))
                                            .map(|s| format!("@{} [{}]", s.id, s.status))
                                            .collect();
                                        let mut note = String::new();
                                        if !new_msgs.is_empty() {
                                            note.push_str("New swarm board updates while waiting:\n");
                                            for m in &new_msgs {
                                                let tgt = m
                                                    .target
                                                    .as_deref()
                                                    .map(|t| format!(" -> @{}", t))
                                                    .unwrap_or_default();
                                                note.push_str(&format!("#{} [{}{}]: {}\n", m.seq, m.sender, tgt, m.content));
                                            }
                                        }
                                        if remaining_active > 0 {
                                            note.push_str(&format!(
                                                "[Swarm Guard] {} background subagent(s) still running{}: {}{}. Wait zero-CPU using `swarm_board{{action:\"wait\", since:{}}}` (or `wait_all: true`) before concluding.",
                                                remaining_active,
                                                if timed_out { " (25s wait elapsed)" } else { "" },
                                                active_names.join(", "),
                                                if failed_names.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!("; failed/stopped: {}", failed_names.join(", "))
                                                },
                                                swarm.latest_seq()
                                            ));
                                        } else {
                                            note.push_str(&format!(
                                                "[Swarm Guard] All background subagents have now finished{}. Review their board messages above, update any open job tags via `complete_job`, verify if needed, and write your complete final response.",
                                                if failed_names.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!(" (note failures: {})", failed_names.join(", "))
                                                }
                                            ));
                                        }
                                        context.messages.push(ChatMessage::user(note));
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
                if final_answer.trim().is_empty() && total_tool_calls > 0 && !empty_answer_nudged {
                    empty_answer_nudged = true;
                    context.messages.push(ChatMessage::user(
                        "You executed tool calls above but did not write a final user-facing response. Summarize the verified results concisely or complete any remaining step."
                            .to_string(),
                    ));
                    continue;
                }
                let duration_secs = start_time.elapsed().as_secs_f64();
                let _ = event_tx.send(HarnessEvent::SessionCompleted {
                    session_id: self.session_id.clone(),
                    final_answer: final_answer.clone(),
                    total_steps: step,
                    duration_secs,
                }).await;
                return Ok((final_answer, context.messages));
            }

            // Execute requested tool calls (concurrent batching for contiguous auto-allowed read-only calls)
            let mut pending_circuit_breaker_alert: Option<String> = None;
            let max_tool_retries = self.config.memory.max_tool_retries.min(10);
            let max_tool_attempts = 1 + max_tool_retries;

            let total_calls_in_turn = turn_result.tool_calls.len();
            let mut tc_idx = 0usize;
            // Permission requests in this turn that were denied or timed out.
            let mut turn_permission_denials = 0usize;
            while tc_idx < total_calls_in_turn {
                let exec_registry = if let Some(ref st) = self.state {
                    st.get_registry_snapshot().await
                } else {
                    active_registry.clone()
                };

                // Look ahead for a contiguous batch of >= 2 auto-allowed read-only tool calls
                let mut batch_items: Vec<(
                    usize,
                    &crate::engine::context::ToolCall,
                    serde_json::Value,
                    Option<ToolContract>,
                )> = Vec::new();
                let mut look = tc_idx;
                while look < total_calls_in_turn {
                    let tc = &turn_result.tool_calls[look];
                    let tname = tc.function.name.as_str();
                    let contract = exec_registry.get_contract(tname);
                    if self.config.knowledge_only
                        && !crate::plugins::is_tool_allowed_in_knowledge_mode(
                            tname,
                            contract.as_ref(),
                        )
                    {
                        break;
                    }
                    let policy = self.check_permission_policy(tname, &exec_registry);
                    if policy != PermissionLevel::Allow {
                        break;
                    }
                    let Ok(parsed) = recovery::parse_and_repair_tool_args(&tc.function.arguments) else {
                        break;
                    };
                    if !is_concurrent_safe_tool_call(tname, &parsed, contract.as_ref()) {
                        break;
                    }
                    batch_items.push((look, tc, parsed, contract));
                    look += 1;
                }

                if batch_items.len() >= 2 {
                    for (_, tc, parsed, _) in &batch_items {
                        total_tool_calls += 1;
                        let _ = event_tx
                            .send(HarnessEvent::ToolCallStarted {
                                call_id: tc.id.clone(),
                                tool: tc.function.name.clone(),
                                args: parsed.clone(),
                            })
                            .await;
                    }

                    let batch_futures = batch_items.iter().map(|(_, tc, parsed, contract)| {
                        let reg = exec_registry.clone();
                        let contract_owned = contract.clone();
                        let mut step_tool_ctx = tool_ctx.clone();
                        step_tool_ctx.registry = exec_registry.clone();
                        let tname = tc.function.name.clone();
                        let cid = tc.id.clone();
                        let args_val = parsed.clone();
                        let sandbox_ref = self.sandbox.clone();
                        let cancel_ref = self.cancel_notify.clone();
                        let tx_ref = event_tx.clone();
                        let t_secs = tool_timeout_secs(&self.config, &tname, contract.as_ref());
                        async move {
                            execute_tool_with_retries(
                                &reg,
                                contract_owned.as_ref(),
                                &tname,
                                &cid,
                                &args_val,
                                &sandbox_ref,
                                &step_tool_ctx,
                                &cancel_ref,
                                &tx_ref,
                                t_secs,
                                max_tool_attempts,
                            )
                            .await
                        }
                    });

                    let batch_outcomes = join_all(batch_futures).await;
                    for ((orig_idx, tc, parsed, _), outcome) in
                        batch_items.into_iter().zip(batch_outcomes.into_iter())
                    {
                        let tool_name = &tc.function.name;
                        let (duration_ms, final_exec) = match outcome {
                            ToolRetryRunResult::Completed { duration_ms, result } => (duration_ms, result),
                            ToolRetryRunResult::Cancelled => {
                                if self.depth == 0 {
                                    if let Some(ref sw) = swarm {
                                        sw.cancel_all_subagents().await;
                                    }
                                }
                                for remaining_tc in &turn_result.tool_calls[orig_idx..] {
                                    context.messages.push(ChatMessage::tool(
                                        remaining_tc.id.clone(),
                                        "Error: Tool execution stopped by user.",
                                    ));
                                }
                                let duration_secs = start_time.elapsed().as_secs_f64();
                                let _ = event_tx
                                    .send(HarnessEvent::Warning {
                                        message: "Session stopped by user during parallel tool execution."
                                            .to_string(),
                                    })
                                    .await;
                                let _ = event_tx
                                    .send(HarnessEvent::SessionCompleted {
                                        session_id: self.session_id.clone(),
                                        final_answer: if final_answer.is_empty() {
                                            "(Session stopped by user)".to_string()
                                        } else {
                                            final_answer
                                        },
                                        total_steps: step,
                                        duration_secs,
                                    })
                                    .await;
                                return Ok(("(Session stopped by user)".to_string(), context.messages));
                            }
                        };

                        let (success, raw_output) = match final_exec {
                            Ok(out) => {
                                consecutive_sig_failures = 0;
                                last_failed_sig = None;
                                consecutive_tool_failures = 0;
                                last_failed_tool = None;
                                let ro_sig = recovery::normalized_failure_signature(tool_name, &parsed);
                                match repeated_readonly_sig {
                                    Some((ref prev_sig, count)) if prev_sig == &ro_sig => {
                                        let next = count + 1;
                                        repeated_readonly_sig = Some((ro_sig, next));
                                        if next >= 3 && pending_circuit_breaker_alert.is_none() {
                                            pending_circuit_breaker_alert = Some(format!(
                                                "Loop Guard: Read-only call '{}' was repeated {} times with identical arguments and no intervening changes. Reuse the existing result above and move on to editing or answering.",
                                                tool_name, next
                                            ));
                                        }
                                    }
                                    _ => {
                                        repeated_readonly_sig = Some((ro_sig, 1));
                                    }
                                }
                                (true, out)
                            }
                            Err((err, diagnosis)) => {
                                let norm_sig = recovery::normalized_failure_signature(tool_name, &parsed);
                                if last_failed_sig.as_deref() == Some(&norm_sig) {
                                    consecutive_sig_failures += 1;
                                } else {
                                    consecutive_sig_failures = 1;
                                    last_failed_sig = Some(norm_sig);
                                }
                                if last_failed_tool.as_deref() == Some(tool_name.as_str()) {
                                    consecutive_tool_failures += 1;
                                } else {
                                    consecutive_tool_failures = 1;
                                    last_failed_tool = Some(tool_name.clone());
                                }
                                if consecutive_sig_failures >= 2 || consecutive_tool_failures >= 3 {
                                    pending_circuit_breaker_alert = Some(format!(
                                        "Self-Correction Alert: Tool '{}' failed repeatedly ({} consecutive tool failures, {} identical signature failures). Do not retry the exact same parameters or approach. Recovery guidance: {}",
                                        tool_name,
                                        consecutive_tool_failures,
                                        consecutive_sig_failures,
                                        diagnosis.recovery_hint
                                    ));
                                }
                                (
                                    false,
                                    recovery::format_tool_error_with_hint(tool_name, &err, &diagnosis),
                                )
                            }
                        };

                        let (truncated_output, was_truncated) =
                            context.truncate_tool_output(&raw_output);
                        let _ = event_tx
                            .send(HarnessEvent::ToolCallFinished {
                                call_id: tc.id.clone(),
                                tool: tool_name.clone(),
                                success,
                                output: truncated_output.clone(),
                                duration_ms,
                                truncated: was_truncated,
                            })
                            .await;
                        context
                            .messages
                            .push(ChatMessage::tool(tc.id.clone(), truncated_output));
                    }

                    tc_idx = look;
                    continue;
                }

                // Single / Sequential execution path (mutations, permission-gated tools, or parse errors)
                let tool_call = &turn_result.tool_calls[tc_idx];
                total_tool_calls += 1;
                let tool_name = &tool_call.function.name;
                let raw_args = &tool_call.function.arguments;
                let tool_contract = exec_registry.get_contract(tool_name);

                let parsed_args: serde_json::Value = match recovery::parse_and_repair_tool_args(raw_args) {
                    Ok(val) => val,
                    Err(parse_err) => {
                        let diagnosis = recovery::diagnose_tool_error(
                            tool_name,
                            &parse_err,
                            tool_contract.as_ref(),
                        );
                        let formatted_err =
                            recovery::format_tool_error_with_hint(tool_name, &parse_err, &diagnosis);
                        let (truncated_output, was_truncated) =
                            context.truncate_tool_output(&formatted_err);

                        let _ = event_tx
                            .send(HarnessEvent::ToolCallStarted {
                                call_id: tool_call.id.clone(),
                                tool: tool_name.clone(),
                                args: serde_json::json!({ "invalid_json": true }),
                            })
                            .await;
                        let _ = event_tx
                            .send(HarnessEvent::ToolCallFinished {
                                call_id: tool_call.id.clone(),
                                tool: tool_name.clone(),
                                success: false,
                                output: truncated_output.clone(),
                                duration_ms: 0,
                                truncated: was_truncated,
                            })
                            .await;

                        let norm_sig = format!("{}:__invalid_json__", tool_name);
                        if last_failed_sig.as_deref() == Some(&norm_sig) {
                            consecutive_sig_failures += 1;
                        } else {
                            consecutive_sig_failures = 1;
                            last_failed_sig = Some(norm_sig);
                        }
                        if last_failed_tool.as_deref() == Some(tool_name.as_str()) {
                            consecutive_tool_failures += 1;
                        } else {
                            consecutive_tool_failures = 1;
                            last_failed_tool = Some(tool_name.clone());
                        }
                        if consecutive_sig_failures >= 2 || consecutive_tool_failures >= 3 {
                            pending_circuit_breaker_alert = Some(format!(
                                "Self-Correction Alert: Tool '{}' failed repeatedly ({} consecutive tool failures, {} identical signature failures). Stop retrying the same arguments. Recovery guidance: {}",
                                tool_name, consecutive_tool_failures, consecutive_sig_failures, diagnosis.recovery_hint
                            ));
                        }

                        context
                            .messages
                            .push(ChatMessage::tool(tool_call.id.clone(), truncated_output));
                        tc_idx += 1;
                        continue;
                    }
                };

                let mut step_tool_ctx = tool_ctx.clone();
                step_tool_ctx.registry = exec_registry.clone();

                if self.config.knowledge_only
                    && !crate::plugins::is_tool_allowed_in_knowledge_mode(
                        tool_name,
                        tool_contract.as_ref(),
                    )
                {
                    let msg = format!(
                        "Tool '{}' is disabled in Knowledge-Work Conversation mode (read/write/shell tools require a Project workspace).",
                        tool_name
                    );
                    let _ = event_tx.send(HarnessEvent::Warning { message: msg.clone() }).await;
                    context.messages.push(ChatMessage::tool(tool_call.id.clone(), format!("Error: {}", msg)));
                    tc_idx += 1;
                    continue;
                }

                // Check contract-driven permission policy
                let policy = self.check_permission_policy(tool_name, &exec_registry);
                match policy {
                    PermissionLevel::Deny => {
                        let msg = format!("Tool '{}' execution blocked by harness security policy.", tool_name);
                        let _ = event_tx.send(HarnessEvent::Warning { message: msg.clone() }).await;
                        context.messages.push(ChatMessage::tool(tool_call.id.clone(), format!("Error: {}", msg)));
                        tc_idx += 1;
                        continue;
                    }
                    PermissionLevel::Ask => {
                        // Already approved for this session? Skip the prompt
                        // entirely instead of re-asking for every call.
                        let already_allowed = match self.state {
                            Some(ref st) => st.is_tool_allowed_for_session(&self.session_id, tool_name).await,
                            None => false,
                        };
                        if already_allowed {
                            // Say it once per session/tool, not once per call: a
                            // chatty tool would otherwise spam the stream with
                            // identical notices.
                            if let Some(ref st) = self.state {
                                let first = !st
                                    .is_tool_allow_noted(&self.session_id, tool_name)
                                    .await;
                                if first {
                                    st.note_tool_allowed(&self.session_id, tool_name).await;
                                    let _ = event_tx
                                        .send(HarnessEvent::Warning {
                                            message: format!(
                                                "Tool '{tool_name}' is approved for the rest of this session (always allow)."
                                            ),
                                        })
                                        .await;
                                }
                            }
                        } else if let Some(ref st) = self.state {
                            let perm_seq = PERM_SEQ.fetch_add(1, Ordering::Relaxed);
                            let perm_id = format!(
                                "perm_{}_{}_{}_{}",
                                self.session_id, self.agent_id, total_tool_calls, perm_seq
                            );
                            let (perm_tx, perm_rx) = oneshot::channel();
                            // Record (tool, session) so a later "always allow"
                            // decision can name the tool it applies to. For
                            // `request_path_access` also record the path: the
                            // grant is made when a HUMAN answers, and the tool
                            // itself must never trust its own arguments.
                            let requested_path = if tool_name == "request_path_access" {
                                parsed_args
                                    .get("path")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                            } else {
                                None
                            };
                            st.register_permission_waiter_with_meta_and_path(
                                perm_id.clone(),
                                self.session_id.clone(),
                                tool_name.clone(),
                                requested_path,
                                perm_tx,
                            )
                            .await;

                            let desc = self.tool_permission_description(tool_name, &parsed_args);
                            let _ = event_tx.send(HarnessEvent::PermissionRequested {
                                perm_id: perm_id.clone(),
                                session_id: self.session_id.clone(),
                                tool: tool_name.clone(),
                                description: desc,
                                args: parsed_args.clone(),
                            }).await;

                            // Wait for user decision with 120s timeout, but let Stop
                            // interrupt immediately instead of appearing dead.
                            let decision = tokio::select! {
                                _ = self.cancel_notify.notified() => {
                                    st.cancel_permission_waiter(&perm_id).await;
                                    return Err("Stopped while awaiting approval.".to_string());
                                }
                                d = tokio::time::timeout(Duration::from_secs(120), perm_rx) => d,
                            };
                            let approved = match decision {
                                Ok(Ok(true)) => true,
                                _ => false,
                            };
                            if !approved {
                                // Remove the waiter on every non-approved path so
                                // pending entries cannot accumulate.
                                st.cancel_permission_waiter(&perm_id).await;
                            }

                            let _ = event_tx.send(HarnessEvent::PermissionDecided {
                                perm_id,
                                tool: tool_name.clone(),
                                approved,
                            }).await;

                            if !approved {
                                // Only a *denied* (or timed-out) request counts
                                // toward the per-turn fail-fast cap. An approved
                                // request must never accumulate here, or a turn
                                // that legitimately needs several approvals would
                                // be aborted after the third one.
                                turn_permission_denials += 1;
                                if turn_permission_denials > 3 {
                                    let msg = format!(
                                        "Aborting turn: {turn_permission_denials} permission requests were \
                                         denied or timed out. Approve the tool in the permission prompt, or \
                                         switch the permission mode to Autonomous."
                                    );
                                    let _ = event_tx
                                        .send(HarnessEvent::Warning { message: msg.clone() })
                                        .await;
                                    context.messages.push(ChatMessage::tool(
                                        tool_call.id.clone(),
                                        format!("Error: {msg}"),
                                    ));
                                    return Err(msg);
                                }
                                let msg = format!("Execution denied by user permission policy for tool '{}'.", tool_name);
                                context.messages.push(ChatMessage::tool(tool_call.id.clone(), format!("Error: {}", msg)));
                                tc_idx += 1;
                                continue;
                            }
                        } else {
                            // Permission required but no approval channel (state-less
                            // runner): fail closed instead of executing unapproved.
                            let msg = format!(
                                "Error: tool '{}' requires approval but no approval channel is available.",
                                tool_name
                            );
                            context.messages.push(ChatMessage::tool(tool_call.id.clone(), msg));
                            tc_idx += 1;
                            continue;
                        }
                    }
                    PermissionLevel::Allow => {}
                }

                let _ = event_tx.send(HarnessEvent::ToolCallStarted {
                    call_id: tool_call.id.clone(),
                    tool: tool_name.clone(),
                    args: parsed_args.clone(),
                }).await;

                let timeout_secs = tool_timeout_secs(&self.config, tool_name, tool_contract.as_ref());
                let (duration_ms, final_exec) = match execute_tool_with_retries(
                    &exec_registry,
                    tool_contract.as_ref(),
                    tool_name,
                    &tool_call.id,
                    &parsed_args,
                    &self.sandbox,
                    &step_tool_ctx,
                    &self.cancel_notify,
                    &event_tx,
                    timeout_secs,
                    max_tool_attempts,
                )
                .await
                {
                    ToolRetryRunResult::Completed { duration_ms, result } => (duration_ms, result),
                    ToolRetryRunResult::Cancelled => {
                        if self.depth == 0 {
                            if let Some(ref sw) = swarm {
                                sw.cancel_all_subagents().await;
                            }
                        }
                        for remaining_tc in &turn_result.tool_calls[tc_idx..] {
                            context.messages.push(ChatMessage::tool(
                                remaining_tc.id.clone(),
                                "Error: Tool execution stopped by user.",
                            ));
                        }
                        let duration_secs = start_time.elapsed().as_secs_f64();
                        let _ = event_tx.send(HarnessEvent::Warning {
                            message: "Session stopped by user during tool execution.".to_string(),
                        }).await;
                        let _ = event_tx.send(HarnessEvent::SessionCompleted {
                            session_id: self.session_id.clone(),
                            final_answer: if final_answer.is_empty() { "(Session stopped by user)".to_string() } else { final_answer },
                            total_steps: step,
                            duration_secs,
                        }).await;
                        return Ok(("(Session stopped by user)".to_string(), context.messages));
                    }
                };

                let (success, raw_output) = match final_exec {
                    Ok(out) => {
                        consecutive_sig_failures = 0;
                        last_failed_sig = None;
                        consecutive_tool_failures = 0;
                        last_failed_tool = None;
                        if is_mutating_tool_call(tool_name, &parsed_args) {
                            repeated_readonly_sig = None;
                        } else if is_concurrent_safe_tool_call(
                            tool_name,
                            &parsed_args,
                            tool_contract.as_ref(),
                        ) {
                            let ro_sig =
                                recovery::normalized_failure_signature(tool_name, &parsed_args);
                            match repeated_readonly_sig {
                                Some((ref prev_sig, count)) if prev_sig == &ro_sig => {
                                    let next = count + 1;
                                    repeated_readonly_sig = Some((ro_sig, next));
                                    if next >= 3 && pending_circuit_breaker_alert.is_none() {
                                        pending_circuit_breaker_alert = Some(format!(
                                            "Loop Guard: Read-only call '{}' was repeated {} times with identical arguments and no intervening changes. Reuse the existing result above and move on to editing or answering.",
                                            tool_name, next
                                        ));
                                    }
                                }
                                _ => {
                                    repeated_readonly_sig = Some((ro_sig, 1));
                                }
                            }
                        }
                        (true, out)
                    }
                    Err((err, diagnosis)) => {
                        let norm_sig =
                            recovery::normalized_failure_signature(tool_name, &parsed_args);
                        if last_failed_sig.as_deref() == Some(&norm_sig) {
                            consecutive_sig_failures += 1;
                        } else {
                            consecutive_sig_failures = 1;
                            last_failed_sig = Some(norm_sig);
                        }
                        if last_failed_tool.as_deref() == Some(tool_name.as_str()) {
                            consecutive_tool_failures += 1;
                        } else {
                            consecutive_tool_failures = 1;
                            last_failed_tool = Some(tool_name.clone());
                        }
                        if consecutive_sig_failures >= 2 || consecutive_tool_failures >= 3 {
                            pending_circuit_breaker_alert = Some(format!(
                                "Self-Correction Alert: Tool '{}' failed repeatedly ({} consecutive tool failures, {} identical signature failures). Do not retry the exact same parameters or approach. Recovery guidance: {}",
                                tool_name,
                                consecutive_tool_failures,
                                consecutive_sig_failures,
                                diagnosis.recovery_hint
                            ));
                        }
                        (
                            false,
                            recovery::format_tool_error_with_hint(tool_name, &err, &diagnosis),
                        )
                    }
                };

                if success && tool_name == "browser_ui" {
                    if let Some(snapshot_evt) = extract_browser_snapshot(&raw_output, &parsed_args) {
                        let _ = event_tx.send(snapshot_evt).await;
                    }
                }

                let (truncated_output, was_truncated) = context.truncate_tool_output(&raw_output);

                let _ = event_tx.send(HarnessEvent::ToolCallFinished {
                    call_id: tool_call.id.clone(),
                    tool: tool_name.clone(),
                    success,
                    output: truncated_output.clone(),
                    duration_ms,
                    truncated: was_truncated,
                }).await;

                // Feed tool result back into context for next turn
                context.messages.push(ChatMessage::tool(tool_call.id.clone(), truncated_output));
                tc_idx += 1;
            }

            if let Some(alert) = pending_circuit_breaker_alert {
                context.messages.push(ChatMessage::system(alert));
            }

            // Drain any mid-run user steering after all tool responses for this turn are recorded
            self.drain_steer_inbox(&mut context, &event_tx).await;
        }
    }
}

fn extract_browser_snapshot(raw_output: &str, args: &serde_json::Value) -> Option<HarnessEvent> {
    let parsed: Option<serde_json::Value> = serde_json::from_str(raw_output)
        .ok()
        .or_else(|| {
            // Prefer fenced ```json ... ``` block emitted by BrowserUiPlugin.
            let marker = "```json";
            let start = raw_output.find(marker)? + marker.len();
            let rest = &raw_output[start..];
            let end = rest.find("```")?;
            serde_json::from_str(rest[..end].trim()).ok()
        })
        .or_else(|| {
            let start = raw_output.find('{')?;
            let end = raw_output.rfind('}')?;
            if end > start {
                serde_json::from_str(&raw_output[start..=end]).ok()
            } else {
                None
            }
        });

    let val = parsed?;
    let url = val
        .get("url")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("url").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let screenshot_url = val
        .get("screenshot_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if url.is_empty() && screenshot_url.is_none() {
        return None;
    }

    let console_errors = val
        .get("console_errors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .or_else(|| Some(item.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    let dom_summary = val
        .get("dom_summary")
        .or_else(|| val.get("text_summary"))
        .or_else(|| val.get("summary"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(500)
        .collect();

    Some(HarnessEvent::BrowserSnapshot {
        url,
        title,
        screenshot_url,
        console_errors,
        dom_summary,
    })
}
