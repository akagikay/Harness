use async_trait::async_trait;
use futures_util::future::join_all;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;
use tokio::sync::{mpsc, Notify};

use crate::config::PermissionMode;
use crate::engine::events::HarnessEvent;
use crate::engine::loop_runner::AgentLoopRunner;
use crate::engine::swarm::SwarmBoard;
use crate::models::{
    google_search_client::{
        is_google_search_model, GoogleSearchClient, EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID,
    },
    mock_client::MockClient,
    openai_client::OpenAiClient,
    ModelAdapter,
};

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::{SessionToolContext, ToolPlugin};

static SUBAGENT_SEQ: AtomicUsize = AtomicUsize::new(1);
static ARTIFACT_SEQ: AtomicUsize = AtomicUsize::new(1);

const MAX_SWARM_DIGEST_CHARS: usize = 220;
const LARGE_OUTPUT_ARTIFACT_THRESHOLD: usize = 240;

async fn emit_parent_event(ctx: &SessionToolContext, event: HarnessEvent) {
    if let Some(ref tx) = ctx.event_tx {
        let _ = tx.send(event).await;
    } else if let Some(ref st) = ctx.state {
        st.emit_session_event(&ctx.session_id, event).await;
    }
}

async fn resolve_swarm(ctx: &SessionToolContext) -> Option<SwarmBoard> {
    if let Some(ref swarm) = ctx.swarm {
        return Some(swarm.clone());
    }
    if let Some(ref st) = ctx.state {
        return Some(st.get_or_create_swarm(&ctx.session_id).await);
    }
    None
}

fn sanitize_slug(raw: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch == ' ') && !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.chars().take(32).collect()
    }
}

fn summarize_tool_args(args: &Value) -> String {
    let s = match args {
        Value::Object(map) => {
            if let Some(cmd) = map.get("command").and_then(|v| v.as_str()) {
                cmd.to_string()
            } else if let Some(path) = map.get("path").and_then(|v| v.as_str()) {
                path.to_string()
            } else if let Some(url) = map.get("url").and_then(|v| v.as_str()) {
                url.to_string()
            } else if let Some(action) = map.get("action").and_then(|v| v.as_str()) {
                action.to_string()
            } else if let Some(q) = map.get("query").and_then(|v| v.as_str()) {
                q.to_string()
            } else {
                args.to_string()
            }
        }
        _ => args.to_string(),
    };
    truncate_chars(&s, 100)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", head)
    } else {
        head
    }
}

/// Collapse newlines and extra whitespace into a single compact line capped at `max_chars`.
fn compact_digest(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = collapsed.chars().count();
    if count <= max_chars {
        return collapsed;
    }
    if max_chars <= 3 {
        return collapsed.chars().take(max_chars).collect();
    }
    let head: String = collapsed.chars().take(max_chars - 3).collect();
    format!("{}...", head)
}

fn format_board_digest(summary: &str, artifact_path: Option<&str>) -> String {
    if let Some(path) = artifact_path {
        let suffix = format!(" [full: {}]", path);
        let budget = MAX_SWARM_DIGEST_CHARS.saturating_sub(suffix.chars().count());
        format!("{}{}", compact_digest(summary, budget), suffix)
    } else {
        compact_digest(summary, MAX_SWARM_DIGEST_CHARS)
    }
}

async fn record_subagent_output_artifact(
    ctx: &SessionToolContext,
    sandbox: &Path,
    swarm: Option<&SwarmBoard>,
    subagent_id: &str,
    role: &str,
    content: &str,
) -> Option<String> {
    let artifacts_dir = sandbox.join(".harness_artifacts");
    if fs::create_dir_all(&artifacts_dir).await.is_err() {
        return None;
    }

    let safe_id = sanitize_slug(subagent_id, "subagent");
    let filename = format!("{}_output.md", safe_id);
    let full_path = artifacts_dir.join(&filename);
    let body = format!("# Subagent Output: {} ({})\n\n{}\n", subagent_id, role, content);
    if fs::write(&full_path, body).await.is_err() {
        return None;
    }

    let rel_path = format!(".harness_artifacts/{}", filename);
    let seq = ARTIFACT_SEQ.fetch_add(1, Ordering::Relaxed);
    let art_id = format!("art_{}_{}", safe_id, seq);
    let title = format!("Subagent {} ({}) Output", subagent_id, role);
    let preview = compact_digest(content, 200);

    if let Some(sw) = swarm {
        let record = sw
            .add_artifact(art_id, title, "report", &rel_path, &preview)
            .await;
        emit_parent_event(ctx, record.to_event()).await;
    } else {
        emit_parent_event(
            ctx,
            HarnessEvent::ArtifactCreated {
                id: art_id,
                title,
                kind: "report".to_string(),
                path: rel_path.clone(),
                preview,
            },
        )
        .await;
    }

    Some(rel_path)
}

#[derive(Clone)]
struct SubagentSpec {
    role: String,
    prompt: String,
    task_digest: String,
    read_only: bool,
    model: Option<String>,
    provider_id: Option<String>,
    thinking_effort: Option<String>,
    server_id: Option<String>,
    job_tag: Option<String>,
    specialist: Option<String>,
}

struct SubagentOutcome {
    subagent_id: String,
    role: String,
    success: bool,
    summary: String,
    duration_secs: f64,
}

fn resolve_subagent_profile<'a>(
    ctx: &'a SessionToolContext,
    role: &str,
    specialist: Option<&str>,
) -> Option<&'a crate::config::SubagentProfileConfig> {
    if !ctx.config.orchestrator.subagent_mode.eq_ignore_ascii_case("multi") {
        return None;
    }
    let profiles = &ctx.config.orchestrator.subagent_profiles;
    if profiles.is_empty() {
        return None;
    }
    if let Some(sp) = specialist.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(hit) = profiles
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(sp) || p.name.eq_ignore_ascii_case(sp))
        {
            return Some(hit);
        }
    }
    let role_trim = role.trim().to_ascii_lowercase();
    profiles.iter().find(|p| {
        p.id.eq_ignore_ascii_case(&role_trim)
            || p.name.eq_ignore_ascii_case(&role_trim)
            || (!p.id.is_empty() && role_trim.contains(&p.id.to_ascii_lowercase()))
            || (!p.name.is_empty() && role_trim.contains(&p.name.to_ascii_lowercase()))
    })
}

fn allocate_subagent_id(role: &str) -> String {
    let seq = SUBAGENT_SEQ.fetch_add(1, Ordering::Relaxed);
    let slug = sanitize_slug(role, "worker");
    format!("sub_{}_{}", slug, seq)
}

fn resolve_effective_model_name(spec: &SubagentSpec, ctx: &SessionToolContext) -> String {
    if ctx.config.mock_mode {
        return "mock".to_string();
    }
    if let Some(ref m) = spec.model {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            if is_google_search_model(trimmed) {
                return EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID.to_string();
            }
            return trimmed.to_string();
        }
    }
    let orch = &ctx.config.orchestrator;
    let orch_sub = orch.subagent_model.trim();
    // "inherit"/"follow" means: use the orchestrator model, then the session model.
    if crate::config::is_inherit_sentinel(orch_sub) {
        let orch_model = orch.model.trim();
        if !orch_model.is_empty() {
            return if is_google_search_model(orch_model) {
                EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID.to_string()
            } else {
                orch_model.to_string()
            };
        }
        return ctx.config.model.clone();
    }
    if !orch_sub.is_empty() {
        if is_google_search_model(orch_sub) {
            return EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID.to_string();
        }
        return orch_sub.to_string();
    }
    // No subagent override: follow the orchestrator model when it has one.
    let orch_model = orch.model.trim();
    if !orch_model.is_empty() {
        return if is_google_search_model(orch_model) {
            EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID.to_string()
        } else {
            orch_model.to_string()
        };
    }
    ctx.config.model.clone()
}

fn resolve_target_server<'a>(
    spec: &SubagentSpec,
    ctx: &'a SessionToolContext,
) -> Option<&'a crate::config::ServerNodeConfig> {
    let raw = spec
        .server_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let s = ctx.config.orchestrator.subagent_server_id.trim();
            (!s.is_empty()).then_some(s)
        })?;

    if raw.eq_ignore_ascii_case("local") {
        return None;
    }

    let remote_enabled: Vec<&crate::config::ServerNodeConfig> = ctx
        .config
        .servers
        .iter()
        .filter(|s| {
            s.enabled
                && (s.url.starts_with("http://") || s.url.starts_with("https://"))
                && !s.id.eq_ignore_ascii_case(&ctx.config.server_identity)
        })
        .collect();

    if remote_enabled.is_empty() {
        return None;
    }

    if raw.eq_ignore_ascii_case("auto") || raw.eq_ignore_ascii_case("round_robin") {
        let idx = SUBAGENT_SEQ.load(Ordering::Relaxed) % remote_enabled.len();
        return Some(remote_enabled[idx]);
    }

    remote_enabled.into_iter().find(|s| {
        s.id.eq_ignore_ascii_case(raw) || s.name.eq_ignore_ascii_case(raw)
    })
}

async fn register_and_announce_subagent(
    ctx: &SessionToolContext,
    swarm: Option<&SwarmBoard>,
    subagent_id: &str,
    spec: &SubagentSpec,
) -> Arc<Notify> {
    let model_name = resolve_effective_model_name(spec, ctx);
    let target_server = resolve_target_server(spec, ctx).map(|s| s.id.clone());
    let cancel = if let Some(sw) = swarm {
        let handle = sw
            .register_subagent_with_tags(
                subagent_id.to_string(),
                spec.role.clone(),
                spec.task_digest.clone(),
                Some(ctx.agent_id.clone()),
                spec.job_tag.clone(),
                spec.specialist.clone(),
            )
            .await;
        sw.set_subagent_model(subagent_id, Some(model_name)).await;
        if let Some(ref srv_id) = target_server {
            sw.set_subagent_server(subagent_id, Some(srv_id.clone())).await;
        }

        let mut tag_parts = Vec::new();
        if let Some(ref jt) = spec.job_tag {
            tag_parts.push(format!("job:{}", jt));
        }
        if let Some(ref sp) = spec.specialist {
            tag_parts.push(format!("specialist:{}", sp));
        }
        let tag_label = if tag_parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", tag_parts.join(", "))
        };
        let assign_prefix = format!("Assigned [{}]{}: ", spec.role, tag_label);
        let assign_budget = MAX_SWARM_DIGEST_CHARS.saturating_sub(assign_prefix.chars().count());
        let assign_content = format!(
            "{}{}",
            assign_prefix,
            compact_digest(&spec.task_digest, assign_budget)
        );
        let assign_msg = sw
            .post_message_with_tags(
                ctx.agent_id.clone(),
                Some(subagent_id.to_string()),
                assign_content,
                spec.job_tag.clone(),
                spec.specialist.clone(),
            )
            .await;
        emit_parent_event(ctx, assign_msg.to_event()).await;
        handle.cancel_notify
    } else {
        Arc::new(Notify::new())
    };

    emit_parent_event(
        ctx,
        HarnessEvent::SubagentSpawned {
            subagent_id: subagent_id.to_string(),
            role: spec.role.clone(),
            task: spec.task_digest.clone(),
            parent_id: Some(ctx.agent_id.clone()),
        },
    )
    .await;

    cancel
}

async fn run_subagent_task(
    ctx: SessionToolContext,
    sandbox: PathBuf,
    spec: SubagentSpec,
    post_to_board_on_finish: bool,
    pre_handle: Option<(String, Arc<Notify>)>,
) -> SubagentOutcome {
    let started = Instant::now();
    let swarm = resolve_swarm(&ctx).await;
    let artifact_sandbox = sandbox.clone();

    let (subagent_id, child_cancel) = match pre_handle {
        Some((id, cancel)) => (id, cancel),
        None => {
            let id = allocate_subagent_id(&spec.role);
            let cancel = register_and_announce_subagent(&ctx, swarm.as_ref(), &id, &spec).await;
            (id, cancel)
        }
    };

    // Propagate parent cancellation to this child subagent until completion.
    let done_notify = Arc::new(Notify::new());
    {
        let parent_cancel = ctx.cancel_notify.clone();
        let child_cancel_fwd = child_cancel.clone();
        let done = done_notify.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = parent_cancel.notified() => {
                    child_cancel_fwd.notify_one();
                }
                _ = done.notified() => {}
            }
        });
    }

    let effective_model = resolve_effective_model_name(&spec, &ctx);
    let effective_provider = spec
        .provider_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let s = ctx.config.orchestrator.subagent_provider_id.trim();
            (!s.is_empty()).then_some(s)
        });
    // Resolve the subagent reasoning effort. A per-call effort wins; otherwise the
    // orchestrator setting applies, where `inherit` copies the orchestrator level and
    // `step_down` runs one notch cheaper (high -> medium -> low -> ...).
    let orch_cfg = &ctx.config.orchestrator;
    let orchestrator_effort = if orch_cfg.thinking_effort.trim().is_empty() {
        ctx.config.reasoning_effort.trim()
    } else {
        orch_cfg.thinking_effort.trim()
    };
    let configured_effort = spec
        .thinking_effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| orch_cfg.subagent_thinking_effort.clone());
    let resolved_effort = crate::config::resolve_subagent_effort(
        &configured_effort,
        orchestrator_effort,
        "medium",
    );
    let effective_effort = Some(resolved_effort.as_str());

    let mut child_config = ctx
        .config
        .with_model_override(&effective_model, effective_provider, effective_effort);
    child_config.sandbox_root = sandbox.clone();
    child_config.max_iterations = 0;
    child_config.knowledge_only = ctx.config.knowledge_only;
    if spec.read_only || ctx.config.permissions.mode == PermissionMode::ReadOnly || child_config.knowledge_only {
        child_config.permissions.mode = PermissionMode::ReadOnly;
    }

    let remote_target = resolve_target_server(&spec, &ctx).cloned();

    let (child_tx, mut child_rx) = mpsc::channel::<HarnessEvent>(100);
    let runner_handle: tokio::task::JoinHandle<Result<String, String>> = if let Some(ref srv) = remote_target {
        let srv_clone = srv.clone();
        let req_config = child_config.clone();
        let run_id = format!("run_{}_{}", subagent_id, chrono::Utc::now().timestamp_millis());
        let sess_id = ctx.session_id.clone();
        let prompt = spec.prompt.clone();
        let ag_id = subagent_id.clone();
        let ag_role = spec.role.clone();
        let parent = ctx.agent_id.clone();
        let depth = ctx.depth + 1;
        let read_only = spec.read_only || req_config.permissions.mode == PermissionMode::ReadOnly;
        let use_mock = req_config.mock_mode;
        let cancel_for_remote = child_cancel.clone();
        let state_for_perm = ctx.state.clone();

        tokio::spawn(async move {
            crate::remote::execute_remote_server_run(
                &srv_clone,
                run_id,
                sess_id,
                prompt,
                Vec::new(),
                Vec::new(),
                req_config,
                Some(ag_id),
                Some(ag_role),
                Some(parent),
                depth,
                read_only,
                use_mock,
                cancel_for_remote,
                state_for_perm,
                child_tx,
            )
            .await
            .map(|(ans, _)| ans)
        })
    } else {
        let child_model: Arc<dyn ModelAdapter> = if child_config.mock_mode {
            Arc::new(MockClient)
        } else if is_google_search_model(&child_config.model) {
            Arc::new(GoogleSearchClient::new())
        } else {
            Arc::new(OpenAiClient::new(child_config.clone()))
        };

        let runner = AgentLoopRunner::new(
            ctx.session_id.clone(),
            spec.prompt.clone(),
            child_config,
            ctx.registry.clone(),
            child_model,
            ctx.state.clone(),
        )
        .with_sandbox(sandbox)
        .with_cancel_notify(child_cancel)
        .with_subagent(
            subagent_id.clone(),
            spec.role.clone(),
            Some(ctx.agent_id.clone()),
            ctx.depth + 1,
        );

        tokio::spawn(async move { runner.run(child_tx).await })
    };

    let mut thought_buf = String::new();
    let mut msg_buf = String::new();
    let mut last_error_msg: Option<String> = None;
    let mut was_stopped = false;
    let mut had_error = false;

    while let Some(evt) = child_rx.recv().await {
        match evt {
            HarnessEvent::ThoughtDelta { delta } => {
                thought_buf.push_str(&delta);
                if thought_buf.len() >= 180 {
                    let chunk = std::mem::take(&mut thought_buf);
                    emit_parent_event(
                        &ctx,
                        HarnessEvent::SubagentUpdate {
                            subagent_id: subagent_id.clone(),
                            role: spec.role.clone(),
                            status: "running".to_string(),
                            delta_kind: "thought".to_string(),
                            content: chunk,
                        },
                    )
                    .await;
                }
            }
            HarnessEvent::MessageDelta { delta } => {
                msg_buf.push_str(&delta);
                if msg_buf.len() >= 180 {
                    let chunk = std::mem::take(&mut msg_buf);
                    emit_parent_event(
                        &ctx,
                        HarnessEvent::SubagentUpdate {
                            subagent_id: subagent_id.clone(),
                            role: spec.role.clone(),
                            status: "running".to_string(),
                            delta_kind: "message".to_string(),
                            content: chunk,
                        },
                    )
                    .await;
                }
            }
            HarnessEvent::ToolCallStarted { tool, args, .. } => {
                if !thought_buf.is_empty() {
                    let chunk = std::mem::take(&mut thought_buf);
                    emit_parent_event(
                        &ctx,
                        HarnessEvent::SubagentUpdate {
                            subagent_id: subagent_id.clone(),
                            role: spec.role.clone(),
                            status: "running".to_string(),
                            delta_kind: "thought".to_string(),
                            content: chunk,
                        },
                    )
                    .await;
                }
                let content = compact_digest(
                    &format!("Running {} ({})", tool, summarize_tool_args(&args)),
                    MAX_SWARM_DIGEST_CHARS,
                );
                if let Some(ref sw) = swarm {
                    sw.update_subagent_status(&subagent_id, "running", Some(content.clone()))
                        .await;
                }
                emit_parent_event(
                    &ctx,
                    HarnessEvent::SubagentUpdate {
                        subagent_id: subagent_id.clone(),
                        role: spec.role.clone(),
                        status: "running".to_string(),
                        delta_kind: "tool_start".to_string(),
                        content,
                    },
                )
                .await;
            }
            HarnessEvent::ToolCallFinished {
                tool,
                success,
                duration_ms,
                ..
            } => {
                let content = format!(
                    "{} finished ({}ms, {})",
                    tool,
                    duration_ms,
                    if success { "ok" } else { "error" }
                );
                emit_parent_event(
                    &ctx,
                    HarnessEvent::SubagentUpdate {
                        subagent_id: subagent_id.clone(),
                        role: spec.role.clone(),
                        status: "running".to_string(),
                        delta_kind: "tool_done".to_string(),
                        content,
                    },
                )
                .await;
            }
            HarnessEvent::Warning { message } => {
                if message.contains("stopped by user") {
                    was_stopped = true;
                }
                let status = if had_error {
                    "failed"
                } else if was_stopped {
                    "stopped"
                } else {
                    "running"
                };
                emit_parent_event(
                    &ctx,
                    HarnessEvent::SubagentUpdate {
                        subagent_id: subagent_id.clone(),
                        role: spec.role.clone(),
                        status: status.to_string(),
                        delta_kind: "warning".to_string(),
                        content: message,
                    },
                )
                .await;
            }
            HarnessEvent::Error { message, .. } => {
                let pre_err_status = if was_stopped { "stopped" } else { "running" };
                if !thought_buf.is_empty() {
                    let chunk = std::mem::take(&mut thought_buf);
                    emit_parent_event(
                        &ctx,
                        HarnessEvent::SubagentUpdate {
                            subagent_id: subagent_id.clone(),
                            role: spec.role.clone(),
                            status: pre_err_status.to_string(),
                            delta_kind: "thought".to_string(),
                            content: chunk,
                        },
                    )
                    .await;
                }
                if !msg_buf.is_empty() {
                    let chunk = std::mem::take(&mut msg_buf);
                    emit_parent_event(
                        &ctx,
                        HarnessEvent::SubagentUpdate {
                            subagent_id: subagent_id.clone(),
                            role: spec.role.clone(),
                            status: pre_err_status.to_string(),
                            delta_kind: "message".to_string(),
                            content: chunk,
                        },
                    )
                    .await;
                }
                had_error = true;
                last_error_msg = Some(message.clone());
                emit_parent_event(
                    &ctx,
                    HarnessEvent::SubagentUpdate {
                        subagent_id: subagent_id.clone(),
                        role: spec.role.clone(),
                        status: "failed".to_string(),
                        delta_kind: "error".to_string(),
                        content: message,
                    },
                )
                .await;
            }
            // Bubble high-level collaborative events directly to the parent stream.
            ev @ (HarnessEvent::ArtifactCreated { .. }
            | HarnessEvent::BrowserSnapshot { .. }
            | HarnessEvent::SwarmBoardMessage { .. }
            | HarnessEvent::PermissionRequested { .. }
            | HarnessEvent::PermissionDecided { .. }
            | HarnessEvent::SubagentSpawned { .. }
            | HarnessEvent::SubagentUpdate { .. }
            | HarnessEvent::SubagentCompleted { .. }) => {
                emit_parent_event(&ctx, ev).await;
            }
            _ => {}
        }
    }

    let flush_status = if had_error {
        "failed"
    } else if was_stopped {
        "stopped"
    } else {
        "running"
    };
    if !thought_buf.is_empty() {
        emit_parent_event(
            &ctx,
            HarnessEvent::SubagentUpdate {
                subagent_id: subagent_id.clone(),
                role: spec.role.clone(),
                status: flush_status.to_string(),
                delta_kind: "thought".to_string(),
                content: thought_buf,
            },
        )
        .await;
    }
    if !msg_buf.is_empty() {
        emit_parent_event(
            &ctx,
            HarnessEvent::SubagentUpdate {
                subagent_id: subagent_id.clone(),
                role: spec.role.clone(),
                status: flush_status.to_string(),
                delta_kind: "message".to_string(),
                content: msg_buf,
            },
        )
        .await;
    }

    let join_res = runner_handle.await;
    done_notify.notify_waiters();

    let duration_secs = started.elapsed().as_secs_f64();
    let (success, status_str, full_summary) = match join_res {
        Ok(Ok(ans)) => {
            if was_stopped || ans == "(Session stopped by user)" {
                (false, "stopped", "(Subagent stopped by user)".to_string())
            } else if had_error && ans.trim().is_empty() {
                (
                    false,
                    "failed",
                    last_error_msg.unwrap_or_else(|| "Subagent encountered an error.".to_string()),
                )
            } else {
                (true, "completed", ans)
            }
        }
        Ok(Err(err)) => (false, "failed", format!("Subagent error: {}", err)),
        Err(join_err) => (false, "failed", format!("Subagent task panicked: {}", join_err)),
    };

    let artifact_path = if full_summary.chars().count() > LARGE_OUTPUT_ARTIFACT_THRESHOLD {
        record_subagent_output_artifact(
            &ctx,
            &artifact_sandbox,
            swarm.as_ref(),
            &subagent_id,
            &spec.role,
            &full_summary,
        )
        .await
    } else {
        None
    };

    let compact_summary = format_board_digest(&full_summary, artifact_path.as_deref());

    if let Some(ref sw) = swarm {
        sw.update_subagent_status(&subagent_id, status_str, Some(compact_summary.clone()))
            .await;
        if post_to_board_on_finish {
            let prefix = format!("[{}] ({}, {:.1}s): ", spec.role, status_str, duration_secs);
            let suffix = artifact_path
                .as_deref()
                .map(|p| format!(" [full: {}]", p))
                .unwrap_or_default();
            let body_budget = MAX_SWARM_DIGEST_CHARS
                .saturating_sub(prefix.chars().count())
                .saturating_sub(suffix.chars().count());
            let board_content = format!(
                "{}{}{}",
                prefix,
                compact_digest(&full_summary, body_budget),
                suffix
            );
            let msg = sw
                .post_message_with_tags(
                    subagent_id.clone(),
                    Some(ctx.agent_id.clone()),
                    board_content,
                    spec.job_tag.clone(),
                    spec.specialist.clone(),
                )
                .await;
            emit_parent_event(&ctx, msg.to_event()).await;
        }
    }

    emit_parent_event(
        &ctx,
        HarnessEvent::SubagentCompleted {
            subagent_id: subagent_id.clone(),
            role: spec.role.clone(),
            success,
            summary: compact_summary,
            duration_secs,
        },
    )
    .await;

    SubagentOutcome {
        subagent_id,
        role: spec.role,
        success,
        summary: full_summary,
        duration_secs,
    }
}

async fn enrich_subagent_prompt(
    swarm: Option<&SwarmBoard>,
    base_prompt: String,
    priority_files: &[String],
    include_board: bool,
) -> String {
    let mut sections = Vec::new();
    if !priority_files.is_empty() {
        sections.push(format!(
            "[Priority Files To Inspect]\n{}",
            priority_files
                .iter()
                .map(|p| format!("- {}", p))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if include_board {
        if let Some(sw) = swarm {
            let msgs = sw.all_messages().await;
            if !msgs.is_empty() {
                let start = msgs.len().saturating_sub(6);
                let recent: Vec<String> = msgs[start..]
                    .iter()
                    .map(|m| {
                        let tgt = m
                            .target
                            .as_deref()
                            .map(|t| format!(" -> @{}", t))
                            .unwrap_or_default();
                        format!("#{} [{}{}]: {}", m.seq, m.sender, tgt, compact_digest(&m.content, 160))
                    })
                    .collect();
                sections.push(format!("[Recent Swarm Board Context]\n{}", recent.join("\n")));
            }
        }
    }
    if sections.is_empty() {
        base_prompt
    } else {
        format!("{}\n\n{}", base_prompt, sections.join("\n\n"))
    }
}

fn extract_string_array(val: &Value, key: &str) -> Vec<String> {
    val.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn extract_opt_str(val: &Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn extract_job_tag(val: &Value) -> Option<String> {
    extract_opt_str(val, "job_tag").or_else(|| extract_opt_str(val, "job_id"))
}

async fn resolve_subagent_tags(
    swarm: Option<&SwarmBoard>,
    role: &str,
    raw_prompt: &str,
    explicit_read_only: Option<bool>,
    raw_job_tag: Option<String>,
    raw_specialist: Option<String>,
) -> (String, bool, Option<String>, Option<String>) {
    let Some(sw) = swarm else {
        return (
            raw_prompt.to_string(),
            explicit_read_only.unwrap_or(false),
            raw_job_tag,
            raw_specialist,
        );
    };

    let resolved_job = if let Some(jt) = raw_job_tag {
        if let Some(existing) = sw.get_job(&jt).await {
            if existing.status == "open" {
                let updated = sw
                    .create_or_update_job(
                        existing.id,
                        existing.title,
                        existing.description,
                        Some("in_progress".to_string()),
                    )
                    .await;
                Some(updated.id)
            } else {
                Some(existing.id)
            }
        } else {
            let created = sw
                .create_or_update_job(
                    jt.clone(),
                    jt,
                    format!("Auto-created job for {}", role),
                    Some("in_progress".to_string()),
                )
                .await;
            Some(created.id)
        }
    } else {
        None
    };

    let (effective_prompt, effective_read_only, resolved_specialist) =
        if let Some(sp) = raw_specialist {
            let spec_tag = if let Some(existing) = sw.get_specialist(&sp).await {
                existing
            } else {
                sw.create_or_update_specialist(
                    sp.clone(),
                    sp.clone(),
                    format!("Specialist focus: {}", sp),
                    explicit_read_only.unwrap_or(false),
                )
                .await
            };
            let ro = explicit_read_only.unwrap_or(spec_tag.default_read_only);
            let prompt_with_specialty = if spec_tag.specialty.trim().is_empty() {
                format!("[Specialist: {}]\n\n{}", spec_tag.name, raw_prompt)
            } else {
                format!(
                    "[Specialist: {}]\nSpecialty Instructions: {}\n\n{}",
                    spec_tag.name,
                    spec_tag.specialty.trim(),
                    raw_prompt
                )
            };
            (prompt_with_specialty, ro, Some(spec_tag.id))
        } else {
            (
                raw_prompt.to_string(),
                explicit_read_only.unwrap_or(false),
                None,
            )
        };

    (
        effective_prompt,
        effective_read_only,
        resolved_job,
        resolved_specialist,
    )
}

// ----------------------------------------------------------------------------
// InvokeSubagentPlugin
// ----------------------------------------------------------------------------
pub struct InvokeSubagentPlugin;

#[async_trait]
impl ToolPlugin for InvokeSubagentPlugin {
    fn name(&self) -> &'static str {
        "invoke_subagent"
    }

    fn description(&self) -> &'static str {
        "Spawn one or more autonomous child subagents (single or parallel batch via `tasks`) with specialized roles, optional `(job)` and `(specialist)` tags, optional read-only mode, and optional model override. Use for parallel multi-file implementation, independent research, or verification."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "swarm".to_string(),
            permission_level: ToolPermission::Meta,
            fields: vec![
                ToolFieldSpec::optional_string(
                    "role",
                    "Short role name for a single subagent (e.g., 'researcher', 'frontend', 'verifier').",
                )
                .with_error_hint("Provide a non-empty 'role' alongside 'prompt' for a single subagent, or pass 'tasks'."),
                ToolFieldSpec::optional_string(
                    "prompt",
                    "Detailed task instructions and acceptance criteria for the single subagent.",
                )
                .with_error_hint("Provide a non-empty 'prompt' alongside 'role' for a single subagent, or pass 'tasks'."),
                ToolFieldSpec::optional_bool(
                    "read_only",
                    "If true, restricts the subagent to read-only inspection (no bash or file edits). If omitted and a specialist is set, honors the specialist's default_read_only.",
                )
                .with_error_hint("Pass a boolean true or false for read_only."),
                ToolFieldSpec::optional_string(
                    "model",
                    "Optional model name override for this subagent.",
                )
                .with_error_hint("Provide a model identifier string or omit to inherit the configured subagent/parent model."),
                ToolFieldSpec::optional_string(
                    "provider_id",
                    "Optional provider ID override for this subagent (e.g. 'nvidia', 'deepseek', 'openrouter').",
                )
                .with_error_hint("Provide a configured provider ID string."),
                ToolFieldSpec::optional_string(
                    "thinking_effort",
                    "Optional thinking/reasoning effort level override ('low', 'medium', 'high', 'minimal', 'none').",
                )
                .with_error_hint("Provide a valid thinking effort string."),
                ToolFieldSpec::optional_string(
                    "server_id",
                    "Optional execution server ID or name to spawn this subagent on (e.g. 'server-2', 'server-3', 'auto', or 'local').",
                )
                .with_error_hint("Provide a configured server ID, 'auto', or 'local'."),
                ToolFieldSpec::optional_string(
                    "job_tag",
                    "Optional (job) tag ID to associate this subagent or batch with (auto-created if missing).",
                )
                .with_error_hint("Provide a non-empty job tag identifier string."),
                ToolFieldSpec::optional_string(
                    "specialist",
                    "Optional (member specialist) tag ID or name to apply specialty instructions and default read-only mode (auto-created if missing).",
                )
                .with_error_hint("Provide a non-empty specialist tag ID or name."),
                ToolFieldSpec::optional_bool(
                    "wait",
                    "Whether to block waiting for completion (default false — background swarm workers execute asynchronously). Pass true only if you strictly need a synchronous call.",
                )
                .with_default(json!(false))
                .with_error_hint("Pass false (default) for asynchronous background execution, or true to wait synchronously."),
                ToolFieldSpec::optional_array(
                    "files",
                    "Optional list of workspace file paths to highlight in the worker prompt.",
                )
                .with_error_hint("Pass an array of workspace relative file paths."),
                ToolFieldSpec::optional_bool(
                    "include_board",
                    "Include a compact digest of the last 6 swarm board messages in the worker prompt (default true).",
                )
                .with_default(json!(true))
                .with_error_hint("Pass true or false for include_board."),
                ToolFieldSpec::optional_array(
                    "tasks",
                    "Optional batch of subagents to execute concurrently in parallel.",
                )
                .with_error_hint("Provide a non-empty array of {role, prompt, read_only?, model?, job_tag?, specialist?, files?} objects (maximum 8)."),
            ],
            expected_output: OutputContract::new(
                "markdown",
                "Per-subagent status, duration, and summary report, or a list of spawned background roles when wait=false.",
                "### Subagent `sub_verifier_1` (role: verifier, status: completed, 2.35s)\nAll checks passed.",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_session_context",
                    "Called without an active session context",
                    "invoke_subagent requires an active session context.",
                    "Invoke subagents only inside an active agent session.",
                ),
                ToolFailureMode::new(
                    "max_depth_exceeded",
                    "Subagent nesting depth reached max_depth",
                    "Maximum subagent recursion depth (<N>) reached.",
                    "Complete the remaining work directly using local tools instead of spawning another subagent.",
                ),
                ToolFailureMode::new(
                    "invalid_or_oversized_tasks",
                    "Missing role/prompt and tasks, empty tasks array, or more than 8 concurrent tasks",
                    "Missing required 'role' (or 'tasks' array) / Too many concurrent subagents requested (max 8 per batch).",
                    "Supply both 'role' and 'prompt', or provide 1 to 8 valid task objects in 'tasks'.",
                ),
            ],
            timeout_secs: 1800,
            is_custom: false,
        }
    }

    fn parameter_schema(&self) -> Value {
        let mut schema = self.contract().to_json_schema();
        if let Some(files_prop) = schema
            .get_mut("properties")
            .and_then(|p| p.get_mut("files"))
            .and_then(|f| f.as_object_mut())
        {
            files_prop.insert("items".to_string(), json!({ "type": "string" }));
        }
        if let Some(tasks_prop) = schema
            .get_mut("properties")
            .and_then(|p| p.get_mut("tasks"))
            .and_then(|t| t.as_object_mut())
        {
            tasks_prop.insert(
                "items".to_string(),
                json!({
                    "type": "object",
                    "properties": {
                        "role": { "type": "string" },
                        "prompt": { "type": "string" },
                        "read_only": { "type": "boolean" },
                        "model": { "type": "string" },
                        "provider_id": { "type": "string" },
                        "thinking_effort": { "type": "string" },
                        "server_id": { "type": "string" },
                        "job_tag": { "type": "string" },
                        "specialist": { "type": "string" },
                        "files": { "type": "array", "items": { "type": "string" } },
                        "include_board": { "type": "boolean" }
                    },
                    "required": ["role", "prompt"]
                }),
            );
        }
        schema
    }

    async fn execute(&self, _args: Value, _sandbox: &Path) -> Result<String, String> {
        Err("invoke_subagent requires an active session context.".to_string())
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let ctx = ctx.ok_or_else(|| "invoke_subagent requires an active session context.".to_string())?;

        if ctx.depth >= ctx.max_depth {
            return Err(format!(
                "Maximum subagent recursion depth ({}) reached. Complete this task directly using available tools.",
                ctx.max_depth
            ));
        }

        let top_read_only = args.get("read_only").and_then(|v| v.as_bool());
        let default_model = extract_opt_str(&args, "model");
        let default_provider = extract_opt_str(&args, "provider_id");
        let default_effort = extract_opt_str(&args, "thinking_effort")
            .or_else(|| extract_opt_str(&args, "reasoning_effort"));
        let default_server = extract_opt_str(&args, "server_id");
        let top_job_tag = extract_job_tag(&args);
        let top_specialist = extract_opt_str(&args, "specialist");
        let top_files = extract_string_array(&args, "files");
        let top_include_board = args
            .get("include_board")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(false);
        let swarm = resolve_swarm(ctx).await;

        let mut specs: Vec<SubagentSpec> = Vec::new();
        if let Some(tasks_arr) = args.get("tasks").and_then(|v| v.as_array()) {
            for item in tasks_arr {
                let role = item
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Each item in 'tasks' must include a non-empty 'role'.".to_string())?;
                let raw_prompt = item
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Each item in 'tasks' must include a non-empty 'prompt'.".to_string())?;
                let explicit_sp = extract_opt_str(item, "specialist")
                    .or_else(|| extract_opt_str(item, "subagent_type"))
                    .or_else(|| top_specialist.clone());
                let matched_profile = resolve_subagent_profile(ctx, role, explicit_sp.as_deref());
                let explicit_ro = item
                    .get("read_only")
                    .and_then(|v| v.as_bool())
                    .or(top_read_only)
                    .or_else(|| matched_profile.map(|p| p.read_only));
                let model = extract_opt_str(item, "model")
                    .or_else(|| default_model.clone())
                    .or_else(|| matched_profile.map(|p| p.model.trim().to_string()).filter(|s| !s.is_empty()));
                let provider_id = extract_opt_str(item, "provider_id")
                    .or_else(|| default_provider.clone())
                    .or_else(|| matched_profile.map(|p| p.provider_id.trim().to_string()).filter(|s| !s.is_empty()));
                let thinking_effort = extract_opt_str(item, "thinking_effort")
                    .or_else(|| extract_opt_str(item, "reasoning_effort"))
                    .or_else(|| default_effort.clone())
                    .or_else(|| matched_profile.map(|p| p.thinking_effort.trim().to_string()).filter(|s| !s.is_empty()));
                let server_id = extract_opt_str(item, "server_id")
                    .or_else(|| default_server.clone())
                    .or_else(|| matched_profile.map(|p| p.server_id.trim().to_string()).filter(|s| !s.is_empty()));
                let raw_job_tag = extract_job_tag(item).or_else(|| top_job_tag.clone());
                let raw_specialist = explicit_sp;
                let item_files = {
                    let specific = extract_string_array(item, "files");
                    if specific.is_empty() { top_files.clone() } else { specific }
                };
                let item_include_board = item
                    .get("include_board")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(top_include_board);

                let (base_prompt, read_only, job_tag, specialist) = resolve_subagent_tags(
                    swarm.as_ref(),
                    role,
                    raw_prompt,
                    explicit_ro,
                    raw_job_tag,
                    raw_specialist,
                )
                .await;
                let prompt = enrich_subagent_prompt(
                    swarm.as_ref(),
                    base_prompt,
                    &item_files,
                    item_include_board,
                )
                .await;

                specs.push(SubagentSpec {
                    role: role.to_string(),
                    prompt,
                    task_digest: compact_digest(raw_prompt, MAX_SWARM_DIGEST_CHARS),
                    read_only,
                    model,
                    provider_id,
                    thinking_effort,
                    server_id,
                    job_tag,
                    specialist,
                });
            }
        } else {
            let role = args
                .get("role")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Missing required 'role' (or 'tasks' array).".to_string())?;
            let raw_prompt = args
                .get("prompt")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Missing required 'prompt' (or 'tasks' array).".to_string())?;

            let matched_profile = resolve_subagent_profile(ctx, role, top_specialist.as_deref());
            let effective_ro = top_read_only.or_else(|| matched_profile.map(|p| p.read_only));
            let (base_prompt, read_only, job_tag, specialist) = resolve_subagent_tags(
                swarm.as_ref(),
                role,
                raw_prompt,
                effective_ro,
                top_job_tag,
                top_specialist,
            )
            .await;
            let prompt = enrich_subagent_prompt(
                swarm.as_ref(),
                base_prompt,
                &top_files,
                top_include_board,
            )
            .await;

            specs.push(SubagentSpec {
                role: role.to_string(),
                prompt,
                task_digest: compact_digest(raw_prompt, MAX_SWARM_DIGEST_CHARS),
                read_only,
                model: default_model.or_else(|| matched_profile.map(|p| p.model.trim().to_string()).filter(|s| !s.is_empty())),
                provider_id: default_provider.or_else(|| matched_profile.map(|p| p.provider_id.trim().to_string()).filter(|s| !s.is_empty())),
                thinking_effort: default_effort.or_else(|| matched_profile.map(|p| p.thinking_effort.trim().to_string()).filter(|s| !s.is_empty())),
                server_id: default_server.or_else(|| matched_profile.map(|p| p.server_id.trim().to_string()).filter(|s| !s.is_empty())),
                job_tag,
                specialist,
            });
        }

        if specs.is_empty() {
            return Err("No subagent tasks provided.".to_string());
        }
        if specs.len() > 8 {
            return Err("Too many concurrent subagents requested (max 8 per batch).".to_string());
        }
        // Global ceiling. The per-batch cap and `max_depth` bound one call and the
        // nesting, but their product is a fan-out: with depth 2 a coordinator can
        // spawn 8 workers, each spawning 8 more, and nothing looked at the global
        // count. A coordinator that fans out further than this would reach 584
        // concurrent loops on one machine, so refuse rather than OOM.
        if let Some(ref st) = ctx.state {
            let active = st.total_active_subagents().await;
            let cap = st.max_active_subagents().await;
            if active.saturating_add(specs.len()) > cap {
                return Err(format!(
                    "Too many subagents are already running ({active} active, limit {cap}). \
                     Wait for the current wave to finish (`swarm_board` action \"wait\") \
                     or raise `max_active_subagents` in settings."
                ));
            }
        }

        let sandbox_buf = sandbox.to_path_buf();

        if !wait {
            let mut spawned_lines = Vec::new();
            for spec in specs {
                let sub_id = allocate_subagent_id(&spec.role);
                let cancel = register_and_announce_subagent(&ctx, swarm.as_ref(), &sub_id, &spec).await;
                let model_label = resolve_effective_model_name(&spec, ctx);
                let tags_label = format_tag_suffix(spec.job_tag.as_deref(), spec.specialist.as_deref());
                spawned_lines.push(format!(
                    "- @{} (role: {}, model: {}){}",
                    sub_id, spec.role, model_label, tags_label
                ));
                let ctx_clone = ctx.clone();
                let sb_clone = sandbox_buf.clone();
                tokio::spawn(async move {
                    let _ = run_subagent_task(ctx_clone, sb_clone, spec, true, Some((sub_id, cancel))).await;
                });
            }
            let head_seq = match swarm {
                Some(ref sw) => sw.latest_seq(),
                None => 0,
            };
            return Ok(format!(
                "Spawned {} background subagent(s) (board head #{}):\n{}\nWorkers are running asynchronously in the background. Use `swarm_board` (`action: \"wait\"`, `since: {}`) to sleep with zero CPU until workers report or finish.",
                spawned_lines.len(),
                head_seq,
                spawned_lines.join("\n"),
                head_seq
            ));
        }

        let futs = specs.into_iter().map(|spec| {
            let ctx_clone = ctx.clone();
            let sb_clone = sandbox_buf.clone();
            tokio::spawn(async move { run_subagent_task(ctx_clone, sb_clone, spec, true, None).await })
        });

        let results = join_all(futs).await;
        let mut report = String::new();
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        for res in results {
            match res {
                Ok(out) => {
                    if out.success {
                        succeeded += 1;
                    } else {
                        failed += 1;
                    }
                    report.push_str(&format!(
                        "### Subagent `{}` (role: {}, status: {}, {:.2}s)\n{}\n\n",
                        out.subagent_id,
                        out.role,
                        if out.success { "completed" } else { "failed/stopped" },
                        out.duration_secs,
                        out.summary
                    ));
                }
                Err(join_err) => {
                    failed += 1;
                    report.push_str(&format!("### Subagent join error: {}\n\n", join_err));
                }
            }
        }

        let trimmed_report = report.trim_end();
        if failed == 0 {
            Ok(trimmed_report.to_string())
        } else if succeeded == 0 {
            Err(format!(
                "All {} subagent(s) failed or stopped without completing:\n\n{}",
                failed, trimmed_report
            ))
        } else {
            Ok(format!(
                "Partial subagent completion ({} succeeded, {} failed/stopped):\n\n{}",
                succeeded, failed, trimmed_report
            ))
        }
    }
}

// ----------------------------------------------------------------------------
// SwarmBoardPlugin
// ----------------------------------------------------------------------------
pub struct SwarmBoardPlugin;

fn format_tag_suffix(job_tag: Option<&str>, specialist: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(j) = job_tag.filter(|s| !s.is_empty()) {
        parts.push(format!("job:{}", j));
    }
    if let Some(s) = specialist.filter(|s| !s.is_empty()) {
        parts.push(format!("specialist:{}", s));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", parts.join(", "))
    }
}

#[async_trait]
impl ToolPlugin for SwarmBoardPlugin {
    fn name(&self) -> &'static str {
        "swarm_board"
    }

    fn description(&self) -> &'static str {
        "Collaborate with background subagents, wait zero-CPU for updates, set swarm title, cancel workers, and manage `(job)` & `(member specialist)` tags via the shared session Swarm Message Board (`post`, `read`, `wait`, `status`, `list_agents`, `set_title`, `cancel`, `clear`, `create_job`, `complete_job`, `list_jobs`, `create_specialist`, `list_specialists`, `assign_job`)."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "swarm".to_string(),
            permission_level: ToolPermission::Meta,
            fields: vec![
                ToolFieldSpec::required_string(
                    "action",
                    "Operation to perform on the session swarm board.",
                    "Choose one of: post, read, wait, list_agents, status, set_title, cancel, clear, create_job, complete_job, list_jobs, create_specialist, list_specialists, assign_job.",
                )
                .with_enum([
                    "post",
                    "read",
                    "wait",
                    "list_agents",
                    "status",
                    "set_title",
                    "cancel",
                    "clear",
                    "create_job",
                    "complete_job",
                    "list_jobs",
                    "create_specialist",
                    "list_specialists",
                    "assign_job",
                ])
                .non_empty(),
                ToolFieldSpec::optional_string(
                    "target",
                    "Optional recipient agent role/id or 'all' for 'post'/'assign_job', or subagent ID for 'cancel'.",
                )
                .with_error_hint("Provide a target agent role/id or 'all' (optional)."),
                ToolFieldSpec::optional_string(
                    "content",
                    "Message text for 'post' (or optional assignment note for 'assign_job'). Supports @mentions and #seq references.",
                )
                .with_error_hint("Provide a non-empty message string when action='post'."),
                ToolFieldSpec::optional_int(
                    "since",
                    "For 'read' or 'wait': return messages with seq > since (default 0).",
                )
                .with_min(0.0)
                .with_default(json!(0))
                .with_error_hint("Provide a non-negative integer >= 0 for since."),
                ToolFieldSpec::optional_bool(
                    "mentions_only",
                    "For 'wait': wake only when @mentioned/@all (true), or wake on any new post or worker finish (false, default).",
                )
                .with_default(json!(false))
                .with_error_hint("Pass a boolean true or false for mentions_only."),
                ToolFieldSpec::optional_bool(
                    "wait_all",
                    "For 'wait': block zero-CPU until ALL active background subagents finish or timeout_secs elapses (default false).",
                )
                .with_default(json!(false))
                .with_error_hint("Pass true to wait until all active background subagents complete, or false to wake on the next update."),
                ToolFieldSpec::optional_int(
                    "timeout_secs",
                    "For 'wait': maximum seconds to sleep zero-CPU before waking (1-120, default 45).",
                )
                .with_min(1.0)
                .with_default(json!(45))
                .with_error_hint("Provide an integer between 1 and 120 for timeout_secs."),
                ToolFieldSpec::optional_string(
                    "job_tag",
                    "Job tag ID for 'create_job', 'assign_job', or optional tag on 'post'.",
                )
                .with_error_hint("Provide a non-empty job tag ID."),
                ToolFieldSpec::optional_string(
                    "job_id",
                    "Alias for 'job_tag' (job ID) for 'create_job', 'assign_job', or 'post'.",
                )
                .with_error_hint("Provide a non-empty job ID."),
                ToolFieldSpec::optional_string(
                    "title",
                    "Swarm title for 'set_title', or human-readable title for 'create_job' / 'create_specialist'.",
                )
                .with_error_hint("Provide a title string."),
                ToolFieldSpec::optional_string(
                    "description",
                    "Description for 'create_job' (or specialty fallback for 'create_specialist').",
                )
                .with_error_hint("Provide a description string."),
                ToolFieldSpec::optional_string(
                    "status",
                    "Job status for 'create_job' or 'assign_job': 'open', 'in_progress', or 'done'.",
                )
                .with_enum(["open", "in_progress", "done"])
                .with_error_hint("Choose one of: open, in_progress, done."),
                ToolFieldSpec::optional_string(
                    "specialist",
                    "Specialist tag ID or name for 'create_specialist', 'assign_job', or optional tag on 'post'.",
                )
                .with_error_hint("Provide a non-empty specialist ID or name."),
                ToolFieldSpec::optional_string(
                    "specialty",
                    "Specialty instructions and domain focus for 'create_specialist'.",
                )
                .with_error_hint("Provide specialty instructions for the specialist."),
                ToolFieldSpec::optional_bool(
                    "read_only",
                    "Default read-only mode for 'create_specialist' (default false).",
                )
                .with_error_hint("Pass a boolean true or false for read_only."),
            ],
            expected_output: OutputContract::new(
                "text",
                "Confirmation of posted #seq message, woken messages from 'wait', job/specialist tag creation/assignment, formatted list of board messages/agents/tags, or full swarm status snapshot.",
                "Posted #2 from 'coordinator' (target: verifier)",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_session_context",
                    "No active session or SwarmBoard attached",
                    "swarm_board requires an active session context. / No SwarmBoard attached to this session.",
                    "Use swarm_board only inside an active session with a SwarmBoard.",
                ),
                ToolFailureMode::new(
                    "missing_required_field",
                    "Required field for the selected action (such as 'content' for post or 'job_tag' for create_job/assign_job) was omitted",
                    "Missing non-empty 'content' for swarm_board post. / Missing 'job_tag' (or 'job_id').",
                    "Supply the required parameters for the chosen action.",
                ),
                ToolFailureMode::new(
                    "invalid_action",
                    "action is not one of the allowed swarm_board actions",
                    "Invalid value '<action>' for field 'action' on tool 'swarm_board'",
                    "Pass one of: post, read, wait, list_agents, status, set_title, cancel, clear, create_job, list_jobs, create_specialist, list_specialists, assign_job.",
                ),
            ],
            timeout_secs: 130,
            is_custom: false,
        }
    }

    async fn execute(&self, _args: Value, _sandbox: &Path) -> Result<String, String> {
        Err("swarm_board requires an active session context.".to_string())
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let ctx = ctx.ok_or_else(|| "swarm_board requires an active session context.".to_string())?;
        let swarm = resolve_swarm(ctx)
            .await
            .ok_or_else(|| "No SwarmBoard attached to this session.".to_string())?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'action'".to_string())?;

        match action {
            "post" => {
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Missing non-empty 'content' for swarm_board post.".to_string())?;
                let target = args
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().trim_start_matches('@').to_string())
                    .filter(|s| !s.is_empty());
                let job_tag = extract_job_tag(&args);
                let specialist = extract_opt_str(&args, "specialist");

                let msg = swarm
                    .post_message_with_tags(
                        ctx.agent_id.clone(),
                        target.clone(),
                        content.to_string(),
                        job_tag.clone(),
                        specialist.clone(),
                    )
                    .await;
                emit_parent_event(ctx, msg.to_event()).await;

                Ok(format!(
                    "Posted #{} from '{}' (target: {}){}",
                    msg.seq,
                    msg.sender,
                    target.as_deref().unwrap_or("none"),
                    format_tag_suffix(msg.job_tag.as_deref(), msg.specialist.as_deref())
                ))
            }
            "read" => {
                let since = args.get("since").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let msgs = swarm.read_messages(since).await;
                if msgs.is_empty() {
                    return Ok(format!("No swarm board messages after #{}.", since));
                }
                let mut out = String::new();
                for m in msgs {
                    let tgt = m
                        .target
                        .as_deref()
                        .map(|t| format!(" -> @{}", t))
                        .unwrap_or_default();
                    let tags = format_tag_suffix(m.job_tag.as_deref(), m.specialist.as_deref());
                    out.push_str(&format!("#{} [{}{}]{}: {}\n", m.seq, m.sender, tgt, tags, m.content));
                }
                Ok(out.trim_end().to_string())
            }
            "list_agents" => {
                let agents = swarm.list_subagents().await;
                if agents.is_empty() {
                    return Ok("No subagents registered in this session yet.".to_string());
                }
                let mut out = String::new();
                for a in agents {
                    let tags = format_tag_suffix(a.job_tag.as_deref(), a.specialist.as_deref());
                    let summary_text = if a.summary.is_empty() {
                        compact_digest(&a.task, MAX_SWARM_DIGEST_CHARS)
                    } else {
                        compact_digest(&a.summary, MAX_SWARM_DIGEST_CHARS)
                    };
                    out.push_str(&format!(
                        "- {} (role: {}, status: {}, parent: {}){}: {}\n",
                        a.id,
                        a.role,
                        a.status,
                        a.parent_id.as_deref().unwrap_or("root"),
                        tags,
                        summary_text
                    ));
                }
                Ok(out.trim_end().to_string())
            }
            "status" => {
                let agents = swarm.list_subagents().await;
                let active = swarm.active_subagent_count().await;
                let msgs = swarm.all_messages().await;
                let artifacts = swarm.list_artifacts().await;
                let jobs = swarm.list_jobs().await;
                let specialists = swarm.list_specialists().await;

                let mut out = format!(
                    "Swarm Status: {} active / {} total subagent(s), {} job(s), {} specialist(s), {} board message(s), {} artifact(s)\n",
                    active,
                    agents.len(),
                    jobs.len(),
                    specialists.len(),
                    msgs.len(),
                    artifacts.len()
                );
                if !jobs.is_empty() {
                    out.push_str("\nJobs:\n");
                    for j in &jobs {
                        out.push_str(&format!("  - {} [{}]: {}\n", j.id, j.status, j.title));
                    }
                }
                if !specialists.is_empty() {
                    out.push_str("\nSpecialists:\n");
                    for s in &specialists {
                        out.push_str(&format!(
                            "  - {} ({}{}): {}\n",
                            s.id,
                            s.name,
                            if s.default_read_only { ", read_only" } else { "" },
                            compact_digest(&s.specialty, 120)
                        ));
                    }
                }
                if !agents.is_empty() {
                    out.push_str("\nAgents:\n");
                    for a in &agents {
                        let tags = format_tag_suffix(a.job_tag.as_deref(), a.specialist.as_deref());
                        out.push_str(&format!("  - {} [{}] ({}){}\n", a.id, a.role, a.status, tags));
                    }
                }
                if !msgs.is_empty() {
                    out.push_str("\nRecent Board Messages:\n");
                    let start = msgs.len().saturating_sub(10);
                    for m in &msgs[start..] {
                        let tgt = m
                            .target
                            .as_deref()
                            .map(|t| format!(" -> @{}", t))
                            .unwrap_or_default();
                        let tags = format_tag_suffix(m.job_tag.as_deref(), m.specialist.as_deref());
                        out.push_str(&format!(
                            "  #{} [{}{}]{}: {}\n",
                            m.seq,
                            m.sender,
                            tgt,
                            tags,
                            compact_digest(&m.content, 160)
                        ));
                    }
                }
                Ok(out.trim_end().to_string())
            }
            "clear" => {
                swarm.clear_messages().await;
                Ok("Cleared swarm board messages.".to_string())
            }
            "create_job" => {
                let title_opt = extract_opt_str(&args, "title");
                let id = extract_job_tag(&args)
                    .or_else(|| title_opt.as_ref().map(|t| sanitize_slug(t, "job")))
                    .ok_or_else(|| "Missing 'job_tag' (or 'job_id' / 'title') for action='create_job'.".to_string())?;
                let title = title_opt.unwrap_or_else(|| id.clone());
                let description = extract_opt_str(&args, "description").unwrap_or_default();
                let status = extract_opt_str(&args, "status");

                let job = swarm
                    .create_or_update_job(id, title, description, status)
                    .await;
                Ok(format!(
                    "Job tag '{}' ready (status: {}, title: '{}')",
                    job.id, job.status, job.title
                ))
            }
            "complete_job" => {
                let job_id = extract_job_tag(&args)
                    .ok_or_else(|| "Missing 'job_tag' (or 'job_id') for action='complete_job'.".to_string())?;
                let existing = swarm.get_job(&job_id).await;
                let title = extract_opt_str(&args, "title")
                    .or_else(|| existing.as_ref().map(|j| j.title.clone()))
                    .unwrap_or_else(|| job_id.clone());
                let description = extract_opt_str(&args, "description")
                    .or_else(|| existing.as_ref().map(|j| j.description.clone()))
                    .unwrap_or_default();
                let job = swarm
                    .create_or_update_job(job_id, title, description, Some("done".to_string()))
                    .await;
                if let Some(note) = extract_opt_str(&args, "content") {
                    let msg = swarm
                        .post_message_with_tags(
                            ctx.agent_id.clone(),
                            None,
                            compact_digest(&note, MAX_SWARM_DIGEST_CHARS),
                            Some(job.id.clone()),
                            None,
                        )
                        .await;
                    emit_parent_event(ctx, msg.to_event()).await;
                }
                Ok(format!("Job tag '{}' marked done (title: '{}').", job.id, job.title))
            }
            "list_jobs" => {
                let jobs = swarm.list_jobs().await;
                if jobs.is_empty() {
                    return Ok("No job tags defined in this session yet.".to_string());
                }
                let mut out = String::new();
                for j in jobs {
                    let desc = if j.description.is_empty() {
                        String::new()
                    } else {
                        format!(" - {}", compact_digest(&j.description, 140))
                    };
                    out.push_str(&format!("- {} [{}]: {}{}\n", j.id, j.status, j.title, desc));
                }
                Ok(out.trim_end().to_string())
            }
            "create_specialist" => {
                let title_opt = extract_opt_str(&args, "title");
                let spec_opt = extract_opt_str(&args, "specialist");
                let id_or_name = spec_opt
                    .clone()
                    .or_else(|| title_opt.clone())
                    .ok_or_else(|| "Missing 'specialist' (or 'title') for action='create_specialist'.".to_string())?;
                let id = sanitize_slug(&id_or_name, "specialist");
                let name = title_opt.unwrap_or(id_or_name);
                let specialty = extract_opt_str(&args, "specialty")
                    .or_else(|| extract_opt_str(&args, "description"))
                    .unwrap_or_default();
                let read_only = args
                    .get("read_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let sp = swarm
                    .create_or_update_specialist(id, name, specialty, read_only)
                    .await;
                Ok(format!(
                    "Specialist tag '{}' ready (name: '{}', default_read_only: {})",
                    sp.id, sp.name, sp.default_read_only
                ))
            }
            "list_specialists" => {
                let specialists = swarm.list_specialists().await;
                if specialists.is_empty() {
                    return Ok("No specialist tags defined in this session yet.".to_string());
                }
                let mut out = String::new();
                for s in specialists {
                    let focus = if s.specialty.is_empty() {
                        "(no specialty instructions)".to_string()
                    } else {
                        compact_digest(&s.specialty, 140)
                    };
                    out.push_str(&format!(
                        "- {} (name: '{}', read_only: {}): {}\n",
                        s.id, s.name, s.default_read_only, focus
                    ));
                }
                Ok(out.trim_end().to_string())
            }
            "assign_job" => {
                let job_id = extract_job_tag(&args)
                    .ok_or_else(|| "Missing 'job_tag' (or 'job_id') for action='assign_job'.".to_string())?;
                let title = extract_opt_str(&args, "title").unwrap_or_default();
                let description = extract_opt_str(&args, "description").unwrap_or_default();
                let status = extract_opt_str(&args, "status").or_else(|| Some("in_progress".to_string()));
                let job = swarm
                    .create_or_update_job(job_id, title, description, status)
                    .await;

                let target = args
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().trim_start_matches('@').to_string())
                    .filter(|s| !s.is_empty());

                let resolved_specialist = if let Some(sp_ref) = extract_opt_str(&args, "specialist") {
                    if let Some(existing) = swarm.get_specialist(&sp_ref).await {
                        Some(existing.id)
                    } else {
                        let created = swarm
                            .create_or_update_specialist(
                                sanitize_slug(&sp_ref, "specialist"),
                                sp_ref.clone(),
                                extract_opt_str(&args, "specialty").unwrap_or_default(),
                                args.get("read_only").and_then(|v| v.as_bool()).unwrap_or(false),
                            )
                            .await;
                        Some(created.id)
                    }
                } else {
                    None
                };

                if let Some(ref tgt) = target {
                    let _ = swarm
                        .assign_subagent_tags(tgt, Some(job.id.clone()), resolved_specialist.clone())
                        .await;
                }

                let note = extract_opt_str(&args, "content")
                    .unwrap_or_else(|| format!("Job '{}' [{}] assigned", job.id, job.status));
                let msg = swarm
                    .post_message_with_tags(
                        ctx.agent_id.clone(),
                        target.clone(),
                        compact_digest(&note, MAX_SWARM_DIGEST_CHARS),
                        Some(job.id.clone()),
                        resolved_specialist.clone(),
                    )
                    .await;
                emit_parent_event(ctx, msg.to_event()).await;

                Ok(format!(
                    "Assigned job '{}' (status: {}, target: {}{}) -> posted #{}",
                    job.id,
                    job.status,
                    target.as_deref().unwrap_or("none"),
                    resolved_specialist
                        .as_deref()
                        .map(|s| format!(", specialist: {}", s))
                        .unwrap_or_default(),
                    msg.seq
                ))
            }
            "wait" => {
                let since = args.get("since").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let mentions_only = args
                    .get("mentions_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let wait_all = args
                    .get("wait_all")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let timeout_secs = args
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(45)
                    .clamp(1, 120);

                let wait_outcome = if wait_all {
                    swarm
                        .wait_for_all_or_cancel(since, timeout_secs, &ctx.cancel_notify)
                        .await
                } else {
                    swarm
                        .wait_for_activity_or_cancel(
                            &ctx.agent_id,
                            since,
                            mentions_only,
                            timeout_secs,
                            &ctx.cancel_notify,
                        )
                        .await
                };

                match wait_outcome {
                    None => Ok("Swarm wait cancelled (session stopping).".to_string()),
                    Some((msgs, active_left, timed_out)) => {
                        let head = swarm.latest_seq();
                        let agents = swarm.list_subagents().await;
                        let active_names: Vec<String> = agents
                            .iter()
                            .filter(|a| matches!(a.status.as_str(), "running" | "waiting"))
                            .map(|a| format!("@{}({})", a.id, a.role))
                            .collect();
                        let done_count = agents.iter().filter(|a| a.status == "completed").count();
                        let fail_count = agents
                            .iter()
                            .filter(|a| matches!(a.status.as_str(), "failed" | "stopped"))
                            .count();

                        let header = if timed_out {
                            format!(
                                "Wait timed out after {}s (board head #{}, {} active background subagent(s) [{} done, {} failed]{}).",
                                timeout_secs,
                                head,
                                active_left,
                                done_count,
                                fail_count,
                                if active_names.is_empty() {
                                    String::new()
                                } else {
                                    format!(": {}", active_names.join(", "))
                                }
                            )
                        } else {
                            format!(
                                "Woken by swarm activity (board head #{}, {} active background subagent(s) [{} done, {} failed], {} message(s) after #{}):",
                                head,
                                active_left,
                                done_count,
                                fail_count,
                                msgs.len(),
                                since
                            )
                        };
                        if msgs.is_empty() {
                            return Ok(header);
                        }
                        let mut out = format!("{}\n", header);
                        for m in msgs {
                            let tgt = m
                                .target
                                .as_deref()
                                .map(|t| format!(" -> @{}", t))
                                .unwrap_or_default();
                            let tags = format_tag_suffix(m.job_tag.as_deref(), m.specialist.as_deref());
                            out.push_str(&format!("#{} [{}{}]{}: {}\n", m.seq, m.sender, tgt, tags, m.content));
                        }
                        Ok(out.trim_end().to_string())
                    }
                }
            }
            "set_title" => {
                let raw_title = extract_opt_str(&args, "title")
                    .or_else(|| extract_opt_str(&args, "content"))
                    .ok_or_else(|| "Missing 'title' (or 'content') for action='set_title'.".to_string())?;
                let saved = swarm.set_title(raw_title).await;
                Ok(format!("Swarm title set to '{}'.", saved))
            }
            "cancel" => {
                let target = args
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().trim_start_matches('@').to_string())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "Missing 'target' subagent ID (or 'all') for action='cancel'.".to_string())?;
                if target.eq_ignore_ascii_case("all") {
                    swarm.cancel_all_subagents().await;
                    Ok("Cancelled all active subagents on this swarm board.".to_string())
                } else if swarm.cancel_subagent(&target).await {
                    Ok(format!("Cancelled subagent @{}.", target))
                } else {
                    Err(format!("Subagent '{}' not found on this swarm board.", target))
                }
            }
            other => Err(format!(
                "Unknown swarm_board action '{}'. Valid actions: post, read, wait, list_agents, status, set_title, cancel, clear, create_job, complete_job, list_jobs, create_specialist, list_specialists, assign_job.",
                other
            )),
        }
    }
}

// ----------------------------------------------------------------------------
// CreateArtifactPlugin
// ----------------------------------------------------------------------------
pub struct CreateArtifactPlugin;

#[async_trait]
impl ToolPlugin for CreateArtifactPlugin {
    fn name(&self) -> &'static str {
        "create_artifact"
    }

    fn description(&self) -> &'static str {
        "Write a structured artifact (implementation plan, walkthrough, report, spec, or markdown) into `.harness_artifacts/` and surface it in the UI Artifact Inspector."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "artifacts".to_string(),
            permission_level: ToolPermission::WorkspaceWrite,
            fields: vec![
                ToolFieldSpec::required_string(
                    "title",
                    "Human-readable artifact title.",
                    "Provide a non-empty title string for the artifact.",
                )
                .non_empty(),
                ToolFieldSpec::optional_string(
                    "kind",
                    "Artifact category (default 'markdown').",
                )
                .with_enum([
                    "plan",
                    "walkthrough",
                    "report",
                    "spec",
                    "markdown",
                    "code",
                    "json",
                ])
                .with_default(json!("markdown"))
                .with_error_hint(
                    "Choose one of: plan, walkthrough, report, spec, markdown, code, json.",
                ),
                ToolFieldSpec::required_string(
                    "content",
                    "Full text/markdown content of the artifact.",
                    "Provide the string content for the artifact.",
                ),
                ToolFieldSpec::optional_string(
                    "filename",
                    "Optional filename inside `.harness_artifacts/` (sanitized; auto-generated if omitted).",
                )
                .with_error_hint(
                    "Provide a simple filename like 'summary.md' (directory components will be stripped).",
                ),
            ],
            expected_output: OutputContract::new(
                "json",
                "Confirmation text followed by a JSON object containing artifact id, title, kind, path, url, and bytes.",
                "Artifact created:\n{\n  \"id\": \"art_plan_1\",\n  \"path\": \".harness_artifacts/plan_1.md\",\n  \"bytes\": 420\n}",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_title_or_content",
                    "title is empty/missing or content is missing",
                    "Missing required parameter 'title' / 'content'",
                    "Provide both a non-empty 'title' and string 'content'.",
                ),
                ToolFailureMode::new(
                    "artifact_write_failed",
                    "Unable to create .harness_artifacts directory or write the artifact file",
                    "Failed to create .harness_artifacts directory / Failed to write artifact '<path>'",
                    "Check workspace write permissions and ensure the filename is valid.",
                ),
            ],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        self.create_artifact_inner(args, sandbox, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        self.create_artifact_inner(args, sandbox, ctx).await
    }
}

impl CreateArtifactPlugin {
    async fn create_artifact_inner(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Missing required parameter 'title'".to_string())?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'content'".to_string())?;
        let kind = args
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("markdown")
            .to_string();

        let seq = ARTIFACT_SEQ.fetch_add(1, Ordering::Relaxed);
        let id = format!("art_{}_{}", sanitize_slug(title, "artifact"), seq);

        let default_ext = if kind == "json" { "json" } else { "md" };
        let safe_filename = match args.get("filename").and_then(|v| v.as_str()) {
            Some(raw) if !raw.trim().is_empty() => {
                let base = Path::new(raw.trim())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                let cleaned: String = base
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                if cleaned.is_empty() || cleaned.starts_with('.') {
                    format!("{}.{}", id, default_ext)
                } else {
                    cleaned
                }
            }
            _ => format!("{}_{}.{}", sanitize_slug(title, "artifact"), seq, default_ext),
        };

        let artifacts_dir = sandbox.join(".harness_artifacts");
        fs::create_dir_all(&artifacts_dir)
            .await
            .map_err(|e| format!("Failed to create .harness_artifacts directory: {}", e))?;

        let full_path = artifacts_dir.join(&safe_filename);
        fs::write(&full_path, content)
            .await
            .map_err(|e| format!("Failed to write artifact '{}': {}", full_path.display(), e))?;

        let rel_path = format!(".harness_artifacts/{}", safe_filename);
        let preview = truncate_chars(content.trim(), 240);

        if let Some(ctx) = ctx {
            if let Some(swarm) = resolve_swarm(ctx).await {
                let record = swarm
                    .add_artifact(id.clone(), title, &kind, &rel_path, &preview)
                    .await;
                emit_parent_event(ctx, record.to_event()).await;
            } else {
                emit_parent_event(
                    ctx,
                    HarnessEvent::ArtifactCreated {
                        id: id.clone(),
                        title: title.to_string(),
                        kind: kind.clone(),
                        path: rel_path.clone(),
                        preview: preview.clone(),
                    },
                )
                .await;
            }
        }

        let summary = json!({
            "id": id,
            "title": title,
            "kind": kind,
            "path": rel_path,
            "url": format!("/api/artifacts/{}", safe_filename),
            "bytes": content.len()
        });
        Ok(format!("Artifact created:\n{}", serde_json::to_string_pretty(&summary).unwrap_or_else(|_| summary.to_string())))
    }
}
