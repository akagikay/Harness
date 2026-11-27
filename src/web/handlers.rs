use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    http::{header, StatusCode},
    Json,
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use sysinfo::{Pid, System};
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, mpsc, Mutex as TokioMutex};

use crate::config::{HarnessConfig, PermissionLevel, PermissionsConfig, SkillDefinition};
use crate::engine::context::{format_attachments_as_text, ChatAttachment, ChatMessage};
use crate::engine::events::HarnessEvent;
use crate::engine::loop_runner::AgentLoopRunner;
use crate::engine::swarm::{ArtifactRecord, JobTag, SpecialistTag, SubagentInfo, SwarmMessage};
use crate::models::{
    capabilities::{
        effective_max_context_tokens, infer_model_context_window, infer_model_modalities,
        infer_model_reasoning_spec, resolve_model_capabilities, ModelCapabilities,
        ModelModalityCaps, ModelReasoningSpec,
    },
    google_search_client::{
        is_google_search_model, GoogleSearchClient, EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID,
    },
    mock_client::MockClient,
    openai_client::OpenAiClient,
    ModelAdapter,
};
use crate::plugins::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use crate::plugins::custom_tool::{
    custom_tools_dir, delete_custom_tool, save_custom_tool, validate_tool_name, CustomToolSpec,
};
use crate::plugins::{is_builtin_tool_name, PluginRegistry};
use crate::state::{AppState, QueuedPrompt};

pub const INDEX_HTML: &str = include_str!("static/index.html");
pub const STYLE_CSS: &str = include_str!("static/style.css");
pub const APP_JS: &str = include_str!("static/app.js");

pub async fn get_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

pub async fn get_style() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    ).into_response()
}

pub async fn get_script() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        APP_JS,
    ).into_response()
}

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    pub mock: Option<bool>,
    pub project_id: Option<String>,
    #[serde(default)]
    pub slash_command: Option<String>,
    #[serde(default)]
    pub slash_args: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub orchestrator_mode: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Serialize)]
pub struct CreateTaskResponse {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct SessionSummary {
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
    pub knowledge_only: bool,
}

pub async fn list_sessions(State(state): State<AppState>) -> Json<Vec<SessionSummary>> {
    let lock = state.sessions.read().await;
    let mut list: Vec<SessionSummary> = lock
        .values()
        .map(|s| SessionSummary {
            id: s.id.clone(),
            project_id: s.project_id.clone(),
            created_at: s.created_at.clone(),
            updated_at: s.updated_at.clone(),
            task: s.task.clone(),
            status: s.status.clone(),
            model: s.model.clone(),
            provider_id: s.provider_id.clone(),
            reasoning_effort: s.reasoning_effort.clone(),
            server_id: s.server_id.clone(),
            orchestrator_mode: s.orchestrator_mode,
            knowledge_only: crate::config::is_knowledge_only_project(&s.project_id),
        })
        .collect();
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| b.created_at.cmp(&a.created_at)));
    Json(list)
}

pub async fn get_session_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<HarnessEvent>>, StatusCode> {
    if !state.ensure_session_loaded(&id).await {
        return Err(StatusCode::NOT_FOUND);
    }
    let lock = state.sessions.read().await;
    if let Some(session) = lock.get(&id) {
        Ok(Json(session.history.clone()))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

fn resolve_slash_input(
    conf: &HarnessConfig,
    sandbox: &FsPath,
    raw_content: &str,
    slash_command: Option<&str>,
    slash_args: Option<&str>,
) -> (String, Option<String>, Option<String>) {
    let trimmed_content = raw_content.trim();
    if let Some(cmd) = slash_command.map(str::trim).filter(|s| !s.is_empty()) {
        let norm_cmd = cmd.trim_start_matches('/').to_ascii_lowercase();
        let args = slash_args
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        return (trimmed_content.to_string(), Some(norm_cmd), args);
    }

    if trimmed_content.starts_with('/') {
        let mut parts = trimmed_content.splitn(2, char::is_whitespace);
        let first = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        let candidate = first.trim_start_matches('/');
        let valid_token = !candidate.is_empty()
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if valid_token {
            let is_builtin = crate::state::is_known_builtin_slash_command(candidate);
            let is_skill = !is_builtin
                && conf.discover_skills_in(sandbox).into_iter().any(|s| {
                    if !s.enabled {
                        return false;
                    }
                    let stem = FsPath::new(&s.path)
                        .file_stem()
                        .and_then(|st| st.to_str())
                        .unwrap_or("");
                    s.name.eq_ignore_ascii_case(candidate) || stem.eq_ignore_ascii_case(candidate)
                });
            if is_builtin || is_skill {
                let args = (!rest.is_empty()).then(|| rest.to_string());
                return (String::new(), Some(candidate.to_ascii_lowercase()), args);
            }
        }
    }

    (trimmed_content.to_string(), None, None)
}

fn format_runner_task(
    prompt: &str,
    slash_command: Option<&str>,
    slash_args: Option<&str>,
) -> String {
    let trimmed = prompt.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Some(cmd) = slash_command.map(str::trim).filter(|s| !s.is_empty()) {
        let clean_cmd = cmd.trim_start_matches('/');
        if let Some(args) = slash_args.map(str::trim).filter(|s| !s.is_empty()) {
            return format!("Execute /{} {}", clean_cmd, args);
        }
        return format!("Execute /{}", clean_cmd);
    }
    String::new()
}

fn format_session_title(
    prompt: &str,
    slash_command: Option<&str>,
    slash_args: Option<&str>,
) -> String {
    let trimmed = prompt.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Some(cmd) = slash_command.map(str::trim).filter(|s| !s.is_empty()) {
        let clean_cmd = cmd.trim_start_matches('/');
        if let Some(args) = slash_args.map(str::trim).filter(|s| !s.is_empty()) {
            return format!("/{} {}", clean_cmd, args);
        }
        return format!("/{}", clean_cmd);
    }
    "New Session".to_string()
}

fn queue_to_json_values(queue: &[QueuedPrompt]) -> Vec<serde_json::Value> {
    queue
        .iter()
        .filter_map(|item| serde_json::to_value(item).ok())
        .collect()
}

fn format_session_title_with_attachments(
    prompt: &str,
    attachments: &[ChatAttachment],
    slash_command: Option<&str>,
    slash_args: Option<&str>,
) -> String {
    let base = format_session_title(prompt, slash_command, slash_args);
    if base == "New Session" && !attachments.is_empty() {
        if attachments.len() == 1 {
            format!("Attached {}", attachments[0].name)
        } else {
            format!("Attached {} files", attachments.len())
        }
    } else {
        base
    }
}

async fn prepare_queued_turn(
    state: &AppState,
    session_id: &str,
    queued: &QueuedPrompt,
    remaining: &[QueuedPrompt],
) -> Option<(
    String,
    Vec<ChatAttachment>,
    Vec<ChatMessage>,
    Arc<TokioMutex<VecDeque<String>>>,
)> {
    let display_content = format_session_title_with_attachments(
        &queued.content,
        &queued.attachments,
        queued.slash_command.as_deref(),
        queued.slash_args.as_deref(),
    );
    state
        .emit_session_event(
            session_id,
            HarnessEvent::PromptDequeued {
                queue_id: queued.id.clone(),
                content: display_content,
                mode: "queue".to_string(),
            },
        )
        .await;
    state
        .emit_session_event(
            session_id,
            HarnessEvent::QueueUpdated {
                queue: queue_to_json_values(remaining),
            },
        )
        .await;

    let (_, next_prior, next_steer) = state
        .begin_turn_with_command_and_attachments(
            session_id,
            &queued.content,
            queued.attachments.clone(),
            queued.slash_command.as_deref(),
            queued.slash_args.as_deref(),
        )
        .await?;
    let mut runner_task = format_runner_task(
        &queued.content,
        queued.slash_command.as_deref(),
        queued.slash_args.as_deref(),
    );
    if runner_task.trim().is_empty() && !queued.attachments.is_empty() {
        runner_task = "Analyze the attached file(s).".to_string();
    }
    Some((
        runner_task,
        queued.attachments.clone(),
        next_prior,
        next_steer,
    ))
}

/// Spawn a unified background turn runner that executes `AgentLoopRunner::run_with_messages`,
/// persists conversation messages, and auto-drains queued prompts sequentially.
#[allow(clippy::too_many_arguments)]
pub fn spawn_session_turn(
    state: AppState,
    session_id: String,
    task: String,
    task_attachments: Vec<ChatAttachment>,
    project_id: String,
    prior_messages: Vec<ChatMessage>,
    steer_inbox: Arc<TokioMutex<VecDeque<String>>>,
    bcast_tx: broadcast::Sender<HarnessEvent>,
    cancel_notify: Arc<tokio::sync::Notify>,
    use_mock: bool,
) {
    tokio::spawn(async move {
        let mut current_task = task;
        let mut current_attachments = task_attachments;
        let mut current_prior = prior_messages;
        let mut current_steer = steer_inbox;

        loop {
            let config = { state.config.read().await.clone() };
            let (session_server_id, session_orch_mode, sess_model, sess_provider, sess_effort) = {
                let lock = state.sessions.read().await;
                lock.get(&session_id)
                    .map(|s| (
                        s.server_id.clone(),
                        s.orchestrator_mode,
                        s.model.clone(),
                        s.provider_id.clone(),
                        s.reasoning_effort.clone(),
                    ))
                    .unwrap_or_else(|| (
                        config.active_server_id.clone(),
                        config.orchestrator.enabled,
                        String::new(),
                        String::new(),
                        String::new(),
                    ))
            };
            let project_path = config
                .projects
                .iter()
                .find(|p| p.id == project_id)
                .map(|p| p.path.clone())
                .unwrap_or_else(|| config.sandbox_root.clone());

            let eff_override = if !sess_effort.trim().is_empty() {
                Some(sess_effort.as_str())
            } else if session_orch_mode {
                Some(config.orchestrator.thinking_effort.as_str())
            } else {
                None
            };
            let mut session_config = if session_orch_mode {
                let orch_m = if !config.orchestrator.model.trim().is_empty() && (sess_model.trim().is_empty() || sess_model == config.model) {
                    config.orchestrator.model.as_str()
                } else if !sess_model.trim().is_empty() && sess_model != "mock" {
                    sess_model.as_str()
                } else {
                    config.model.as_str()
                };
                let orch_p = if !sess_provider.trim().is_empty() {
                    Some(sess_provider.as_str())
                } else {
                    Some(config.orchestrator.provider_id.as_str())
                };
                let mut orch_cfg = config.with_model_override(orch_m, orch_p, eff_override);
                orch_cfg.orchestrator.enabled = true;
                orch_cfg
            } else {
                let eff_m = if !sess_model.trim().is_empty() && sess_model != "mock" {
                    sess_model.as_str()
                } else {
                    config.model.as_str()
                };
                let eff_p = if !sess_provider.trim().is_empty() {
                    Some(sess_provider.as_str())
                } else {
                    Some(config.provider.as_str())
                };
                config.with_model_override(eff_m, eff_p, eff_override)
            };
            session_config.sandbox_root = project_path;
            session_config.knowledge_only = crate::config::is_knowledge_only_project(&project_id);
            if session_config.knowledge_only {
                session_config.permissions.mode = crate::config::PermissionMode::ReadOnly;
            }
            state
                .update_session_model(
                    &session_id,
                    if use_mock {
                        "mock".to_string()
                    } else {
                        session_config.model.clone()
                    },
                )
                .await;

            let remote_primary = config.servers.iter().find(|srv| {
                srv.enabled
                    && srv.id.eq_ignore_ascii_case(&session_server_id)
                    && (srv.url.starts_with("http://") || srv.url.starts_with("https://"))
                    && !srv.id.eq_ignore_ascii_case(&config.server_identity)
            }).cloned();

            let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(100);
            let runner_handle = if let Some(remote_srv) = remote_primary {
                let run_id = format!("run_{}_{}", session_id, chrono::Utc::now().timestamp_millis());
                {
                    let mut active_runs = state.active_remote_runs.write().await;
                    active_runs.insert(session_id.clone(), (remote_srv.clone(), run_id.clone()));
                }
                let sess_id_c = session_id.clone();
                let task_c = current_task.clone();
                let prior_c = current_prior.clone();
                let att_c = current_attachments.clone();
                let cfg_c = session_config.clone();
                let cancel_c = cancel_notify.clone();
                let state_c = state.clone();
                tokio::spawn(async move {
                    let res = crate::remote::execute_remote_server_run(
                        &remote_srv,
                        run_id,
                        sess_id_c.clone(),
                        task_c,
                        prior_c,
                        att_c,
                        cfg_c,
                        None,
                        None,
                        None,
                        0,
                        false,
                        use_mock,
                        cancel_c,
                        Some(state_c.clone()),
                        event_tx,
                    )
                    .await;
                    let mut active_runs = state_c.active_remote_runs.write().await;
                    active_runs.remove(&sess_id_c);
                    res
                })
            } else {
                let registry = state.get_registry_snapshot().await;
                let model: Arc<dyn ModelAdapter> = if use_mock {
                    Arc::new(MockClient)
                } else if is_google_search_model(&session_config.model) {
                    Arc::new(GoogleSearchClient::new())
                } else {
                    Arc::new(OpenAiClient::new(session_config.clone()))
                };

                let runner = AgentLoopRunner::new(
                    session_id.clone(),
                    current_task,
                    session_config,
                    registry,
                    model,
                    Some(state.clone()),
                )
                .with_cancel_notify(cancel_notify.clone())
                .with_prior_messages(current_prior)
                .with_task_attachments(current_attachments)
                .with_steer_inbox(current_steer);

                tokio::spawn(async move { runner.run_with_messages(event_tx).await })
            };

            // Forward internal events to state history and broadcast SSE listeners while this turn owns cancel_notify
            while let Some(evt) = event_rx.recv().await {
                let still_active = {
                    let lock = state.sessions.read().await;
                    lock.get(&session_id)
                        .is_some_and(|s| Arc::ptr_eq(&s.cancel_notify, &cancel_notify))
                };
                if !still_active {
                    continue;
                }
                state.append_session_event(&session_id, evt.clone()).await;
                let _ = bcast_tx.send(evt);
            }

            let run_res = runner_handle.await;

            // If undo_to_turn or a newer turn replaced cancel_notify, do not overwrite rolled-back state
            let (still_active, was_stopped) = {
                let lock = state.sessions.read().await;
                match lock.get(&session_id) {
                    Some(s) => (Arc::ptr_eq(&s.cancel_notify, &cancel_notify), s.status == "stopped"),
                    None => (false, false),
                }
            };
            if !still_active {
                break;
            }

            match run_res {
                Ok(Ok((_, updated_msgs))) => {
                    state.save_llm_messages(&session_id, updated_msgs).await;
                    if was_stopped {
                        break;
                    }

                    if let Some((queued, remaining)) = state.pop_queued_prompt(&session_id).await {
                        if let Some((next_task, next_attachments, next_prior, next_steer)) =
                            prepare_queued_turn(&state, &session_id, &queued, &remaining).await
                        {
                            current_task = next_task;
                            current_attachments = next_attachments;
                            current_prior = next_prior;
                            current_steer = next_steer;
                            continue;
                        }
                        break;
                    }

                    state.update_session_status(&session_id, "completed").await;

                    // Catch any prompt enqueued in the tiny window between pop_queued_prompt and update_session_status
                    if let Some((queued, remaining)) = state.pop_queued_prompt(&session_id).await {
                        if let Some((next_task, next_attachments, next_prior, next_steer)) =
                            prepare_queued_turn(&state, &session_id, &queued, &remaining).await
                        {
                            current_task = next_task;
                            current_attachments = next_attachments;
                            current_prior = next_prior;
                            current_steer = next_steer;
                            continue;
                        }
                    }
                    break;
                }
                Ok(Err(_)) => {
                    if !was_stopped {
                        state.update_session_status(&session_id, "failed").await;
                    }
                    break;
                }
                Err(_) => {
                    if !was_stopped {
                        state.update_session_status(&session_id, "panic").await;
                    }
                    break;
                }
            }
        }
    });
}

pub async fn create_session(
    State(state): State<AppState>,
    Json(payload): Json<CreateTaskRequest>,
) -> Json<CreateTaskResponse> {
    let config = { state.config.read().await.clone() };
    let project_id = payload.project_id.unwrap_or_else(|| config.active_project_id.clone());
    let sandbox = config
        .projects
        .iter()
        .find(|p| p.id == project_id)
        .map(|p| p.path.clone())
        .unwrap_or_else(|| config.sandbox_root.clone());

    let (prompt, slash_cmd, slash_args) = resolve_slash_input(
        &config,
        &sandbox,
        &payload.task,
        payload.slash_command.as_deref(),
        payload.slash_args.as_deref(),
    );
    let session_title = format_session_title_with_attachments(
        &prompt,
        &payload.attachments,
        slash_cmd.as_deref(),
        slash_args.as_deref(),
    );
    let mut runner_task = format_runner_task(&prompt, slash_cmd.as_deref(), slash_args.as_deref());
    if runner_task.trim().is_empty() && !payload.attachments.is_empty() {
        runner_task = "Analyze the attached file(s).".to_string();
    }

    let session_id = format!("ses_{}", chrono::Utc::now().timestamp_millis());
    let (bcast_tx, cancel_notify) = state
        .create_session_with_options(
            session_id.clone(),
            session_title,
            project_id.clone(),
            payload.server_id.clone(),
            payload.orchestrator_mode,
            // An empty model string means "use the project default", not an override.
            payload
                .model
                .clone()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()),
            "running",
        )
        .await;
    if payload
        .provider_id
        .as_deref()
        .map(|p| !p.trim().is_empty())
        .unwrap_or(false)
        || payload
            .reasoning_effort
            .as_deref()
            .map(|e| !e.trim().is_empty())
            .unwrap_or(false)
    {
        state
            .update_session_meta(
                &session_id,
                None,
                None,
                None,
                None,
                None,
                payload.provider_id.clone(),
                payload.reasoning_effort.clone(),
            )
            .await;
    }

    let (_, prior_messages, steer_inbox) = state
        .begin_turn_with_command_and_attachments(
            &session_id,
            &prompt,
            payload.attachments.clone(),
            slash_cmd.as_deref(),
            slash_args.as_deref(),
        )
        .await
        .expect("newly created session must exist");

    let use_mock = payload.mock.unwrap_or(config.mock_mode);
    spawn_session_turn(
        state,
        session_id.clone(),
        runner_task,
        payload.attachments,
        project_id,
        prior_messages,
        steer_inbox,
        bcast_tx,
        cancel_notify,
        use_mock,
    );

    Json(CreateTaskResponse { session_id })
}

pub async fn stream_session_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let lock = state.sessions.read().await;
    let session = lock.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    let rx = session.event_tx.subscribe();

    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json_data = serde_json::to_string(&event).unwrap_or_default();
                    return Some((Ok(Event::default().data(json_data)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

#[derive(Deserialize)]
pub struct PermissionDecisionRequest {
    pub approved: bool,
}

pub async fn decide_permission(
    State(state): State<AppState>,
    Path(perm_id): Path<String>,
    Json(payload): Json<PermissionDecisionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.resolve_permission(&perm_id, payload.approved).await {
        return Ok(Json(serde_json::json!({ "status": "resolved", "approved": payload.approved })));
    }
    let remote_server_id = {
        let mut routes = state.remote_perm_routes.write().await;
        routes.remove(&perm_id)
    };
    if let Some(srv_id) = remote_server_id {
        let srv_opt = {
            let conf = state.config.read().await;
            conf.servers.iter().find(|s| s.id == srv_id).cloned()
        };
        if let Some(srv) = srv_opt {
            if crate::remote::forward_remote_permission_decision(&srv, &perm_id, payload.approved)
                .await
                .is_ok()
            {
                return Ok(Json(serde_json::json!({ "status": "forwarded", "server_id": srv_id, "approved": payload.approved })));
            }
        }
    }
    Err(StatusCode::NOT_FOUND)
}

static METRICS_SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();

#[derive(Serialize)]
pub struct MetricsResponse {
    pub process_rss_mb: f64,
    pub process_cpu_pct: f32,
    pub host_total_ram_mb: f64,
    pub host_used_ram_mb: f64,
    pub cpu_cores: usize,
    pub active_sessions_count: usize,
    pub active_subagents: usize,
    pub os_name: String,
}

pub async fn get_metrics(State(state): State<AppState>) -> Json<MetricsResponse> {
    let pid = Pid::from_u32(std::process::id());
    let (process_rss_mb, process_cpu_pct, host_total_ram_mb, host_used_ram_mb, cpu_cores) = {
        let sys_mutex = METRICS_SYSTEM.get_or_init(|| {
            let mut s = System::new();
            s.refresh_cpu();
            s.refresh_memory();
            s.refresh_process(pid);
            Mutex::new(s)
        });
        let mut sys = sys_mutex.lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_memory();
        sys.refresh_process(pid);

        let proc_info = sys.process(pid);
        let rss = proc_info
            .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
            .unwrap_or(0.0);
        let cpu = proc_info.map(|p| p.cpu_usage()).unwrap_or(0.0);
        let total_ram = sys.total_memory() as f64 / 1024.0 / 1024.0;
        let used_ram = sys.used_memory() as f64 / 1024.0 / 1024.0;
        let cores = sys.cpus().len();
        (rss, cpu, total_ram, used_ram, cores)
    };

    let active_sessions_count = {
        let lock = state.sessions.read().await;
        lock.values().filter(|s| s.status == "running").count()
    };
    let active_subagents = state.total_active_subagents().await;

    let os_name = System::name().unwrap_or_else(|| "Unknown".into());

    Json(MetricsResponse {
        process_rss_mb,
        process_cpu_pct,
        host_total_ram_mb,
        host_used_ram_mb,
        cpu_cores,
        active_sessions_count,
        active_subagents,
        os_name,
    })
}

pub async fn get_config(State(state): State<AppState>) -> Json<HarnessConfig> {
    let mut conf = state.config.read().await.clone();
    conf.memory.max_context_tokens = effective_max_context_tokens(
        &conf.model,
        conf.memory.max_context_tokens,
        conf.memory.auto_detect_context,
        None,
    );
    // Never return stored credentials; clients send `SECRET_REDACTED` back unchanged.
    conf.redact_secrets();
    Json(conf)
}

/// Merge a client-supplied full-config POST into the server-owned configuration.
///
/// `POST /api/config` submits the *entire* `HarnessConfig` as a stale snapshot, so a naive
/// wholesale replacement silently wipes projects added through `/api/projects` and reverts
/// `active_project_id` / `sandbox_root` (orphaning every session that points at them).
/// Project ownership is server-side, so those three fields are always taken from `existing`
/// while every other setting is taken from the incoming payload.
pub fn merge_config_update(existing: &HarnessConfig, incoming: HarnessConfig) -> HarnessConfig {
    let mut merged = incoming;
    merged.projects = existing.projects.clone();
    merged.active_project_id = existing.active_project_id.clone();
    merged.sandbox_root = existing.sandbox_root.clone();
    // Onboarding is a server-owned one-way flag: a stale or minimal client payload
    // must never re-trigger the first-run wizard.
    merged.onboarded = existing.onboarded;
    // The auto-updater writes this marker; a stale client snapshot must not
    // clear it and restart an install loop.
    merged.last_auto_update_target = existing.last_auto_update_target.clone();
    merged
}

/// Sandbox root that must be in effect after saving project `saved_id` with `new_path`.
///
/// Returns `Some(new_path)` when the save changes the effective sandbox: either because the
/// project was explicitly activated, or because it is the already-active project and its
/// workspace path changed. Returns `None` when the sandbox must be left untouched.
pub fn effective_sandbox_after_project_save(
    active_project_id: &str,
    saved_id: &str,
    set_active: bool,
    previous_active_path: Option<&std::path::Path>,
    new_path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if set_active {
        return Some(new_path.to_path_buf());
    }
    if active_project_id != saved_id {
        return None;
    }
    let changed = match previous_active_path {
        Some(prev) => prev != new_path,
        None => true,
    };
    if changed {
        Some(new_path.to_path_buf())
    } else {
        None
    }
}

/// Explicit fallback project to activate when the active project disappears:
/// the first remaining project (configs are kept non-empty by the last-project guard).
pub fn fallback_project_id(projects: &[crate::config::Project]) -> Option<String> {
    projects.first().map(|p| p.id.clone())
}

/// Re-derive every project's `is_active` flag so exactly the active project is flagged and
/// `active_project_id` can never point at a project reporting `is_active: false`.
pub fn reconcile_active_project_flags(
    projects: &mut [crate::config::Project],
    active_id: &str,
) {
    for p in projects.iter_mut() {
        p.is_active = p.id == active_id;
    }
}

/// Validate an explicit project id supplied by the client.
///
/// Empty/whitespace ids are rejected so callers fall back to the derived slug (handled by the
/// caller via `Option`), and the reserved conversations pseudo-project is refused outright.
pub fn validate_explicit_project_id(id: &str) -> Result<(), (StatusCode, String)> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Project id cannot be empty.".to_string(),
        ));
    }
    if trimmed == crate::config::CONVERSATIONS_PROJECT_ID {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is a reserved project id.", trimmed),
        ));
    }
    Ok(())
}

pub async fn update_config(
    State(state): State<AppState>,
    Json(mut new_conf): Json<HarnessConfig>,
) -> Json<HarnessConfig> {
    new_conf.display_verbosity =
        crate::config::normalize_display_verbosity(&new_conf.display_verbosity);
    new_conf.send_mode = crate::config::normalize_send_mode(&new_conf.send_mode);
    new_conf.memory.max_tool_retries = new_conf.memory.max_tool_retries.min(10);
    // Clamp so truncation cannot be disabled into a no-op that grows payloads.
    new_conf.max_tool_output_bytes = new_conf.max_tool_output_bytes.clamp(1024, 1024 * 1024);
    new_conf.memory.max_context_tokens = effective_max_context_tokens(
        &new_conf.model,
        new_conf.memory.max_context_tokens,
        new_conf.memory.auto_detect_context,
        None,
    );
    // Never wholesale-replace: project state is owned by the server (see merge_config_update).
    let merged = {
        let mut conf = state.config.write().await;
        let existing = conf.clone();
        // Restore any secrets the client echoed back as redacted placeholders.
        new_conf.restore_secrets(&existing);
        let merged = merge_config_update(&existing, new_conf);
        *conf = merged.clone();
        merged
    };
    state.reload_plugins().await;
    state.persist_config().await;
    let mut out = merged;
    out.redact_secrets();
    Json(out)
}

#[cfg(test)]
mod handlers_config_merge_tests {
    use super::*;

    fn base() -> crate::config::HarnessConfig {
        crate::config::HarnessConfig::default()
    }

    #[test]
    fn merge_preserves_server_owned_projects_from_stale_snapshot() {
        let mut existing = base();
        existing.projects = vec![crate::config::Project {
            id: "foo".into(),
            name: "Foo".into(),
            path: std::path::PathBuf::from("C:\\tmp\\foo"),
            icon: "📁".into(),
            created_at: "now".into(),
            is_active: true,
            defaults: Default::default(),
            extra_paths: Vec::new(),
        }];
        existing.active_project_id = "foo".into();
        existing.sandbox_root = std::path::PathBuf::from("C:\\tmp\\foo");

        // A stale client snapshot with no projects must not wipe the new project.
        let incoming = base();
        let merged = merge_config_update(&existing, incoming);
        assert_eq!(merged.projects.len(), 1);
        assert_eq!(merged.projects[0].id, "foo");
        assert_eq!(merged.active_project_id, "foo");
        assert_eq!(merged.sandbox_root, std::path::PathBuf::from("C:\\tmp\\foo"));
    }

    #[test]
    fn merge_keeps_incoming_non_project_settings() {
        let existing = base();
        let mut incoming = base();
        incoming.display_verbosity = "verbose".into();
        let merged = merge_config_update(&existing, incoming);
        assert_eq!(merged.display_verbosity, "verbose");
    }

    #[test]
    fn merge_never_clears_the_onboarded_flag() {
        let mut existing = base();
        existing.onboarded = true;
        // A client that omits the flag deserializes it as the default `false`.
        let incoming = base();
        assert!(!incoming.onboarded);
        let merged = merge_config_update(&existing, incoming);
        assert!(merged.onboarded, "onboarded must survive a settings save");
    }

    #[test]
    fn merge_keeps_the_auto_update_install_marker() {
        let mut existing = base();
        existing.last_auto_update_target = "1.1.1".into();
        let mut incoming = base();
        incoming.last_auto_update_target = String::new();
        let merged = merge_config_update(&existing, incoming);
        assert_eq!(
            merged.last_auto_update_target, "1.1.1",
            "a stale snapshot must not clear the updater's install marker"
        );
    }

    #[test]
    fn effective_sandbox_helper_encodes_project_rules() {
        let new_path = std::path::Path::new("C:\\tmp\\bar");
        let prev = std::path::Path::new("C:\\tmp\\foo");
        // explicit activation always switches the sandbox
        assert_eq!(
            effective_sandbox_after_project_save("active", "other", true, None, new_path).as_deref(),
            Some(new_path)
        );
        // editing the active project with a changed path switches the sandbox
        assert!(effective_sandbox_after_project_save("foo", "foo", false, Some(prev), new_path).is_some());
        // editing a non-active project leaves the sandbox untouched
        assert!(effective_sandbox_after_project_save("foo", "other", false, Some(prev), new_path).is_none());
        // editing the active project without a path change leaves it untouched
        assert!(effective_sandbox_after_project_save("foo", "foo", false, Some(prev), prev).is_none());
    }
}

#[cfg(test)]
mod ui_structure_tests {
    use super::INDEX_HTML;

    /// Strip HTML comments so markup quoted inside them is not counted.
    fn strip_comments(html: &str) -> String {
        let mut out = String::with_capacity(html.len());
        let mut rest = html;
        while let Some(start) = rest.find("<!--") {
            out.push_str(&rest[..start]);
            match rest[start + 4..].find("-->") {
                Some(end) => rest = &rest[start + 4 + end + 3..],
                None => return out,
            }
        }
        out.push_str(rest);
        out
    }

    /// Track `<div>` nesting and report the depth of each requested modal id.
    fn modal_depths(html: &str, ids: &[&str]) -> (Vec<(String, i32)>, i32, bool) {
        let lower = strip_comments(html).to_ascii_lowercase();
        let bytes = lower.as_bytes();
        let mut depth = 0i32;
        let mut went_negative = false;
        let mut found: Vec<(String, i32)> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'<' {
                i += 1;
                continue;
            }
            let is_open = lower[i..].starts_with("<div")
                && bytes
                    .get(i + 4)
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>');
            let is_close = lower[i..].starts_with("</div");
            if is_open || is_close {
                let end = lower[i..]
                    .find('>')
                    .map(|e| i + e + 1)
                    .unwrap_or(lower.len());
                if is_open {
                    depth += 1;
                    let tag = &lower[i..end];
                    for id in ids {
                        let needle = format!("id=\"{id}\"");
                        if tag.contains(&needle) && !found.iter().any(|(k, _)| k.as_str() == *id) {
                            found.push((id.to_string(), depth));
                        }
                    }
                } else {
                    depth -= 1;
                    if depth < 0 {
                        went_negative = true;
                    }
                }
                i = end;
                continue;
            }
            i += 1;
        }
        (found, depth, went_negative)
    }

    /// Regression: several modals were nested inside the hidden project modal,
    /// so opening them produced an invisible (zero-size) element.
    #[test]
    fn modals_are_top_level_so_they_can_open() {
        let ids = [
            "config-modal",
            "project-modal",
            "project-settings-modal",
            "file-browser-modal",
            "command-palette-modal",
            "onboarding-modal",
        ];
        let (found, final_depth, went_negative) = modal_depths(INDEX_HTML, &ids);
        assert!(!went_negative, "index.html closes more <div>s than it opens");
        assert_eq!(final_depth, 0, "index.html has unclosed <div> elements");
        for id in ids {
            let depth = found.iter().find(|(k, _)| k == id).map(|(_, d)| *d);
            assert_eq!(
                depth,
                Some(1),
                "#{id} must be a direct child of <body>; got {depth:?}. A parent modal hides it."
            );
        }
    }
}

#[derive(Deserialize)]
pub struct SearchTestRequest {
    pub query: String,
}

#[derive(Serialize)]
pub struct SearchTestResponse {
    pub success: bool,
    pub provider: String,
    pub query: String,
    pub output: String,
}

pub async fn test_search(
    State(state): State<AppState>,
    Json(payload): Json<SearchTestRequest>,
) -> Json<SearchTestResponse> {
    let conf = state.config.read().await.clone();
    let res = crate::plugins::web_search::execute_search(&conf.search, &payload.query, 5).await;
    match res {
        Ok(out) => Json(SearchTestResponse {
            success: true,
            provider: conf.search.active_provider.clone(),
            query: payload.query,
            output: out,
        }),
        Err(err) => Json(SearchTestResponse {
            success: false,
            provider: conf.search.active_provider.clone(),
            query: payload.query,
            output: format!("Search error: {}", err),
        }),
    }
}

pub async fn list_mcp_servers(State(state): State<AppState>) -> Json<Vec<crate::config::McpServerConfig>> {
    let conf = state.config.read().await;
    // Never return stored MCP credentials; clients send `SECRET_REDACTED` back unchanged.
    let mut servers = conf.mcp_servers.clone();
    for s in servers.iter_mut() {
        s.redact_secrets();
    }
    Json(servers)
}

pub async fn save_mcp_server(
    State(state): State<AppState>,
    Json(mut server): Json<crate::config::McpServerConfig>,
) -> Json<crate::config::McpServerConfig> {
    {
        let mut conf = state.config.write().await;
        // Preserve stored env secrets echoed back as redacted placeholders.
        if let Some(existing) = conf.mcp_servers.iter().find(|s| s.id == server.id) {
            server.restore_secrets(existing);
        }
        if let Some(pos) = conf.mcp_servers.iter().position(|s| s.id == server.id) {
            conf.mcp_servers[pos] = server.clone();
        } else {
            conf.mcp_servers.push(server.clone());
        }
    }
    state.reload_plugins().await;
    state.persist_config().await;
    let mut out = server;
    out.redact_secrets();
    Json(out)
}

pub async fn delete_mcp_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    {
        let mut conf = state.config.write().await;
        let orig_len = conf.mcp_servers.len();
        conf.mcp_servers.retain(|s| s.id != id);
        if conf.mcp_servers.len() == orig_len {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    state.reload_plugins().await;
    state.persist_config().await;
    Ok(Json(serde_json::json!({ "status": "deleted", "id": id })))
}

pub async fn test_mcp_server_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::plugins::mcp::McpTestResult>, StatusCode> {
    let server_cfg = {
        let conf = state.config.read().await;
        conf.mcp_servers.iter().find(|s| s.id == id).cloned()
    };

    match server_cfg {
        Some(cfg) => {
            let res = crate::plugins::mcp::test_mcp_server(&cfg).await;
            let mut reg = state.registry.write().await;
            if res.success && cfg.enabled {
                reg.register_mcp_test_result(&cfg, &res);
            } else if !cfg.enabled {
                reg.unregister_mcp_server_tools(&cfg.id);
            }
            Ok(Json(res))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Deserialize)]
pub struct ActivateProviderRequest {
    pub provider_id: String,
    pub model: Option<String>,
}

pub async fn activate_provider(
    State(state): State<AppState>,
    Json(payload): Json<ActivateProviderRequest>,
) -> Result<Json<HarnessConfig>, StatusCode> {
    let mut conf = state.config.write().await;
    let mut selected_info: Option<(String, String, String, String)> = None;

    for p in &mut conf.custom_providers {
        if p.id == payload.provider_id {
            p.is_active = true;
            if let Some(ref m) = payload.model {
                p.default_model = m.clone();
            }
            selected_info = Some((p.id.clone(), p.api_base.clone(), p.api_key.clone(), p.default_model.clone()));
        } else {
            p.is_active = false;
        }
    }

    if let Some((id, base, key, model)) = selected_info {
        conf.provider = id;
        conf.api_base = base;
        conf.api_key = key;
        conf.model = model;
        conf.memory.max_context_tokens = effective_max_context_tokens(
            &conf.model,
            conf.memory.max_context_tokens,
            conf.memory.auto_detect_context,
            None,
        );
        let cloned = conf.clone();
        drop(conf);
        state.persist_config().await;
        let mut out = cloned;
        out.redact_secrets();
        Ok(Json(out))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

#[derive(Serialize)]
pub struct ToolContractApiItem {
    pub name: String,
    pub description: String,
    pub category: String,
    pub permission_level: ToolPermission,
    pub effective_policy: PermissionLevel,
    pub fields: Vec<ToolFieldSpec>,
    pub expected_output: OutputContract,
    pub failure_modes: Vec<ToolFailureMode>,
    pub timeout_secs: u64,
    pub is_custom: bool,
    pub schema: serde_json::Value,
}

fn build_tool_contract_item(
    contract: ToolContract,
    permissions: &PermissionsConfig,
    schema: serde_json::Value,
) -> ToolContractApiItem {
    let effective_policy =
        permissions.policy_for_contract(&contract.name, &contract.permission_level);
    ToolContractApiItem {
        name: contract.name,
        description: contract.description,
        category: contract.category,
        permission_level: contract.permission_level,
        effective_policy,
        fields: contract.fields,
        expected_output: contract.expected_output,
        failure_modes: contract.failure_modes,
        timeout_secs: contract.timeout_secs,
        is_custom: contract.is_custom,
        schema,
    }
}

fn extract_schemas_by_name(reg: &PluginRegistry) -> HashMap<String, serde_json::Value> {
    let mut map = HashMap::new();
    if let Some(arr) = reg.get_tools_schema().as_array() {
        for entry in arr {
            if let Some(func) = entry.get("function") {
                if let (Some(name), Some(params)) = (
                    func.get("name").and_then(|v| v.as_str()),
                    func.get("parameters"),
                ) {
                    map.insert(name.to_string(), params.clone());
                }
            }
        }
    }
    map
}

pub async fn list_tools(State(state): State<AppState>) -> Json<Vec<ToolContractApiItem>> {
    let reg = state.get_registry_snapshot().await;
    let permissions = state.config.read().await.permissions.clone();
    let mut schemas = extract_schemas_by_name(&reg);
    let mut items: Vec<ToolContractApiItem> = reg
        .get_contracts()
        .into_iter()
        .map(|contract| {
            let schema = schemas
                .remove(&contract.name)
                .unwrap_or_else(|| contract.to_json_schema());
            build_tool_contract_item(contract, &permissions, schema)
        })
        .collect();
    items.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    Json(items)
}

pub async fn create_custom_tool(
    State(state): State<AppState>,
    Query(query): Query<SessionScopedQuery>,
    Json(payload): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<ToolContractApiItem>), (StatusCode, String)> {
    let spec = CustomToolSpec::from_json_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let tool_name = spec.contract.name.clone();

    let is_builtin = is_builtin_tool_name(&tool_name)
        || state.registry.read().await.is_builtin(&tool_name);
    if is_builtin {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Cannot overwrite built-in tool '{}'.", tool_name),
        ));
    }

    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    save_custom_tool(&sandbox, &spec)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .register_custom_tool(spec.clone())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let permissions = state.config.read().await.permissions.clone();
    let contract = spec.contract;
    let schema = contract.to_json_schema();
    Ok((
        StatusCode::CREATED,
        Json(build_tool_contract_item(contract, &permissions, schema)),
    ))
}

pub async fn delete_custom_tool_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<SessionScopedQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let trimmed = name.trim().to_string();
    validate_tool_name(&trimmed).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let is_builtin = is_builtin_tool_name(&trimmed)
        || state.registry.read().await.is_builtin(&trimmed);
    if is_builtin {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Cannot delete built-in tool '{}'.", trimmed),
        ));
    }

    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let path = custom_tools_dir(&sandbox).join(format!("{}.json", trimmed));
    let file_existed = path.exists();
    if file_existed {
        delete_custom_tool(&sandbox, &trimmed)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }

    let unregistered = state
        .unregister_custom_tool(&trimmed)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    if !file_existed && !unregistered {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Custom tool '{}' not found.", trimmed),
        ));
    }

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": trimmed
    })))
}

pub async fn validate_tool_args(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let trimmed = name.trim();
    let reg = state.get_registry_snapshot().await;
    let contract = reg.get_contract(trimmed).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Tool '{}' not found.", trimmed),
        )
    })?;

    let args = match payload.as_object() {
        Some(obj)
            if obj.len() == 1
                && obj.contains_key("args")
                && !contract.fields.iter().any(|f| f.name == "args") =>
        {
            obj.get("args").cloned().unwrap_or(payload)
        }
        _ => payload,
    };

    match contract.validate_args(&args) {
        Ok(()) => Ok(Json(serde_json::json!({ "valid": true }))),
        Err(err) => Ok(Json(serde_json::json!({ "valid": false, "error": err }))),
    }
}

#[derive(Serialize)]
pub struct RemoteModelItem {
    pub id: String,
    pub owned_by: Option<String>,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub provider_name: String,
    pub reasoning: ModelReasoningSpec,
    pub modalities: ModelModalityCaps,
    pub max_context_tokens: usize,
    #[serde(default)]
    pub experimental: bool,
}

pub fn built_in_experimental_models() -> Vec<RemoteModelItem> {
    vec![RemoteModelItem {
        id: EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID.to_string(),
        owned_by: Some("google-search-ai".into()),
        provider_id: "builtin".to_string(),
        provider_name: "Built-in Experimental".to_string(),
        reasoning: infer_model_reasoning_spec(EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID, None),
        modalities: infer_model_modalities(EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID, None),
        max_context_tokens: infer_model_context_window(EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID, None),
        experimental: true,
    }]
}

#[derive(Deserialize, Default)]
pub struct ModelReasoningQuery {
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct ModelReasoningResponse {
    #[serde(flatten)]
    pub reasoning: ModelReasoningSpec,
    pub modalities: ModelModalityCaps,
    pub max_context_tokens: usize,
    pub experimental: bool,
}

pub async fn get_model_reasoning(
    State(state): State<AppState>,
    Query(params): Query<ModelReasoningQuery>,
) -> Json<ModelReasoningResponse> {
    let model_id = match params.model.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(m) => m,
        None => state.config.read().await.model.clone(),
    };
    let caps = resolve_model_capabilities(&model_id, None);
    let experimental = is_google_search_model(&model_id);
    Json(ModelReasoningResponse {
        reasoning: caps.reasoning,
        modalities: caps.modalities,
        max_context_tokens: caps.max_context_tokens,
        experimental,
    })
}

/// Inspect inferred (and optionally upstream-queried) capabilities for a specific model ID.
pub async fn get_model_capabilities(
    State(state): State<AppState>,
    Query(params): Query<ModelReasoningQuery>,
) -> Json<ModelCapabilities> {
    let conf = state.config.read().await.clone();
    let model_id = match params.model.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(m) => m,
        None => conf.model.clone(),
    };

    let mut api_meta: Option<serde_json::Value> = None;
    let base = conf.api_base.trim().trim_end_matches('/');
    if !base.is_empty() && !is_google_search_model(&model_id) {
        if let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(6)).build() {
            let url = if base.ends_with("/models") {
                base.to_string()
            } else {
                format!("{}/models", base)
            };
            let mut req = client.get(&url);
            if !conf.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", conf.api_key));
            }
            if let Ok(resp) = req.send().await {
                if resp.status().is_success() {
                    if let Ok(val) = resp.json::<serde_json::Value>().await {
                        if let Some(arr) = val.get("data").and_then(|d| d.as_array()) {
                            api_meta = arr
                                .iter()
                                .find(|item| {
                                    item.get("id")
                                        .and_then(|v| v.as_str())
                                        .is_some_and(|id| id.eq_ignore_ascii_case(&model_id))
                                })
                                .cloned();
                        }
                    }
                }
            }
        }
    }

    Json(resolve_model_capabilities(&model_id, api_meta.as_ref()))
}

#[derive(Deserialize, Default)]
pub struct ListModelsQuery {
    pub api_base: Option<String>,
    /// Deprecated: keys in query strings leak via logs/history. Prefer POST body.
    pub api_key: Option<String>,
}

/// Body for `POST /api/models/remote` so keys never travel in a URL.
#[derive(Deserialize, Default)]
pub struct ListModelsBody {
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

async fn fetch_models_for_provider(
    provider_id: String,
    provider_name: String,
    api_base: String,
    api_key: String,
) -> Result<Vec<RemoteModelItem>, (StatusCode, String)> {
    let base = api_base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let url = if base.ends_with("/models") {
        base.to_string()
    } else {
        format!("{}/models", base)
    };

    let mut req = client.get(&url);
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Failed to reach {}: {}", url, e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Cap the body read and redact: upstream error bodies can embed keys/HTML.
        let body = resp.text().await.unwrap_or_default();
        let body = body.chars().take(2048).collect::<String>();
        // Map upstream auth failures to 502: returning the provider's 401 would be
        // indistinguishable from the harness's own auth failure, which made the UI
        // prompt for a harness token whenever a provider key was rejected.
        let out_status = match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                StatusCode::BAD_GATEWAY
            }
            _ => StatusCode::BAD_GATEWAY,
        };
        return Err((
            out_status,
            format!(
                "{} rejected the request: {}",
                provider_name,
                crate::plugins::web_search::redact_provider_error(status, &body)
            ),
        ));
    }

    let val = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Failed to parse {} response: {}", provider_name, e)))?;

    let arr = val.get("data").and_then(|d| d.as_array()).ok_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            format!("{} returned no model list (unexpected response shape)", provider_name),
        )
    })?;

    let mut list = Vec::new();
    for item in arr {
        if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
            let owned_by = item.get("owned_by").and_then(|o| o.as_str()).map(|s| s.to_string());
            let caps = resolve_model_capabilities(id, Some(item));
            list.push(RemoteModelItem {
                id: id.to_string(),
                owned_by,
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
                reasoning: caps.reasoning,
                modalities: caps.modalities,
                max_context_tokens: caps.max_context_tokens,
                experimental: is_google_search_model(id),
            });
        }
    }
    Ok(list)
}

pub async fn list_remote_models(
    State(state): State<AppState>,
    Query(params): Query<ListModelsQuery>,
) -> Result<Json<Vec<RemoteModelItem>>, (StatusCode, String)> {
    resolve_models(state, params.api_base, params.api_key).await
}

/// `POST /api/models/remote` — same as GET but accepts `{api_base, api_key}` in
/// the body so credentials never appear in URLs, logs, or browser history.
pub async fn list_remote_models_post(
    State(state): State<AppState>,
    Json(body): Json<ListModelsBody>,
) -> Result<Json<Vec<RemoteModelItem>>, (StatusCode, String)> {
    resolve_models(state, body.api_base, body.api_key).await
}

async fn resolve_models(
    state: AppState,
    api_base: Option<String>,
    api_key: Option<String>,
) -> Result<Json<Vec<RemoteModelItem>>, (StatusCode, String)> {
    let conf = state.config.read().await.clone();
    let api_base = api_base
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| conf.api_base.clone());
    let base_norm = api_base.trim().trim_end_matches('/').to_string();

    // Match the requested base to a configured provider. Its key is the only key
    // that may be used: falling back to the *active* provider's key silently sent
    // one provider's credential to another provider's host (and caused 401s).
    let matched = conf
        .custom_providers
        .iter()
        .find(|p| p.api_base.trim_end_matches('/') == base_norm);

    let (pid, pname, stored_key) = match matched {
        Some(p) => (p.id.clone(), p.name.clone(), p.api_key.clone()),
        None if base_norm == conf.api_base.trim().trim_end_matches('/') => (
            conf.provider.clone(),
            if conf.provider.is_empty() { "provider".to_string() } else { conf.provider.clone() },
            conf.api_key.clone(),
        ),
        None => (
            String::new(),
            base_norm
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("provider")
                .to_string(),
            String::new(),
        ),
    };

    // A redacted placeholder (or empty value) means "use the stored key for THIS
    // provider"; never borrow a different provider's credential.
    let api_key = api_key
        .filter(|k| !k.is_empty() && k != crate::secret_redaction::SECRET_REDACTED)
        .unwrap_or(stored_key);

    if base_norm.is_empty() {
        return Ok(Json(built_in_experimental_models()));
    }

    let mut list = fetch_models_for_provider(pid, pname, api_base, api_key).await?;

    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(list))
}

#[derive(Serialize)]
pub struct ProviderModelsGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub is_active: bool,
    pub has_credentials: bool,
    pub error: Option<String>,
    pub models: Vec<RemoteModelItem>,
}

pub async fn list_all_provider_models(
    State(state): State<AppState>,
) -> Json<Vec<ProviderModelsGroup>> {
    let conf = state.config.read().await.clone();
    let mut futs = Vec::new();

    for p in conf.custom_providers.iter().filter(|p| p.enabled) {
        let has_cred = !p.api_key.trim().is_empty()
            || p.api_base.contains("localhost")
            || p.api_base.contains("127.0.0.1");
        let p_clone = p.clone();
        let is_act = p.is_active || p.id == conf.provider;
        futs.push(async move {
            if !has_cred || p_clone.api_base.trim().is_empty() {
                let mut fallback = Vec::new();
                if !p_clone.default_model.trim().is_empty() {
                    let caps = resolve_model_capabilities(&p_clone.default_model, None);
                    fallback.push(RemoteModelItem {
                        id: p_clone.default_model.clone(),
                        owned_by: Some(p_clone.id.clone()),
                        provider_id: p_clone.id.clone(),
                        provider_name: p_clone.name.clone(),
                        reasoning: caps.reasoning,
                        modalities: caps.modalities,
                        max_context_tokens: caps.max_context_tokens,
                        experimental: is_google_search_model(&p_clone.default_model),
                    });
                }
                return ProviderModelsGroup {
                    provider_id: p_clone.id,
                    provider_name: p_clone.name,
                    is_active: is_act,
                    has_credentials: false,
                    error: None,
                    models: fallback,
                };
            }

            match fetch_models_for_provider(
                p_clone.id.clone(),
                p_clone.name.clone(),
                p_clone.api_base.clone(),
                p_clone.api_key.clone(),
            )
            .await
            {
                Ok(mut models) => {
                    models.sort_by(|a, b| a.id.cmp(&b.id));
                    ProviderModelsGroup {
                        provider_id: p_clone.id,
                        provider_name: p_clone.name,
                        is_active: is_act,
                        has_credentials: true,
                        error: None,
                        models,
                    }
                }
                Err((_, err_msg)) => {
                    let mut fallback = Vec::new();
                    if !p_clone.default_model.trim().is_empty() {
                        let caps = resolve_model_capabilities(&p_clone.default_model, None);
                        fallback.push(RemoteModelItem {
                            id: p_clone.default_model.clone(),
                            owned_by: Some(p_clone.id.clone()),
                            provider_id: p_clone.id.clone(),
                            provider_name: p_clone.name.clone(),
                            reasoning: caps.reasoning,
                            modalities: caps.modalities,
                            max_context_tokens: caps.max_context_tokens,
                            experimental: is_google_search_model(&p_clone.default_model),
                        });
                    }
                    ProviderModelsGroup {
                        provider_id: p_clone.id,
                        provider_name: p_clone.name,
                        is_active: is_act,
                        has_credentials: true,
                        error: Some(err_msg),
                        models: fallback,
                    }
                }
            }
        });
    }

    let mut groups = futures_util::future::join_all(futs).await;
    groups.push(ProviderModelsGroup {
        provider_id: "builtin".to_string(),
        provider_name: "Built-in Experimental".to_string(),
        is_active: false,
        has_credentials: true,
        error: None,
        models: built_in_experimental_models(),
    });

    Json(groups)
}

#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub icon: String,
    pub is_active: bool,
    pub session_count: usize,
    pub defaults: crate::config::ProjectDefaults,
    /// Additional folders in this project (primary `path` excluded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_paths: Vec<String>,
}

pub async fn list_projects(State(state): State<AppState>) -> Json<Vec<ProjectSummary>> {
    let conf = state.config.read().await;
    let sessions = state.sessions.read().await;

    let mut list = Vec::new();
    for p in &conf.projects {
        let count = sessions.values().filter(|s| s.project_id == p.id).count();
        list.push(ProjectSummary {
            id: p.id.clone(),
            name: p.name.clone(),
            path: p.path.display().to_string(),
            icon: p.icon.clone(),
            is_active: p.id == conf.active_project_id,
            session_count: count,
            defaults: p.defaults.clone(),
            extra_paths: p
                .extra_paths
                .iter()
                .map(|f| f.display().to_string())
                .collect(),
        });
    }
    Json(list)
}

fn json_api_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": message.into() })))
}

fn normalize_display_path(p: &std::path::Path) -> String {
    let raw = p.display().to_string();
    let stripped = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw
    };
    // Ensure bare Windows drive letter like "C:" becomes "C:\"
    if stripped.len() == 2 && stripped.as_bytes()[1] == b':' && stripped.as_bytes()[0].is_ascii_alphabetic() {
        format!("{}\\", stripped)
    } else {
        stripped
    }
}

#[derive(Deserialize)]
pub struct SaveProjectRequest {
    pub id: Option<String>,
    pub name: String,
    pub path: String,
    pub icon: Option<String>,
    pub set_active: Option<bool>,
    #[serde(default)]
    pub defaults: Option<crate::config::ProjectDefaults>,
    /// Additional workspace folders to attach to this project.
    #[serde(default)]
    pub extra_paths: Vec<String>,
}

pub async fn save_project(
    State(state): State<AppState>,
    Json(payload): Json<SaveProjectRequest>,
) -> Result<Json<ProjectSummary>, (StatusCode, Json<serde_json::Value>)> {
    let clean_name = payload.name.trim().to_string();
    if clean_name.is_empty() {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Project name cannot be empty."));
    }
    let trimmed_path = payload.path.trim();
    if trimmed_path.is_empty() {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Project workspace path cannot be empty."));
    }

    // Normalize bare drive e.g. "D:" -> "D:\"
    let raw_path = if trimmed_path.len() == 2
        && trimmed_path.as_bytes()[1] == b':'
        && trimmed_path.as_bytes()[0].is_ascii_alphabetic()
    {
        std::path::PathBuf::from(format!("{}\\", trimmed_path))
    } else {
        std::path::PathBuf::from(trimmed_path)
    };

    let resolved_path = if raw_path.is_absolute() {
        raw_path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(raw_path)
    };

    if resolved_path.is_file() {
        return Err(json_api_error(
            StatusCode::BAD_REQUEST,
            format!("'{}' is a file, not a directory.", resolved_path.display()),
        ));
    }

    if let Err(e) = tokio::fs::create_dir_all(&resolved_path).await {
        return Err(json_api_error(
            StatusCode::BAD_REQUEST,
            format!("Failed to create project folder '{}': {}", resolved_path.display(), e),
        ));
    }

    let final_path_str = tokio::fs::canonicalize(&resolved_path)
        .await
        .map(|p| normalize_display_path(&p))
        .unwrap_or_else(|_| normalize_display_path(&resolved_path));
    let path_buf = std::path::PathBuf::from(&final_path_str);

    let mut conf = state.config.write().await;
    let id = match payload.id.filter(|s| !s.trim().is_empty()) {
        Some(explicit_id) => {
            // Reject reserved ids (e.g. `__conversations__`) and blank ids.
            validate_explicit_project_id(&explicit_id)
                .map_err(|(code, msg)| json_api_error(code, msg))?;
            explicit_id
        }
        None => {
            let base_slug: String = clean_name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .split('-')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            let seed = if base_slug.is_empty() {
                format!("proj_{}", chrono::Utc::now().timestamp_millis())
            } else {
                base_slug
            };
            let mut candidate = seed.clone();
            let mut suffix = 2usize;
            while conf.projects.iter().any(|p| p.id == candidate)
                || candidate == crate::config::CONVERSATIONS_PROJECT_ID
            {
                candidate = format!("{}-{}", seed, suffix);
                suffix += 1;
            }
            candidate
        }
    };

    let icon = payload.icon.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "📁".to_string());
    let set_active = payload.set_active.unwrap_or(true);

    // Resolve and validate additional folders (created if missing).
    let mut extra_paths: Vec<std::path::PathBuf> = Vec::new();
    for raw in &payload.extra_paths {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = {
            let p = if trimmed.len() == 2
                && trimmed.as_bytes()[1] == b':'
                && trimmed.as_bytes()[0].is_ascii_alphabetic()
            {
                std::path::PathBuf::from(format!("{}\\", trimmed))
            } else {
                std::path::PathBuf::from(trimmed)
            };
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join(p)
            }
        };
        if candidate.is_file() {
            return Err(json_api_error(
                StatusCode::BAD_REQUEST,
                format!("'{}' is a file, not a directory.", candidate.display()),
            ));
        }
        if let Err(e) = tokio::fs::create_dir_all(&candidate).await {
            return Err(json_api_error(
                StatusCode::BAD_REQUEST,
                format!("Failed to create folder '{}': {}", candidate.display(), e),
            ));
        }
        let resolved = tokio::fs::canonicalize(&candidate)
            .await
            .map(|p| std::path::PathBuf::from(normalize_display_path(&p)))
            .unwrap_or(candidate);
        if resolved != path_buf && !extra_paths.iter().any(|existing| existing == &resolved) {
            extra_paths.push(resolved);
        }
    }

    let existing_defaults = conf
        .projects
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.defaults.clone())
        .unwrap_or_default();
    let defaults = payload.defaults.unwrap_or(existing_defaults);

    let project = crate::config::Project {
        id: id.clone(),
        name: clean_name.clone(),
        path: path_buf.clone(),
        icon: icon.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        is_active: set_active,
        defaults: defaults.clone(),
        extra_paths: extra_paths.clone(),
    };

    if let Some(pos) = conf.projects.iter().position(|p| p.id == id) {
        conf.projects[pos] = project.clone();
    } else {
        conf.projects.push(project.clone());
    }

    if set_active {
        conf.active_project_id = id.clone();
        conf.sandbox_root = path_buf.clone();
        for p in &mut conf.projects {
            p.is_active = p.id == id;
        }
    }
    drop(conf);
    state.persist_config().await;
    if let Some(ref db) = state.db {
        let _ = db.upsert_project(&project);
    }

    Ok(Json(ProjectSummary {
        id,
        name: clean_name,
        path: final_path_str,
        icon,
        is_active: set_active,
        session_count: 0,
        defaults,
        extra_paths: extra_paths
            .iter()
            .map(|f| f.display().to_string())
            .collect(),
    }))
}

pub async fn set_active_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSummary>, StatusCode> {
    let mut conf = state.config.write().await;
    let mut target_summary = None;

    if let Some(p) = conf.projects.iter().find(|p| p.id == id).cloned() {
        conf.active_project_id = id.clone();
        conf.sandbox_root = p.path.clone();
        for proj in &mut conf.projects {
            proj.is_active = proj.id == id;
        }
        target_summary = Some(ProjectSummary {
            id: p.id,
            name: p.name,
            path: p.path.display().to_string(),
            icon: p.icon,
            is_active: true,
            session_count: 0,
            extra_paths: p
                .extra_paths
                .iter()
                .map(|f| f.display().to_string())
                .collect(),
            defaults: p.defaults,
        });
    }
    drop(conf);
    if target_summary.is_some() {
        state.persist_config().await;
    }

    match target_summary {
        Some(sum) => Ok(Json(sum)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// Resolve, validate and canonicalize a list of additional project folders.
///
/// Missing folders are created (matching project creation), relative paths resolve
/// against the process cwd, and the primary project path is never duplicated.
async fn resolve_extra_paths(
    raw_paths: &[String],
) -> Result<Vec<std::path::PathBuf>, (StatusCode, Json<serde_json::Value>)> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    for raw in raw_paths {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = {
            let p = if trimmed.len() == 2
                && trimmed.as_bytes()[1] == b':'
                && trimmed.as_bytes()[0].is_ascii_alphabetic()
            {
                std::path::PathBuf::from(format!("{}\\", trimmed))
            } else {
                std::path::PathBuf::from(trimmed)
            };
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join(p)
            }
        };
        if candidate.is_file() {
            return Err(json_api_error(
                StatusCode::BAD_REQUEST,
                format!("'{}' is a file, not a directory.", candidate.display()),
            ));
        }
        if let Err(e) = tokio::fs::create_dir_all(&candidate).await {
            return Err(json_api_error(
                StatusCode::BAD_REQUEST,
                format!("Failed to create folder '{}': {}", candidate.display(), e),
            ));
        }
        let resolved = tokio::fs::canonicalize(&candidate)
            .await
            .map(|p| std::path::PathBuf::from(normalize_display_path(&p)))
            .unwrap_or(candidate);
        if !out.iter().any(|existing| existing == &resolved) {
            out.push(resolved);
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct UpdateProjectDefaultsRequest {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub path: Option<String>,
    pub defaults: crate::config::ProjectDefaults,
    /// Additional folders the agent may read/write in this project.
    /// `None` leaves the current list untouched; `Some` replaces it.
    #[serde(default)]
    pub extra_paths: Option<Vec<String>>,
}

pub async fn update_project_defaults(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateProjectDefaultsRequest>,
) -> Result<Json<ProjectSummary>, (StatusCode, Json<serde_json::Value>)> {
    let mut validated_new_path: Option<std::path::PathBuf> = None;
    if let Some(ref raw_p) = payload.path {
        let trimmed = raw_p.trim();
        if !trimmed.is_empty() {
            let p_buf = if trimmed.len() == 2
                && trimmed.as_bytes()[1] == b':'
                && trimmed.as_bytes()[0].is_ascii_alphabetic()
            {
                std::path::PathBuf::from(format!("{}\\", trimmed))
            } else {
                std::path::PathBuf::from(trimmed)
            };
            let resolved = if p_buf.is_absolute() {
                p_buf
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join(p_buf)
            };
            if resolved.is_file() {
                return Err(json_api_error(
                    StatusCode::BAD_REQUEST,
                    format!("'{}' is a file, not a directory.", resolved.display()),
                ));
            }
            if let Err(e) = tokio::fs::create_dir_all(&resolved).await {
                return Err(json_api_error(
                    StatusCode::BAD_REQUEST,
                    format!("Failed to access/create project directory '{}': {}", resolved.display(), e),
                ));
            }
            let norm = tokio::fs::canonicalize(&resolved)
                .await
                .map(|p| std::path::PathBuf::from(normalize_display_path(&p)))
                .unwrap_or_else(|_| std::path::PathBuf::from(normalize_display_path(&resolved)));
            validated_new_path = Some(norm);
        }
    }

    // Additional folders, when supplied, replace the current list. `None` keeps them.
    let validated_extra_paths = match payload.extra_paths.as_deref() {
        Some(raw) => Some(resolve_extra_paths(raw).await?),
        None => None,
    };

    let updated = {
        let mut conf = state.config.write().await;
        let is_active_proj = conf.active_project_id == id;
        let mut found_proj = None;
        if let Some(p) = conf.projects.iter_mut().find(|p| p.id == id) {
            if let Some(ref n) = payload.name {
                if !n.trim().is_empty() {
                    p.name = n.trim().to_string();
                }
            }
            if let Some(ref ic) = payload.icon {
                if !ic.trim().is_empty() {
                    p.icon = ic.trim().to_string();
                }
            }
            if let Some(ref np) = validated_new_path {
                p.path = np.clone();
            }
            if let Some(ref extras) = validated_extra_paths {
                // Never list the primary project path twice.
                let primary = p.path.clone();
                p.extra_paths = extras
                    .iter()
                    .filter(|e| e.as_path() != primary.as_path())
                    .cloned()
                    .collect();
            }
            p.defaults = payload.defaults;
            found_proj = Some(p.clone());
        }
        if is_active_proj {
            if let Some(ref np) = validated_new_path {
                conf.sandbox_root = np.clone();
            }
        }
        found_proj
    };
    if let Some(proj) = updated {
        state.persist_config().await;
        if let Some(ref db) = state.db {
            let _ = db.upsert_project(&proj);
        }
        Ok(Json(ProjectSummary {
            id: proj.id,
            name: proj.name,
            path: proj.path.display().to_string(),
            icon: proj.icon,
            is_active: proj.is_active,
            session_count: 0,
            extra_paths: proj
                .extra_paths
                .iter()
                .map(|f| f.display().to_string())
                .collect(),
            defaults: proj.defaults,
        }))
    } else {
        Err(json_api_error(StatusCode::NOT_FOUND, "Project not found"))
    }
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let fallback_id = {
        let mut conf = state.config.write().await;
        if conf.projects.len() <= 1 {
            return Err((StatusCode::BAD_REQUEST, "Cannot delete the only remaining project.".to_string()));
        }

        let orig_len = conf.projects.len();
        conf.projects.retain(|p| p.id != id);

        if conf.projects.len() == orig_len {
            return Err((StatusCode::NOT_FOUND, "Project not found".to_string()));
        }

        if conf.active_project_id == id {
            if let Some(first) = conf.projects.first() {
                let first_id = first.id.clone();
                let first_path = first.path.clone();
                conf.active_project_id = first_id;
                conf.sandbox_root = first_path;
                if let Some(first_mut) = conf.projects.first_mut() {
                    first_mut.is_active = true;
                }
            }
        }

        conf.active_project_id.clone()
    };

    let reassigned_sessions = state.reassign_project_sessions(&id, &fallback_id).await;
    state.persist_config().await;
    if let Some(ref db) = state.db {
        let _ = db.delete_project(&id);
    }

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "id": id,
        "fallback_project_id": fallback_id,
        "reassigned_sessions": reassigned_sessions
    })))
}

pub async fn stop_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.stop_session(&id).await {
        Ok(Json(serde_json::json!({ "session_id": id, "status": "stopped" })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn compact_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.compact_session_memory(&id).await {
        Some((events_compacted, before_tokens, after_tokens, messages_folded)) => {
            // Compaction rewrote history/messages/checkpoints; persist or a restart
            // restores the full uncompacted context.
            state.persist_session_record(&id).await;
            Ok(Json(serde_json::json!({
                "session_id": id,
                "status": "compacted",
                "events_compacted": events_compacted,
                "before_tokens": before_tokens,
                "after_tokens": after_tokens,
                "messages_folded": messages_folded
            })))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

// ============================================================================
// Multi-Turn Messaging, Queue / Steer, Undo, Fork & Rename APIs
// ============================================================================

#[derive(Deserialize)]
pub struct SendMessageRequest {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    #[serde(default)]
    pub mode: Option<String>,
    pub mock: Option<bool>,
    #[serde(default)]
    pub slash_command: Option<String>,
    #[serde(default)]
    pub slash_args: Option<String>,
}

pub async fn send_session_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SendMessageRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mode = payload
        .mode
        .as_deref()
        .unwrap_or("queue")
        .trim()
        .to_ascii_lowercase();
    if mode != "queue" && mode != "steer" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid mode '{}': expected 'queue' or 'steer'", mode),
        ));
    }

    if !state.ensure_session_loaded(&id).await {
        return Err((StatusCode::NOT_FOUND, "Session not found".to_string()));
    }

    let is_running = {
        let lock = state.sessions.read().await;
        let session = lock
            .get(&id)
            .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;
        session.status == "running"
    };

    let conf = { state.config.read().await.clone() };
    let sandbox = state.resolve_session_sandbox(&id).await;
    let (prompt, slash_cmd, slash_args) = resolve_slash_input(
        &conf,
        &sandbox,
        &payload.content,
        payload.slash_command.as_deref(),
        payload.slash_args.as_deref(),
    );

    if prompt.is_empty() && slash_cmd.is_none() && payload.attachments.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Message content, attachments, or slash command cannot be empty".to_string(),
        ));
    }

    let use_mock = payload.mock.unwrap_or(conf.mock_mode);

    if is_running {
        if mode == "steer" {
            let base_steer = if let Some(ref cmd) = slash_cmd {
                let (_, _, directive) = crate::state::build_slash_command_directive(
                    &conf,
                    &sandbox,
                    cmd,
                    slash_args.as_deref().unwrap_or(""),
                );
                if prompt.is_empty() {
                    directive
                } else {
                    format!("{}\n\n{}", directive, prompt)
                }
            } else {
                prompt.clone()
            };
            let att_text = format_attachments_as_text(&payload.attachments, true);
            let steer_text = if att_text.is_empty() {
                base_steer
            } else if base_steer.is_empty() {
                att_text
            } else {
                format!("{}\n\n{}", base_steer, att_text)
            };
            let remote_run = { state.active_remote_runs.read().await.get(&id).cloned() };
            if let Some((remote_srv, remote_run_id)) = remote_run {
                crate::remote::forward_remote_steer(&remote_srv, &remote_run_id, &steer_text)
                    .await
                    .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
            } else if !state.push_steer(&id, steer_text.clone()).await {
                return Err((StatusCode::NOT_FOUND, "Session not found".to_string()));
            }
            return Ok(Json(serde_json::json!({
                "status": "steered",
                "content": steer_text
            })));
        }

        let (item, position, queue) = state
            .enqueue_prompt_with_command_and_attachments(
                &id,
                prompt,
                payload.attachments,
                slash_cmd,
                slash_args,
            )
            .await
            .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;

        let display_content = format_session_title_with_attachments(
            &item.content,
            &item.attachments,
            item.slash_command.as_deref(),
            item.slash_args.as_deref(),
        );
        state
            .emit_session_event(
                &id,
                HarnessEvent::PromptQueued {
                    queue_id: item.id.clone(),
                    content: display_content,
                    position,
                },
            )
            .await;
        state
            .emit_session_event(
                &id,
                HarnessEvent::QueueUpdated {
                    queue: queue_to_json_values(&queue),
                },
            )
            .await;

        // Guard against the active turn completing right before enqueue_prompt acquired the write lock
        let status_now = {
            let lock = state.sessions.read().await;
            lock.get(&id).map(|s| s.status.clone())
        };
        if status_now.as_deref() == Some("completed") {
            if let Some((popped, remaining)) = state.pop_queued_prompt(&id).await {
                if let Some((bcast_tx, cancel_notify, project_id)) = state.reset_cancel_notify(&id).await {
                    if let Some((next_task, next_attachments, prior_messages, steer_inbox)) =
                        prepare_queued_turn(&state, &id, &popped, &remaining).await
                    {
                        spawn_session_turn(
                            state,
                            id,
                            next_task,
                            next_attachments,
                            project_id,
                            prior_messages,
                            steer_inbox,
                            bcast_tx,
                            cancel_notify,
                            use_mock,
                        );
                    }
                }
            }
        }

        return Ok(Json(serde_json::json!({
            "status": "queued",
            "queue_id": item.id,
            "position": position,
            "queue": queue
        })));
    }

    let (bcast_tx, cancel_notify, project_id) = state
        .reset_cancel_notify(&id)
        .await
        .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    let (turn_id, prior_messages, steer_inbox) = state
        .begin_turn_with_command_and_attachments(
            &id,
            &prompt,
            payload.attachments.clone(),
            slash_cmd.as_deref(),
            slash_args.as_deref(),
        )
        .await
        .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    let mut runner_task = format_runner_task(&prompt, slash_cmd.as_deref(), slash_args.as_deref());
    if runner_task.trim().is_empty() && !payload.attachments.is_empty() {
        runner_task = "Analyze the attached file(s).".to_string();
    }
    let should_retitle = {
        let lock = state.sessions.read().await;
        lock.get(&id).is_some_and(|s| s.task == "New Conversation")
    };
    if should_retitle {
        let new_title = format_session_title_with_attachments(
            &prompt,
            &payload.attachments,
            slash_cmd.as_deref(),
            slash_args.as_deref(),
        );
        state.rename_session(&id, new_title).await;
    }
    spawn_session_turn(
        state,
        id,
        runner_task,
        payload.attachments,
        project_id,
        prior_messages,
        steer_inbox,
        bcast_tx,
        cancel_notify,
        use_mock,
    );

    Ok(Json(serde_json::json!({
        "status": "running",
        "turn_id": turn_id
    })))
}

pub async fn get_session_queue(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let queue = state
        .list_queued_prompts(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "queue": queue })))
}

pub async fn delete_queued_prompt(
    State(state): State<AppState>,
    Path((id, queue_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let queue = state
        .remove_queued_prompt(&id, &queue_id)
        .await
        .ok_or((StatusCode::NOT_FOUND, "Session or queued prompt not found".to_string()))?;

    state
        .emit_session_event(
            &id,
            HarnessEvent::QueueUpdated {
                queue: queue_to_json_values(&queue),
            },
        )
        .await;

    Ok(Json(serde_json::json!({
        "status": "removed",
        "queue": queue
    })))
}

pub async fn steer_queued_prompt(
    State(state): State<AppState>,
    Path((id, queue_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (item, queue) = state
        .promote_queued_to_steer(&id, &queue_id)
        .await
        .ok_or((StatusCode::NOT_FOUND, "Session or queued prompt not found".to_string()))?;

    state
        .emit_session_event(
            &id,
            HarnessEvent::QueueUpdated {
                queue: queue_to_json_values(&queue),
            },
        )
        .await;

    Ok(Json(serde_json::json!({
        "status": "steered",
        "queue_id": item.id,
        "content": item.content,
        "queue": queue
    })))
}

#[derive(Deserialize)]
pub struct UndoSessionRequest {
    pub turn_id: usize,
}

pub async fn undo_session_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UndoSessionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = state.ensure_session_loaded(&id).await;
    let (restored_prompt, history) = state
        .undo_to_turn(&id, payload.turn_id)
        .await
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Session '{}' or turn {} not found", id, payload.turn_id),
        ))?;
    state.persist_session_record(&id).await;

    Ok(Json(serde_json::json!({
        "status": "undone",
        "session_id": id,
        "restored_prompt": restored_prompt,
        "history": history
    })))
}

#[derive(Deserialize, Default)]
pub struct ForkSessionRequest {
    pub turn_id: Option<usize>,
}

pub async fn fork_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = state.ensure_session_loaded(&id).await;
    let payload: ForkSessionRequest = if body.is_empty() {
        ForkSessionRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid JSON body: {}", e)))?
    };

    let (new_session_id, title, project_id) = state
        .fork_session(&id, payload.turn_id)
        .await
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("Session '{}' or turn {:?} not found", id, payload.turn_id),
        ))?;

    Ok(Json(serde_json::json!({
        "session_id": new_session_id,
        "title": title,
        "project_id": project_id
    })))
}

#[derive(Deserialize)]
pub struct RenameSessionRequest {
    pub title: String,
}

pub async fn rename_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RenameSessionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let title = payload.title.trim().to_string();
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Title cannot be empty".to_string()));
    }

    if state.rename_session(&id, title.clone()).await {
        Ok(Json(serde_json::json!({
            "status": "renamed",
            "id": id,
            "title": title
        })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Session '{}' not found", id)))
    }
}

#[derive(Deserialize, Default)]
pub struct CreateIdleConversationRequest {
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub server_id: Option<String>,
    pub orchestrator_mode: Option<bool>,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub reasoning_effort: Option<String>,
}

pub async fn create_idle_conversation(
    State(state): State<AppState>,
    Json(payload): Json<CreateIdleConversationRequest>,
) -> Json<SessionSummary> {
    let session_id = format!("ses_{}", chrono::Utc::now().timestamp_millis());
    let title = payload
        .title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "New Conversation".to_string());
    // Fall back to the active project so project defaults still apply.
    let project_id = payload
        .project_id
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| {
            state
                .config
                .try_read()
                .map(|c| c.active_project_id.clone())
                .unwrap_or_default()
        });

    let _ = state
        .create_session_with_options(
            session_id.clone(),
            title,
            project_id,
            payload.server_id,
            payload.orchestrator_mode,
            // An empty model means "use the project default", not an override.
            payload
                .model
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()),
            "idle",
        )
        .await;
    if payload
        .provider_id
        .as_deref()
        .map(|p| !p.trim().is_empty())
        .unwrap_or(false)
        || payload
            .reasoning_effort
            .as_deref()
            .map(|e| !e.trim().is_empty())
            .unwrap_or(false)
    {
        state
            .update_session_meta(
                &session_id,
                None,
                None,
                None,
                None,
                None,
                payload.provider_id,
                payload.reasoning_effort,
            )
            .await;
    }

    let lock = state.sessions.read().await;
    let s = lock.get(&session_id).expect("idle session created");
    Json(SessionSummary {
        id: s.id.clone(),
        project_id: s.project_id.clone(),
        created_at: s.created_at.clone(),
        updated_at: s.updated_at.clone(),
        task: s.task.clone(),
        status: s.status.clone(),
        model: s.model.clone(),
        provider_id: s.provider_id.clone(),
        reasoning_effort: s.reasoning_effort.clone(),
        server_id: s.server_id.clone(),
        orchestrator_mode: s.orchestrator_mode,
        knowledge_only: crate::config::is_knowledge_only_project(&s.project_id),
    })
}

#[derive(Deserialize, Default)]
pub struct UpdateSessionMetaRequest {
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub server_id: Option<String>,
    pub orchestrator_mode: Option<bool>,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub reasoning_effort: Option<String>,
}

pub async fn update_session_meta_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateSessionMetaRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.ensure_session_loaded(&id).await {
        return Err((StatusCode::NOT_FOUND, format!("Session '{}' not found", id)));
    }
    if let Some(ref t) = payload.title {
        if t.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Title cannot be empty".to_string()));
        }
    }
    let title_opt = payload.title.map(|s| s.trim().to_string());
    let is_rename_only = title_opt.is_some()
        && payload.project_id.is_none()
        && payload.server_id.is_none()
        && payload.orchestrator_mode.is_none()
        && payload.model.is_none()
        && payload.provider_id.is_none()
        && payload.reasoning_effort.is_none();
    if state
        .update_session_meta(
            &id,
            title_opt.clone(),
            payload.project_id.clone(),
            payload.server_id.clone(),
            payload.orchestrator_mode,
            payload.model.clone(),
            payload.provider_id.clone(),
            payload.reasoning_effort.clone(),
        )
        .await
    {
        Ok(Json(serde_json::json!({
            "status": if is_rename_only { "renamed" } else { "updated" },
            "id": id,
            "title": title_opt,
            "project_id": payload.project_id,
            "server_id": payload.server_id,
            "orchestrator_mode": payload.orchestrator_mode,
            "model": payload.model,
            "provider_id": payload.provider_id,
            "reasoning_effort": payload.reasoning_effort
        })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Session '{}' not found", id)))
    }
}

pub async fn delete_session_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.delete_session(&id).await {
        Ok(Json(serde_json::json!({ "status": "deleted", "id": id })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ============================================================================
// Multi-Server Directory & Remote Server Execution Endpoints
// ============================================================================

pub async fn list_servers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let conf = state.config.read().await;
    // Never return stored api_tokens; clients send `SECRET_REDACTED` back unchanged.
    let mut servers = conf.servers.clone();
    for s in servers.iter_mut() {
        s.redact_secrets();
    }
    Json(serde_json::json!({
        "active_server_id": conf.active_server_id,
        "servers": servers
    }))
}

pub async fn save_server(
    State(state): State<AppState>,
    Json(mut srv): Json<crate::config::ServerNodeConfig>,
) -> Result<Json<crate::config::ServerNodeConfig>, (StatusCode, String)> {
    srv.id = srv.id.trim().to_string();
    srv.name = srv.name.trim().to_string();
    srv.url = srv.url.trim().trim_end_matches('/').to_string();
    if srv.id.is_empty() {
        srv.id = format!("srv_{}", chrono::Utc::now().timestamp_millis());
    }
    if srv.name.is_empty() {
        srv.name = srv.id.clone();
    }
    if srv.url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Server URL cannot be empty".to_string()));
    }

    {
        let mut conf = state.config.write().await;
        // Preserve a stored token when the client echoes the redacted placeholder.
        if let Some(existing) = conf.servers.iter().find(|s| s.id == srv.id) {
            srv.restore_secrets(existing);
        }
        if srv.is_default {
            for s in &mut conf.servers {
                s.is_default = false;
            }
            conf.active_server_id = srv.id.clone();
        }
        if let Some(pos) = conf.servers.iter().position(|s| s.id == srv.id) {
            conf.servers[pos] = srv.clone();
        } else {
            conf.servers.push(srv.clone());
        }
    }
    state.persist_config().await;
    if let Some(ref db) = state.db {
        let _ = db.upsert_server(&srv);
    }
    let mut out = srv;
    out.redact_secrets();
    Ok(Json(out))
}

pub async fn delete_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if id == "local" {
        return Err((StatusCode::BAD_REQUEST, "Cannot delete the built-in local server.".to_string()));
    }
    {
        let mut conf = state.config.write().await;
        let orig = conf.servers.len();
        conf.servers.retain(|s| s.id != id);
        if conf.servers.len() == orig {
            return Err((StatusCode::NOT_FOUND, format!("Server '{}' not found", id)));
        }
        if conf.active_server_id == id {
            conf.active_server_id = "local".to_string();
        }
    }
    state.persist_config().await;
    if let Some(ref db) = state.db {
        let _ = db.delete_server(&id);
    }
    Ok(Json(serde_json::json!({ "status": "deleted", "id": id })))
}

pub async fn set_active_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut found = false;
    {
        let mut conf = state.config.write().await;
        if conf.servers.iter().any(|s| s.id == id) {
            conf.active_server_id = id.clone();
            for s in &mut conf.servers {
                s.is_default = s.id == id;
            }
            found = true;
        }
    }
    if !found {
        return Err(StatusCode::NOT_FOUND);
    }
    state.persist_config().await;
    Ok(Json(serde_json::json!({ "active_server_id": id })))
}

pub async fn check_server_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let srv = {
        let conf = state.config.read().await;
        conf.servers.iter().find(|s| s.id == id).cloned()
    }
    .ok_or(StatusCode::NOT_FOUND)?;

    if srv.id == "local" || srv.url.starts_with("local://") {
        let reg = state.get_registry_snapshot().await;
        let conf = state.config.read().await;
        return Ok(Json(serde_json::json!({
            "online": true,
            "ok": true,
            "server_id": "local",
            "name": srv.name,
            "sandbox_root": conf.sandbox_root.display().to_string(),
            "tools_count": reg.get_contracts().len()
        })));
    }

    match crate::remote::probe_remote_server(&srv).await {
        Ok(mut info) => {
            if let Some(obj) = info.as_object_mut() {
                obj.insert("online".to_string(), serde_json::json!(true));
                obj.insert("ok".to_string(), serde_json::json!(true));
            }
            Ok(Json(info))
        }
        Err(err) => Ok(Json(serde_json::json!({
            "online": false,
            "ok": false,
            "server_id": srv.id,
            "name": srv.name,
            "error": err
        }))),
    }
}

pub async fn server_info(State(state): State<AppState>) -> Json<serde_json::Value> {
    let conf = state.config.read().await.clone();
    let reg = state.get_registry_snapshot().await;
    let tools: Vec<String> = reg
        .get_contracts()
        .into_iter()
        .map(|c| c.name)
        .collect();
    let active_runs = {
        let runs = state.server_runs.read().await;
        runs.values().filter(|r| r.status == "running").count()
    };
    let sid = if conf.server_identity.trim().is_empty() {
        "local".to_string()
    } else {
        conf.server_identity.clone()
    };
    Json(serde_json::json!({
        "server_id": sid,
        "name": format!("Harness Execution Server ({})", sid),
        "version": env!("CARGO_PKG_VERSION"),
        "sandbox_root": conf.sandbox_root.display().to_string(),
        "tools_count": tools.len(),
        "tools": tools,
        "active_runs": active_runs
    }))
}

pub async fn start_server_run(
    State(state): State<AppState>,
    Json(payload): Json<crate::remote::RemoteRunRequest>,
) -> Json<serde_json::Value> {
    let run_id = payload.run_id.clone();
    let session_id = payload.session_id.clone();
    let (tx, _) = broadcast::channel(256);
    let cancel_notify = Arc::new(tokio::sync::Notify::new());
    let steer_inbox = Arc::new(TokioMutex::new(VecDeque::new()));

    {
        let mut runs = state.server_runs.write().await;
        runs.insert(
            run_id.clone(),
            crate::state::ServerRunHandle {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                status: "running".to_string(),
                cancel_notify: cancel_notify.clone(),
                steer_inbox: steer_inbox.clone(),
                event_tx: tx.clone(),
                history: Vec::new(),
                updated_messages: None,
                final_answer: None,
                error: None,
            },
        );
    }

    // Ensure a SwarmBoard exists on this server for the session so local/remote subagents coordinate cleanly.
    let _ = state.get_or_create_swarm(&session_id).await;

    let state_bg = state.clone();
    tokio::spawn(async move {
        let server_conf = state_bg.config.read().await.clone();
        let mut run_conf = payload.config.clone();
        // Each execution server operates within its own configured sandbox workspace.
        run_conf.sandbox_root = server_conf.sandbox_root.clone();
        if !server_conf.server_identity.trim().is_empty() {
            run_conf.server_identity = server_conf.server_identity.clone();
        }
        if payload.read_only {
            run_conf.permissions.mode = crate::config::PermissionMode::ReadOnly;
        }

        let sandbox = run_conf.sandbox_root.clone();
        let registry = state_bg.get_registry_snapshot().await;
        let model: Arc<dyn ModelAdapter> = if payload.use_mock || run_conf.mock_mode {
            Arc::new(MockClient)
        } else if is_google_search_model(&run_conf.model) {
            Arc::new(GoogleSearchClient::new())
        } else {
            Arc::new(OpenAiClient::new(run_conf.clone()))
        };

        let mut runner = AgentLoopRunner::new(
            session_id.clone(),
            payload.task,
            run_conf,
            registry,
            model,
            Some(state_bg.clone()),
        )
        .with_sandbox(sandbox)
        .with_cancel_notify(cancel_notify)
        .with_prior_messages(payload.prior_messages)
        .with_task_attachments(payload.attachments)
        .with_steer_inbox(steer_inbox);

        if let Some(ref ag_id) = payload.agent_id {
            runner = runner.with_subagent(
                ag_id.clone(),
                payload.role.unwrap_or_else(|| "worker".to_string()),
                payload.parent_id,
                payload.depth,
            );
        }

        let (evt_tx, mut evt_rx) = mpsc::channel::<HarnessEvent>(128);
        let join = tokio::spawn(async move { runner.run_with_messages(evt_tx).await });

        while let Some(evt) = evt_rx.recv().await {
            {
                let mut runs = state_bg.server_runs.write().await;
                if let Some(h) = runs.get_mut(&run_id) {
                    if h.history.len() < 2000 {
                        h.history.push(evt.clone());
                    }
                    let _ = h.event_tx.send(evt);
                }
            }
        }

        let res = join.await;
        let mut runs = state_bg.server_runs.write().await;
        if let Some(h) = runs.get_mut(&run_id) {
            match res {
                Ok(Ok((ans, msgs))) => {
                    h.status = "completed".to_string();
                    h.final_answer = Some(ans);
                    h.updated_messages = Some(msgs);
                }
                Ok(Err(err)) => {
                    h.status = "error".to_string();
                    h.error = Some(err);
                }
                Err(je) => {
                    h.status = "error".to_string();
                    h.error = Some(format!("Server run task panicked: {}", je));
                }
            }
        }
    });

    Json(serde_json::json!({
        "run_id": payload.run_id,
        "status": "running"
    }))
}

pub async fn stream_server_run_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let (rx, snapshot, initial_done) = {
        let runs = state.server_runs.read().await;
        let h = runs.get(&run_id).ok_or(StatusCode::NOT_FOUND)?;
        let done = if h.status != "running" {
            Some(crate::remote::RemoteRunFinished {
                status: h.status.clone(),
                final_answer: h.final_answer.clone().unwrap_or_default(),
                error: h.error.clone(),
                updated_messages: h.updated_messages.clone().unwrap_or_default(),
            })
        } else {
            None
        };
        (h.event_tx.subscribe(), h.history.clone(), done)
    };

    let state_c = state.clone();
    let rid_c = run_id.clone();

    let s = async_stream_unfold_run(state_c, rid_c, rx, snapshot, initial_done);
    Ok(Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

fn async_stream_unfold_run(
    state: AppState,
    run_id: String,
    rx: broadcast::Receiver<HarnessEvent>,
    snapshot: Vec<HarnessEvent>,
    initial_done: Option<crate::remote::RemoteRunFinished>,
) -> impl futures_util::Stream<Item = Result<Event, Infallible>> {
    struct StreamState {
        state: AppState,
        run_id: String,
        rx: broadcast::Receiver<HarnessEvent>,
        pending: VecDeque<HarnessEvent>,
        initial_done: Option<crate::remote::RemoteRunFinished>,
        finished_sent: bool,
    }

    let init = StreamState {
        state,
        run_id,
        rx,
        pending: VecDeque::from(snapshot),
        initial_done,
        finished_sent: false,
    };

    stream::unfold(init, |mut st| async move {
        if st.finished_sent {
            return None;
        }
        if let Some(evt) = st.pending.pop_front() {
            let json_data = serde_json::to_string(&evt).unwrap_or_default();
            return Some((Ok(Event::default().data(json_data)), st));
        }
        if let Some(fin) = st.initial_done.take() {
            st.finished_sent = true;
            let fin_json = serde_json::to_string(&fin).unwrap_or_default();
            return Some((Ok(Event::default().event("run_finished").data(fin_json)), st));
        }

        loop {
            match tokio::time::timeout(Duration::from_millis(250), st.rx.recv()).await {
                Ok(Ok(evt)) => {
                    let json_data = serde_json::to_string(&evt).unwrap_or_default();
                    return Some((Ok(Event::default().data(json_data)), st));
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => {
                    let fin_opt = {
                        let runs = st.state.server_runs.read().await;
                        runs.get(&st.run_id).and_then(|h| {
                            if h.status != "running" {
                                Some(crate::remote::RemoteRunFinished {
                                    status: h.status.clone(),
                                    final_answer: h.final_answer.clone().unwrap_or_default(),
                                    error: h.error.clone(),
                                    updated_messages: h.updated_messages.clone().unwrap_or_default(),
                                })
                            } else {
                                None
                            }
                        })
                    };
                    if let Some(fin) = fin_opt {
                        st.finished_sent = true;
                        let fin_json = serde_json::to_string(&fin).unwrap_or_default();
                        return Some((Ok(Event::default().event("run_finished").data(fin_json)), st));
                    }
                }
            }
        }
    })
}

pub async fn stop_server_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session_id = {
        let mut runs = state.server_runs.write().await;
        let h = runs.get_mut(&run_id).ok_or(StatusCode::NOT_FOUND)?;
        h.cancel_notify.notify_one();
        if h.status == "running" {
            h.status = "stopped".to_string();
        }
        h.session_id.clone()
    };
    if let Some(swarm) = state.get_session_swarm(&session_id).await {
        swarm.cancel_all_subagents().await;
    }
    Ok(Json(serde_json::json!({ "status": "stopped", "run_id": run_id })))
}

#[derive(Deserialize)]
pub struct SteerServerRunRequest {
    pub content: String,
}

pub async fn steer_server_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(payload): Json<SteerServerRunRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let inbox = {
        let runs = state.server_runs.read().await;
        let h = runs.get(&run_id).ok_or(StatusCode::NOT_FOUND)?;
        h.steer_inbox.clone()
    };
    inbox.lock().await.push_back(payload.content);
    Ok(Json(serde_json::json!({ "status": "steered", "run_id": run_id })))
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
    pub strict: Option<bool>,
}

#[derive(Serialize)]
pub struct BrowseEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: Option<u64>,
    pub modified: Option<String>,
}

#[derive(Serialize)]
pub struct FavoritePath {
    pub name: String,
    pub path: String,
    pub icon: String,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub drives: Vec<String>,
    pub favorites: Vec<FavoritePath>,
    pub entries: Vec<BrowseEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

fn collect_drives_and_favorites() -> (Vec<String>, Vec<FavoritePath>) {
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        if std::path::Path::new(&drive).exists() {
            drives.push(drive);
        }
    }

    let mut favorites = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let cwd_norm = normalize_display_path(&cwd);
    favorites.push(FavoritePath {
        name: "Workspace".to_string(),
        path: cwd_norm,
        icon: "📁".to_string(),
    });

    let projects_dir = cwd.join("projects");
    if projects_dir.exists() {
        favorites.push(FavoritePath {
            name: "Projects".to_string(),
            path: normalize_display_path(&projects_dir),
            icon: "🚀".to_string(),
        });
    }

    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let home_p = std::path::PathBuf::from(&home);
        if home_p.exists() {
            favorites.push(FavoritePath {
                name: "Home".to_string(),
                path: normalize_display_path(&home_p),
                icon: "🏠".to_string(),
            });
        }

        let downloads = home_p.join("Downloads");
        if downloads.exists() {
            favorites.push(FavoritePath {
                name: "Downloads".to_string(),
                path: normalize_display_path(&downloads),
                icon: "📥".to_string(),
            });
        }

        let desktop = home_p.join("Desktop");
        if desktop.exists() {
            favorites.push(FavoritePath {
                name: "Desktop".to_string(),
                path: normalize_display_path(&desktop),
                icon: "🖥️".to_string(),
            });
        }

        let docs = home_p.join("Documents");
        if docs.exists() {
            favorites.push(FavoritePath {
                name: "Documents".to_string(),
                path: normalize_display_path(&docs),
                icon: "📄".to_string(),
            });
        }
    }

    (drives, favorites)
}

pub async fn browse_filesystem(
    State(state): State<AppState>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, (StatusCode, Json<serde_json::Value>)> {
    let conf = state.config.read().await;
    let default_path = conf.sandbox_root.clone();
    drop(conf);

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let strict = query.strict.unwrap_or(false);
    let requested_raw = query.path.as_deref().map(|s| s.trim()).unwrap_or("");

    let mut candidate = if !requested_raw.is_empty() {
        // Normalize bare Windows drive like "C:" -> "C:\"
        let fixed = if requested_raw.len() == 2
            && requested_raw.as_bytes()[1] == b':'
            && requested_raw.as_bytes()[0].is_ascii_alphabetic()
        {
            format!("{}\\", requested_raw)
        } else {
            requested_raw.to_string()
        };
        let p = std::path::PathBuf::from(fixed);
        if p.is_absolute() { p } else { cwd.join(p) }
    } else {
        default_path.clone()
    };

    if candidate.is_file() {
        candidate = candidate.parent().unwrap_or(&candidate).to_path_buf();
    }

    let mut notice: Option<String> = None;
    if !candidate.exists() || !candidate.is_dir() {
        if strict {
            return Err(json_api_error(
                StatusCode::BAD_REQUEST,
                format!("Directory '{}' does not exist.", candidate.display()),
            ));
        }
        // Gracefully resolve to nearest existing ancestor, else default_path / cwd / home / drive
        let mut resolved_fallback: Option<std::path::PathBuf> = None;
        for anc in candidate.ancestors() {
            if !anc.as_os_str().is_empty() && anc.exists() && anc.is_dir() {
                resolved_fallback = Some(anc.to_path_buf());
                break;
            }
        }
        if resolved_fallback.is_none() && default_path.exists() && default_path.is_dir() {
            resolved_fallback = Some(default_path);
        }
        if resolved_fallback.is_none() && cwd.exists() && cwd.is_dir() {
            resolved_fallback = Some(cwd.clone());
        }
        if let Some(fb) = resolved_fallback {
            if !requested_raw.is_empty() {
                notice = Some(format!(
                    "'{}' does not exist yet — showing '{}'.",
                    requested_raw,
                    normalize_display_path(&fb)
                ));
            }
            candidate = fb;
        }
    }

    let canonical = tokio::fs::canonicalize(&candidate)
        .await
        .unwrap_or_else(|_| candidate.clone());
    let current_path_str = normalize_display_path(&canonical);

    let parent_path = std::path::Path::new(&current_path_str)
        .parent()
        .map(normalize_display_path)
        .filter(|p| !p.is_empty() && p != &current_path_str);

    let mut entries = Vec::new();
    match tokio::fs::read_dir(&current_path_str).await {
        Ok(mut dir) => {
            while let Ok(Some(entry)) = dir.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                let meta = entry.metadata().await.ok();
                let size_bytes = if is_dir { None } else { meta.as_ref().map(|m| m.len()) };
                let modified = meta.and_then(|m| m.modified().ok()).map(|t| {
                    let datetime: chrono::DateTime<chrono::Local> = t.into();
                    datetime.to_rfc3339()
                });

                let clean_entry_path = normalize_display_path(&entry.path());

                entries.push(BrowseEntry {
                    name,
                    path: clean_entry_path,
                    is_dir,
                    size_bytes,
                    modified,
                });
            }
        }
        Err(e) => {
            return Err(json_api_error(
                StatusCode::BAD_REQUEST,
                format!("Cannot read directory '{}': {}", current_path_str, e),
            ));
        }
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    let (drives, favorites) = collect_drives_and_favorites();

    Ok(Json(BrowseResponse {
        current_path: current_path_str,
        parent_path,
        drives,
        favorites,
        entries,
        notice,
    }))
}

#[derive(Deserialize)]
pub struct MkdirRequest {
    pub parent_path: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct MkdirResponse {
    pub success: bool,
    pub path: String,
}

pub async fn create_directory(
    Json(payload): Json<MkdirRequest>,
) -> Result<Json<MkdirResponse>, (StatusCode, Json<serde_json::Value>)> {
    let clean_parent = payload.parent_path.trim();
    if clean_parent.is_empty() {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Parent directory path is empty."));
    }
    let clean_name = payload.name.trim();
    if clean_name.is_empty()
        || clean_name.contains('/')
        || clean_name.contains('\\')
        || clean_name.contains(':')
        || clean_name == ".."
        || clean_name == "."
    {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Invalid folder name."));
    }

    let parent = std::path::PathBuf::from(clean_parent);
    if !parent.exists() || !parent.is_dir() {
        return Err(json_api_error(
            StatusCode::BAD_REQUEST,
            format!("Parent directory '{}' does not exist.", clean_parent),
        ));
    }

    let target = parent.join(clean_name);

    if let Err(e) = tokio::fs::create_dir_all(&target).await {
        return Err(json_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create folder: {}", e),
        ));
    }

    let final_path = tokio::fs::canonicalize(&target)
        .await
        .map(|p| normalize_display_path(&p))
        .unwrap_or_else(|_| normalize_display_path(&target));

    Ok(Json(MkdirResponse {
        success: true,
        path: final_path,
    }))
}

// ============================================================================
// Swarm & Subagent REST APIs
// ============================================================================

#[derive(Serialize)]
pub struct CoordinatorSummary {
    pub id: String,
    pub role: String,
    pub model: String,
    pub status: String,
    pub summary: String,
    pub last_seq: Option<usize>,
}

#[derive(Serialize)]
pub struct SwarmStateResponse {
    pub swarm_title: String,
    pub coordinator: CoordinatorSummary,
    pub subagents: Vec<SubagentInfo>,
    pub board_messages: Vec<SwarmMessage>,
    pub artifacts: Vec<ArtifactRecord>,
    pub jobs: Vec<JobTag>,
    pub specialists: Vec<SpecialistTag>,
}

pub async fn get_session_swarm(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SwarmStateResponse>, StatusCode> {
    let swarm = state
        .get_session_swarm(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let (swarm_title_raw, subagents, board_messages, artifacts, jobs, specialists) = tokio::join!(
        swarm.get_title(),
        swarm.list_subagents(),
        swarm.all_messages(),
        swarm.list_artifacts(),
        swarm.list_jobs(),
        swarm.list_specialists()
    );

    let (sess_task, sess_status, sess_model) = {
        let lock = state.sessions.read().await;
        if let Some(sess) = lock.get(&id) {
            (sess.task.clone(), sess.status.clone(), sess.model.clone())
        } else {
            drop(lock);
            let cfg_model = state.config.read().await.model.clone();
            ("Swarm Session".to_string(), "completed".to_string(), cfg_model)
        }
    };

    let swarm_title = if swarm_title_raw.trim().is_empty() {
        sess_task.clone()
    } else {
        swarm_title_raw
    };

    let coord_last_msg = board_messages
        .iter()
        .rev()
        .find(|m| m.sender.eq_ignore_ascii_case("coordinator") || m.sender.eq_ignore_ascii_case("root"));

    let coordinator = CoordinatorSummary {
        id: "coordinator".to_string(),
        role: "Coordinator".to_string(),
        model: sess_model,
        status: sess_status,
        summary: coord_last_msg
            .map(|m| m.content.clone())
            .unwrap_or_else(|| sess_task.clone()),
        last_seq: coord_last_msg.map(|m| m.seq),
    };

    Ok(Json(SwarmStateResponse {
        swarm_title,
        coordinator,
        subagents,
        board_messages,
        artifacts,
        jobs,
        specialists,
    }))
}

fn default_swarm_sender() -> String {
    "user".to_string()
}

#[derive(Deserialize)]
pub struct SwarmPostRequest {
    #[serde(default = "default_swarm_sender")]
    pub sender: String,
    #[serde(default)]
    pub target: Option<String>,
    pub content: String,
}

pub async fn post_session_swarm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<SwarmPostRequest>,
) -> Result<Json<SwarmMessage>, (StatusCode, String)> {
    let content = payload.content.trim().to_string();
    if content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Message content cannot be empty".to_string()));
    }

    let swarm = state
        .get_session_swarm(&id)
        .await
        .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    let sender = if payload.sender.trim().is_empty() {
        "user".to_string()
    } else {
        payload.sender.trim().to_string()
    };

    let target = payload
        .target
        .map(|t| t.trim().trim_start_matches('@').to_string())
        .filter(|t| !t.is_empty());

    let msg = swarm.post_message(sender, target, content).await;
    state.emit_session_event(&id, msg.to_event()).await;

    Ok(Json(msg))
}

pub async fn stop_subagent(
    State(state): State<AppState>,
    Path((id, sub_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let swarm = state
        .get_session_swarm(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let role = swarm.get_subagent(&sub_id).await.map(|h| h.info.role);
    if swarm.cancel_subagent(&sub_id).await {
        if let Some(role) = role {
            state
                .emit_session_event(
                    &id,
                    HarnessEvent::SubagentUpdate {
                        subagent_id: sub_id.clone(),
                        role,
                        status: "stopped".to_string(),
                        delta_kind: "status".to_string(),
                        content: "Stopped by user".to_string(),
                    },
                )
                .await;
        }
        Ok(Json(serde_json::json!({
            "session_id": id,
            "subagent_id": sub_id,
            "status": "stopped"
        })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ============================================================================
// Artifacts & Workspace Explorer APIs
// ============================================================================

const MAX_ARTIFACT_BYTES: u64 = 25 * 1024 * 1024;
const MAX_WORKSPACE_FILE_BYTES: usize = 200 * 1024;
const MAX_WORKSPACE_TREE_ENTRIES: usize = 500;

async fn resolve_sandbox(state: &AppState, session_id: Option<&str>) -> PathBuf {
    if let Some(sid) = session_id.map(str::trim).filter(|s| !s.is_empty()) {
        return state.resolve_session_sandbox(sid).await;
    }
    state.config.read().await.sandbox_root.clone()
}

fn normalize_relative_path(raw: &str) -> Result<PathBuf, (StatusCode, String)> {
    let trimmed = raw.trim().trim_start_matches('/').trim_start_matches('\\');
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err((StatusCode::BAD_REQUEST, "Invalid empty or null path".to_string()));
    }

    let mut clean = PathBuf::new();
    for comp in FsPath::new(trimmed).components() {
        match comp {
            Component::Normal(seg) => clean.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err((StatusCode::FORBIDDEN, "Path traversal is not allowed".to_string()));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty relative path".to_string()));
    }
    Ok(clean)
}

async fn resolve_contained_path(
    base_dir: &FsPath,
    rel: &FsPath,
) -> Result<PathBuf, (StatusCode, String)> {
    let canonical_base = tokio::fs::canonicalize(base_dir)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Base directory does not exist".to_string()))?;
    let canonical_target = tokio::fs::canonicalize(canonical_base.join(rel))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Requested path not found".to_string()))?;

    if !canonical_target.starts_with(&canonical_base) {
        return Err((StatusCode::FORBIDDEN, "Access outside sandbox is forbidden".to_string()));
    }
    Ok(canonical_target)
}

fn content_type_for_path(path: &FsPath) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "md" | "markdown" => "text/markdown; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "txt" | "log" | "csv" | "toml" | "yaml" | "yml" | "rs" | "xml" => {
            "text/plain; charset=utf-8"
        }
        _ => "application/octet-stream",
    }
}

fn language_for_path(path: &FsPath) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "jsx" => "jsx",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "py" => "python",
        "sh" | "bash" => "bash",
        "ps1" => "powershell",
        "sql" => "sql",
        "xml" | "svg" => "xml",
        _ => "text",
    }
}

#[derive(Deserialize, Default)]
pub struct SessionScopedQuery {
    pub session_id: Option<String>,
}

pub async fn get_artifact(
    State(state): State<AppState>,
    Path(raw_path): Path<String>,
    Query(query): Query<SessionScopedQuery>,
) -> Result<Response, (StatusCode, String)> {
    let trimmed = raw_path.trim_start_matches('/').trim_start_matches('\\');
    let stripped = trimmed
        .strip_prefix(".harness_artifacts/")
        .or_else(|| trimmed.strip_prefix(".harness_artifacts\\"))
        .unwrap_or(trimmed);

    let rel = normalize_relative_path(stripped)?;
    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let artifacts_root = sandbox.join(".harness_artifacts");
    let target = resolve_contained_path(&artifacts_root, &rel).await?;

    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Artifact not found".to_string()))?;
    if !meta.is_file() {
        return Err((StatusCode::NOT_FOUND, "Artifact is not a file".to_string()));
    }
    if meta.len() > MAX_ARTIFACT_BYTES {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "Artifact exceeds size limit".to_string()));
    }

    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read artifact: {}", e)))?;

    let mime = content_type_for_path(&target);
    // Active content (HTML/SVG/XML/JS) served inline on the app origin would run
    // script with full API access. Force a download and sandbox it.
    let is_active_content = matches!(
        mime,
        "text/html" | "image/svg+xml" | "application/xhtml+xml" | "text/xml" | "application/xml"
            | "application/javascript" | "text/javascript"
    );
    if is_active_content {
        let filename = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("artifact")
            .replace(['"', '\r', '\n'], "_");
        return Ok((
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                ),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; sandbox".to_string(),
                ),
            ],
            bytes,
        )
            .into_response());
    }
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; sandbox"
                    .to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize, Default)]
pub struct WorkspaceTreeQuery {
    pub path: Option<String>,
    pub depth: Option<usize>,
    pub session_id: Option<String>,
}

#[derive(Serialize)]
pub struct WorkspaceTreeNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WorkspaceTreeNode>>,
}

#[derive(Serialize)]
pub struct WorkspaceTreeResponse {
    pub root: String,
    pub relative_path: String,
    pub truncated: bool,
    pub entries: Vec<WorkspaceTreeNode>,
}

fn should_skip_workspace_dir(name: &str) -> bool {
    matches!(name, "target" | ".git" | "node_modules")
}

async fn read_workspace_dir_recursive(
    dir: &FsPath,
    rel_prefix: &str,
    depth_remaining: usize,
    total_count: &mut usize,
    truncated: &mut bool,
) -> Vec<WorkspaceTreeNode> {
    if depth_remaining == 0 || *total_count >= MAX_WORKSPACE_TREE_ENTRIES {
        if *total_count >= MAX_WORKSPACE_TREE_ENTRIES {
            *truncated = true;
        }
        return Vec::new();
    }

    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return Vec::new();
    };

    struct RawEntry {
        name: String,
        abs_path: PathBuf,
        is_dir: bool,
        size_bytes: Option<u64>,
    }

    let mut raw_entries = Vec::new();
    while let Ok(Some(entry)) = rd.next_entry().await {
        let Ok(ft) = entry.file_type().await else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = ft.is_dir();
        if is_dir && should_skip_workspace_dir(&name) {
            continue;
        }
        let size_bytes = if is_dir {
            None
        } else {
            entry.metadata().await.ok().map(|m| m.len())
        };
        raw_entries.push(RawEntry {
            name,
            abs_path: entry.path(),
            is_dir,
            size_bytes,
        });
    }

    raw_entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    let mut nodes = Vec::new();
    for item in raw_entries {
        if *total_count >= MAX_WORKSPACE_TREE_ENTRIES {
            *truncated = true;
            break;
        }
        *total_count += 1;

        let rel_path = if rel_prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", rel_prefix, item.name)
        };

        let children = if item.is_dir && depth_remaining > 1 {
            Some(
                Box::pin(read_workspace_dir_recursive(
                    &item.abs_path,
                    &rel_path,
                    depth_remaining - 1,
                    total_count,
                    truncated,
                ))
                .await,
            )
        } else {
            None
        };

        nodes.push(WorkspaceTreeNode {
            name: item.name,
            path: rel_path,
            is_dir: item.is_dir,
            size_bytes: item.size_bytes,
            children,
        });
    }

    nodes
}

pub async fn get_workspace_tree(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceTreeQuery>,
) -> Result<Json<WorkspaceTreeResponse>, (StatusCode, String)> {
    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let canonical_sandbox = tokio::fs::canonicalize(&sandbox)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Workspace sandbox does not exist".to_string()))?;

    let (target_dir, rel_prefix) = match query.path.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() && p != "." => {
            let rel = normalize_relative_path(p)?;
            let target = resolve_contained_path(&canonical_sandbox, &rel).await?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            (target, rel_str)
        }
        _ => (canonical_sandbox.clone(), String::new()),
    };

    if !target_dir.is_dir() {
        return Err((StatusCode::BAD_REQUEST, "Target path is not a directory".to_string()));
    }

    let max_depth = query.depth.unwrap_or(3).clamp(1, 5);
    let mut total_count = 0usize;
    let mut truncated = false;

    let entries = read_workspace_dir_recursive(
        &target_dir,
        &rel_prefix,
        max_depth,
        &mut total_count,
        &mut truncated,
    )
    .await;

    let root_display = canonical_sandbox.display().to_string();
    let clean_root = root_display
        .strip_prefix(r"\\?\")
        .unwrap_or(&root_display)
        .to_string();

    Ok(Json(WorkspaceTreeResponse {
        root: clean_root,
        relative_path: rel_prefix,
        truncated,
        entries,
    }))
}

#[derive(Deserialize)]
pub struct WorkspaceFileQuery {
    pub path: String,
    pub session_id: Option<String>,
}

#[derive(Serialize)]
pub struct WorkspaceFileResponse {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub truncated: bool,
    pub is_binary: bool,
    pub language: String,
    pub content: String,
}

pub async fn get_workspace_file(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceFileQuery>,
) -> Result<Json<WorkspaceFileResponse>, (StatusCode, String)> {
    let rel = normalize_relative_path(&query.path)?;
    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let target = resolve_contained_path(&sandbox, &rel).await?;

    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found".to_string()))?;
    if !meta.is_file() {
        return Err((StatusCode::BAD_REQUEST, "Target path is not a file".to_string()));
    }

    let size_bytes = meta.len();
    let file = tokio::fs::File::open(&target)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to open file: {}", e)))?;

    let mut buf = Vec::with_capacity((size_bytes as usize).min(MAX_WORKSPACE_FILE_BYTES + 1));
    let mut reader = file.take((MAX_WORKSPACE_FILE_BYTES + 1) as u64);
    reader
        .read_to_end(&mut buf)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read file: {}", e)))?;

    let truncated = buf.len() > MAX_WORKSPACE_FILE_BYTES;
    if truncated {
        buf.truncate(MAX_WORKSPACE_FILE_BYTES);
    }

    let is_binary = buf.iter().take(8192).any(|&b| b == 0);
    let content = if is_binary {
        String::new()
    } else {
        String::from_utf8_lossy(&buf).into_owned()
    };

    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let language = language_for_path(&target).to_string();

    Ok(Json(WorkspaceFileResponse {
        path: rel_str,
        name,
        size_bytes,
        truncated,
        is_binary,
        language,
        content,
    }))
}

// ============================================================================
// Customizations / Skills APIs
// ============================================================================

fn slugify_skill_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Skill name cannot be empty".to_string()));
    }
    let slug: String = trimmed
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = slug.trim_matches('-').to_string();
    if clean.is_empty() || clean == "." || clean == ".." {
        return Err((StatusCode::BAD_REQUEST, "Invalid skill name".to_string()));
    }
    Ok(clean)
}

pub async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SessionScopedQuery>,
) -> Json<Vec<SkillDefinition>> {
    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let conf = state.config.read().await;
    Json(conf.discover_skills_in(&sandbox))
}

#[derive(Deserialize)]
pub struct SaveSkillRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
    pub enabled: Option<bool>,
}

pub async fn save_skill(
    State(state): State<AppState>,
    Query(query): Query<SessionScopedQuery>,
    Json(payload): Json<SaveSkillRequest>,
) -> Result<Json<SkillDefinition>, (StatusCode, String)> {
    let name = payload.name.trim().to_string();
    let slug = slugify_skill_name(&name)?;
    let description = payload.description.trim().to_string();
    let body = payload.content.replace("\r\n", "\n");

    // Write to the session's workspace (active project) when known, else sandbox_root,
    // matching `list_skills` discovery.
    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let skills_dir = sandbox.join(".harness").join("skills");
    tokio::fs::create_dir_all(&skills_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create skills dir: {}", e)))?;

    let file_path = skills_dir.join(format!("{}.md", slug));
    let markdown = if body.trim_start().starts_with("---\n") {
        body.clone()
    } else {
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}",
            name, description, body
        )
    };

    tokio::fs::write(&file_path, markdown)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write skill file: {}", e)))?;

    let enabled = {
        let mut conf = state.config.write().await;
        if let Some(flag) = payload.enabled {
            let norm = name.to_lowercase();
            conf.disabled_skills.retain(|s| s.to_lowercase() != norm);
            if !flag {
                conf.disabled_skills.push(name.clone());
            }
            flag
        } else {
            let norm = name.to_lowercase();
            !conf.disabled_skills.iter().any(|s| s.to_lowercase() == norm)
        }
    };

    // Persist `disabled_skills` changes; previously in-memory only.
    state.persist_config().await;

    Ok(Json(SkillDefinition {
        name,
        description,
        content: body,
        path: format!(".harness/skills/{}.md", slug),
        enabled,
    }))
}

pub async fn delete_skill(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<SessionScopedQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let slug = slugify_skill_name(&name)?;
    let sandbox = resolve_sandbox(&state, query.session_id.as_deref()).await;
    let file_path = sandbox.join(".harness").join("skills").join(format!("{}.md", slug));

    if !file_path.is_file() {
        return Err((StatusCode::NOT_FOUND, "Skill file not found".to_string()));
    }

    tokio::fs::remove_file(&file_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete skill: {}", e)))?;

    {
        let mut conf = state.config.write().await;
        let norm = name.trim().to_lowercase();
        conf.disabled_skills
            .retain(|s| s.to_lowercase() != norm && s.to_lowercase() != slug);
    }

    state.persist_config().await;

    Ok(Json(serde_json::json!({ "status": "deleted", "name": name })))
}


/// `POST /api/db/optimize` — checkpoint WAL, ANALYZE, PRAGMA optimize, VACUUM.
pub async fn db_optimize(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let db = state
        .db
        .clone()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "No database in server-only mode".to_string()))?;
    // Run the blocking maintenance off the async worker threads.
    tokio::task::spawn_blocking(move || db.optimize())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("join error: {e}")))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "status": "optimized" })))
}

// ============================================================================
// Auto-update (GitHub Releases)
// ============================================================================

/// `GET /api/update/check` - compare the running version with the latest release.
///
/// The outcome is also recorded in the shared status so the UI reflects it.
pub async fn check_update(
    State(state): State<AppState>,
) -> Result<Json<crate::updater::UpdateInfo>, (StatusCode, String)> {
    let repo = state.config.read().await.update_repo.clone();
    let info = crate::updater::check(&repo, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    state
        .set_update_status(|s| {
            s.current_version = info.current_version.clone();
            if info.available {
                s.progress_phase(
                    crate::updater::UpdatePhase::Available,
                    format!("v{} available", info.latest_version),
                );
                s.target_version = info.latest_version.clone();
                s.html_url = info.html_url.clone();
                s.notes = info.notes.clone();
            } else {
                s.phase = crate::updater::UpdatePhase::UpToDate;
                s.target_version = info.current_version.clone();
                s.message = "Up to date".into();
                s.error = None;
                s.percent = None;
                s.downloaded_bytes = 0;
                s.total_bytes = 0;
                s.touch();
            }
        })
        .await;
    Ok(Json(info))
}

/// `GET /api/update/status` - live progress of the update pipeline.
pub async fn update_status_handler(State(state): State<AppState>) -> Json<crate::updater::UpdateStatus> {
    Json(state.update_status().await)
}

/// `POST /api/update/apply` - download (with progress), verify, install, restart.
///
/// Explicit user action, so it never depends on `auto_update_apply`.
pub async fn apply_update(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let repo = state.config.read().await.update_repo.clone();
    let info = crate::updater::check(&repo, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    if !info.available {
        return Ok(Json(serde_json::json!({
            "status": "up-to-date",
            "version": info.current_version
        })));
    }

    let latest = info.latest_version.clone();
    let from = info.current_version.clone();
    let st = std::sync::Arc::new(state.clone());
    tokio::spawn(async move {
        if let Err(e) = crate::auto_update::install(&st, &info).await {
            st.set_update_status(|s| s.fail(e)).await;
        }
    });
    Ok(Json(serde_json::json!({
        "status": "updating",
        "from": from,
        "to": latest
    })))
}

/// `GET /api/security/status` — non-sensitive summary of the security posture.
pub async fn security_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let conf = state.config.read().await;
    Json(serde_json::json!({
        "auth_enabled": !conf.auth_token.trim().is_empty()
            || std::env::var("HARNESS_AUTH_TOKEN").map(|v| !v.trim().is_empty()).unwrap_or(false),
        "bind_host": conf.host,
        "loopback_only": crate::web::security::is_loopback_bind(&conf.host),
        "secrets_encrypted": state.db.is_some(),
        "cors": "same-origin",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
// ============================================================================
// Automations (first-class scheduled agent runs)
// ============================================================================

/// `POST /api/automations` - create or update an automation.
pub async fn save_automation(
    State(state): State<AppState>,
    Json(mut automation): Json<crate::automation::Automation>,
) -> Result<Json<crate::automation::Automation>, (StatusCode, Json<serde_json::Value>)> {
    automation.name = automation.name.trim().to_string();
    automation.prompt = automation.prompt.trim().to_string();
    if automation.id.trim().is_empty() {
        automation.id = format!(
            "auto-{}",
            automation
                .name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .trim_matches('-')
                .to_string()
        );
        if automation.id == "auto-" {
            automation.id = format!("auto-{}", chrono::Utc::now().timestamp_millis());
        }
    }
    if automation.created_at.trim().is_empty() {
        automation.created_at = chrono::Local::now().to_rfc3339();
    }
    crate::automation::validate(&automation)
        .map_err(|e| json_api_error(StatusCode::BAD_REQUEST, e))?;

    {
        let mut conf = state.config.write().await;
        if let Some(existing) = conf.automations.iter_mut().find(|a| a.id == automation.id) {
            let last_run = existing.last_run_at.clone();
            let run_count = existing.run_count;
            *existing = automation.clone();
            existing.last_run_at = last_run;
            existing.run_count = run_count;
        } else {
            conf.automations.push(automation.clone());
        }
    }
    state.persist_config().await;
    Ok(Json(automation))
}

/// `DELETE /api/automations/:id`
pub async fn delete_automation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let removed = {
        let mut conf = state.config.write().await;
        let before = conf.automations.len();
        conf.automations.retain(|a| a.id != id);
        conf.automations.len() != before
    };
    if !removed {
        return Err(StatusCode::NOT_FOUND);
    }
    state.persist_config().await;
    Ok(Json(serde_json::json!({ "status": "deleted", "id": id })))
}

/// `GET /api/automations` - list automations with status, run counts and logs.
///
/// The UI renders the sidebar from this: `is_running` and the recent-run
/// counters let it show state without a second request per automation.
pub async fn list_automations(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Snapshot the in-flight runs once (one lock) so every row renders from the
    // same view of the world without a lock per automation.
    let running = state.all_running_run_ids().await;
    let conf = state.config.read().await;
    let items: Vec<serde_json::Value> = conf
        .automations
        .iter()
        .map(|a| {
            let runs = running.get(&a.id).cloned().unwrap_or_default();
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "description": a.description,
                "prompt": a.prompt,
                "project_id": a.project_id,
                "provider_id": a.provider_id,
                "model": a.model,
                "enabled": a.enabled,
                "trigger": a.trigger,
                "trigger_label": a.trigger.describe(),
                "created_at": a.created_at,
                "last_run_at": a.last_run_at,
                "run_count": a.run_count,
                "last_status": a.last_status,
                "is_running": !runs.is_empty(),
                "running_run_ids": runs,
                "next_due_at": a.next_due_at(chrono::Local::now())
            })
        })
        .collect();
    Json(serde_json::json!({ "automations": items }))
}

/// `PATCH /api/automations/:id` - edit an existing automation in place.
///
/// Only the fields present in the body change; everything else is preserved
/// (including `last_run_at` / `run_count`, which the scheduler owns).
pub async fn update_automation_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let edit = crate::automation::AutomationEdit::from_json(&body)
        .map_err(|e| json_api_error(StatusCode::BAD_REQUEST, e))?;

    let updated = {
        let mut conf = state.config.write().await;
        let Some(existing) = conf.automations.iter_mut().find(|a| a.id == id) else {
            return Err(json_api_error(
                StatusCode::NOT_FOUND,
                format!("Automation '{id}' not found"),
            ));
        };
        existing.apply_edit(&edit);
        existing.clone()
    };
    crate::automation::validate(&updated)
        .map_err(|e| json_api_error(StatusCode::BAD_REQUEST, e))?;

    // Re-validate inside the lock: a concurrent writer may have changed the base.
    {
        let conf = state.config.read().await;
        let stored = conf
            .automations
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| json_api_error(StatusCode::NOT_FOUND, format!("Automation '{id}' not found")))?;
        crate::automation::validate(stored)
            .map_err(|e| json_api_error(StatusCode::BAD_REQUEST, e))?;
    }

    state.reload_plugins().await;
    state.persist_config().await;
    Ok(Json(serde_json::json!({ "status": "updated", "automation": updated })))
}

/// `GET /api/automation/logs?automation_id=&limit=` - recent automation run logs.
pub async fn automation_logs_handler(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let items = match q.get("automation_id").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(id) => state.automation_logs_for(id, limit).await,
        None => state.automation_logs(limit).await,
    };
    Json(serde_json::json!({
        "logs": items,
        "count": items.len(),
        "automation_id": q.get("automation_id").cloned()
    }))
}

/// `GET /api/automations/:id/logs` - logs for one automation.
pub async fn automation_logs_for_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let items = state.automation_logs_for(&id, 100).await;
    Json(serde_json::json!({
        "automation_id": id,
        "logs": items,
        "count": items.len(),
        "summary": crate::automation::summarize_logs(&items)
    }))
}

/// `POST /api/automations/:id/run` - trigger immediately.
///
/// Returns the *run* id: an automation run is a log entry, not a conversation.
pub async fn run_automation_now(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let automation = {
        let conf = state.config.read().await;
        conf.automations
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Automation '{id}' not found")))?
    };
    let trigger = automation.trigger.clone();
    // A manual run is explicit even when the automation is triggered on a schedule.
    let _ = trigger;
    let run_id = state
        .run_automation_with_trigger(automation, crate::automation::AutomationRunTrigger::Manual)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::json!({
        "status": "started",
        "run_id": run_id,
        "automation_id": id
    })))
}

/// `POST /api/automations/:id/stop` - cancel a running automation run.
pub async fn stop_automation_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.stop_automation(&id).await {
        Ok(run_ids) => Ok(Json(serde_json::json!({
            "status": "stopping",
            "automation_id": id,
            "run_ids": run_ids
        }))),
        Err(e) => Err((StatusCode::CONFLICT, e)),
    }
}
// ============================================================================
// First-run onboarding
// ============================================================================

/// `GET /api/onboarding` — what the first-run wizard should show.
pub async fn onboarding_state(State(state): State<AppState>) -> Json<serde_json::Value> {
    let conf = state.config.read().await;
    let provider_configured = conf
        .custom_providers
        .iter()
        .any(|p| !p.api_key.trim().is_empty() || p.api_base.contains("localhost"));
    let project = conf
        .projects
        .iter()
        .find(|p| p.id == conf.active_project_id)
        .or_else(|| conf.projects.first());
    Json(serde_json::json!({
        "onboarded": conf.onboarded,
        "provider_configured": provider_configured,
        "providers": conf.custom_providers.len(),
        "project_name": project.map(|p| p.name.clone()).unwrap_or_default(),
        "project_path": project.map(|p| p.path.display().to_string()).unwrap_or_default(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize)]
pub struct CompleteOnboardingRequest {
    #[serde(default)]
    pub skipped: bool,
}

/// `POST /api/onboarding/complete` — mark the wizard as done.
pub async fn complete_onboarding(
    State(state): State<AppState>,
    Json(payload): Json<CompleteOnboardingRequest>,
) -> Json<serde_json::Value> {
    {
        let mut conf = state.config.write().await;
        conf.onboarded = true;
    }
    state.persist_config().await;
    Json(serde_json::json!({
        "status": "ok",
        "skipped": payload.skipped
    }))
}
// ============================================================================
// Filesystem rename / delete (used by the browser context menu)
// ============================================================================

#[derive(Deserialize)]
pub struct FsPathRequest {
    pub path: String,
}

#[derive(Deserialize)]
pub struct FsRenameRequest {
    pub path: String,
    pub new_name: String,
}

/// Refuse to touch a drive root or a home directory root.
fn is_protected_fs_root(path: &std::path::Path) -> bool {
    // A drive root has no parent (e.g. `C:\`) and `components().count() <= 1` on Unix.
    let has_no_parent = path.parent().is_none();
    let looks_like_drive_root = path
        .to_str()
        .map(|s| s.len() <= 3 && (s.ends_with(":\\") || s.ends_with(":/")))
        .unwrap_or(false);
    if has_no_parent || looks_like_drive_root {
        return true;
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(std::path::PathBuf::from);
    if let Some(h) = home {
        if let (Ok(a), Ok(b)) = (path.canonicalize(), h.canonicalize()) {
            return a == b;
        }
    }
    false
}

/// `POST /api/filesystem/rename` — rename a file or folder in place.
pub async fn rename_path(
    Json(payload): Json<FsRenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let raw = payload.path.trim();
    if raw.is_empty() {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Path is empty."));
    }
    let clean_name = payload.new_name.trim();
    if clean_name.is_empty()
        || clean_name.contains('/')
        || clean_name.contains('\\')
        || clean_name.contains(':')
        || clean_name == ".."
        || clean_name == "."
    {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Invalid name."));
    }
    let src = std::path::PathBuf::from(raw);
    if !src.exists() {
        return Err(json_api_error(StatusCode::NOT_FOUND, "Path does not exist."));
    }
    if is_protected_fs_root(&src) {
        return Err(json_api_error(
            StatusCode::BAD_REQUEST,
            "Refusing to rename a drive root or home directory.",
        ));
    }
    let dest = src.with_file_name(clean_name);
    if dest.exists() {
        return Err(json_api_error(
            StatusCode::CONFLICT,
            format!("'{}' already exists.", clean_name),
        ));
    }
    tokio::fs::rename(&src, &dest)
        .await
        .map_err(|e| json_api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("Rename failed: {e}")))?;
    Ok(Json(serde_json::json!({
        "status": "renamed",
        "path": normalize_display_path(&dest)
    })))
}

/// `POST /api/filesystem/delete` — delete a file, or a folder recursively.
pub async fn delete_path(
    Json(payload): Json<FsPathRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let raw = payload.path.trim();
    if raw.is_empty() {
        return Err(json_api_error(StatusCode::BAD_REQUEST, "Path is empty."));
    }
    let target = std::path::PathBuf::from(raw);
    if !target.exists() {
        return Err(json_api_error(StatusCode::NOT_FOUND, "Path does not exist."));
    }
    if is_protected_fs_root(&target) {
        return Err(json_api_error(
            StatusCode::BAD_REQUEST,
            "Refusing to delete a drive root or home directory.",
        ));
    }
    let meta = tokio::fs::metadata(&target)
        .await
        .map_err(|e| json_api_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let result = if meta.is_dir() {
        tokio::fs::remove_dir_all(&target).await
    } else {
        tokio::fs::remove_file(&target).await
    };
    result
        .map_err(|e| json_api_error(StatusCode::INTERNAL_SERVER_ERROR, format!("Delete failed: {e}")))?;
    Ok(Json(serde_json::json!({ "status": "deleted", "path": raw })))
}