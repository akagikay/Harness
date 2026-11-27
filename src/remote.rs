use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

use crate::config::{HarnessConfig, ServerNodeConfig};
use crate::engine::context::{ChatAttachment, ChatMessage};
use crate::engine::events::HarnessEvent;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRunRequest {
    pub run_id: String,
    pub session_id: String,
    pub task: String,
    #[serde(default)]
    pub prior_messages: Vec<ChatMessage>,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    pub config: HarnessConfig,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub depth: usize,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub use_mock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRunFinished {
    pub status: String,
    #[serde(default)]
    pub final_answer: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub updated_messages: Vec<ChatMessage>,
}

fn normalized_base_url(server: &ServerNodeConfig) -> String {
    let trimmed = server.url.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("local://")
    {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    }
}

fn apply_auth(req: reqwest::RequestBuilder, server: &ServerNodeConfig) -> reqwest::RequestBuilder {
    let tok = server.api_token.trim();
    if tok.is_empty() {
        req
    } else {
        req.header("Authorization", format!("Bearer {}", tok))
    }
}

/// Reject credential-bearing calls to a plaintext remote host.
///
/// A remote run sends the model configuration (which includes provider keys) to
/// the execution server. Over plain HTTP on a non-loopback host that leaks every
/// credential to anyone on the network path, so refuse it.
pub fn ensure_secure_transport(server: &ServerNodeConfig) -> Result<(), String> {
    let base = normalized_base_url(server);
    let lower = base.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("local://") {
        return Ok(());
    }
    let host = lower
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    // Same parser as the web layer's rebinding guard: a host that merely starts
    // with `127.` (`127.evil.com`) is not loopback, and this call carries the
    // whole config - provider API keys included - in plaintext.
    let loopback = crate::web::security::is_loopback_host(&host);
    if loopback {
        Ok(())
    } else {
        Err(format!(
            "Refusing to send credentials to '{}' over plaintext HTTP. \
             Use https:// or a loopback address.",
            server.url
        ))
    }
}

pub async fn probe_remote_server(server: &ServerNodeConfig) -> Result<serde_json::Value, String> {
    let base = normalized_base_url(server);
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err("Server URL must start with http:// or https://".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = apply_auth(client.get(format!("{}/api/server/info", base)), server)
        .send()
        .await
        .map_err(|e| format!("Failed to reach {}: {}", base, e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} from {}", resp.status(), base));
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Invalid JSON from {}: {}", base, e))
}

pub async fn forward_remote_stop(server: &ServerNodeConfig, run_id: &str) -> Result<(), String> {
    let base = normalized_base_url(server);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;
    let _ = apply_auth(
        client.post(format!("{}/api/server/runs/{}/stop", base, run_id)),
        server,
    )
    .send()
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn forward_remote_steer(
    server: &ServerNodeConfig,
    run_id: &str,
    content: &str,
) -> Result<(), String> {
    let base = normalized_base_url(server);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = apply_auth(
        client
            .post(format!("{}/api/server/runs/{}/steer", base, run_id))
            .json(&serde_json::json!({ "content": content })),
        server,
    )
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Remote steer failed: HTTP {}", resp.status()));
    }
    Ok(())
}

pub async fn forward_remote_permission_decision(
    server: &ServerNodeConfig,
    perm_id: &str,
    approved: bool,
) -> Result<(), String> {
    forward_remote_permission_decision_with(server, perm_id, approved, false).await
}

/// Forward a decision, optionally carrying "always allow".
///
/// Without `always_allow` in the body the remote server records a one-shot
/// decision, so the UI's "Approve & always allow" would be a lie on every
/// multi-server setup: the next identical tool call prompts again.
pub async fn forward_remote_permission_decision_with(
    server: &ServerNodeConfig,
    perm_id: &str,
    approved: bool,
    always_allow: bool,
) -> Result<(), String> {
    let base = normalized_base_url(server);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = apply_auth(
        client
            .post(format!("{}/api/permissions/{}/decision", base, perm_id))
            .json(&serde_json::json!({
                "approved": approved,
                "always_allow": always_allow
            })),
        server,
    )
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Remote permission decision failed: HTTP {}", resp.status()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_remote_server_run(
    server: &ServerNodeConfig,
    run_id: String,
    session_id: String,
    task: String,
    prior_messages: Vec<ChatMessage>,
    attachments: Vec<ChatAttachment>,
    config: HarnessConfig,
    agent_id: Option<String>,
    role: Option<String>,
    parent_id: Option<String>,
    depth: usize,
    read_only: bool,
    use_mock: bool,
    cancel_notify: Arc<Notify>,
    state: Option<AppState>,
    event_tx: mpsc::Sender<HarnessEvent>,
) -> Result<(String, Vec<ChatMessage>), String> {
    let base = normalized_base_url(server);
    // The request body carries the model configuration, which includes provider
    // keys. Never ship that over plaintext HTTP to a non-loopback host.
    ensure_secure_transport(server)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let req_body = RemoteRunRequest {
        run_id: run_id.clone(),
        session_id,
        task,
        prior_messages,
        attachments,
        config,
        agent_id,
        role,
        parent_id,
        depth,
        read_only,
        use_mock,
    };

    let start_resp = apply_auth(client.post(format!("{}/api/server/runs", base)), server)
        .json(&req_body)
        .send()
        .await
        .map_err(|e| format!("Failed to dispatch run to server '{}': {}", server.name, e))?;

    if !start_resp.status().is_success() {
        let status = start_resp.status();
        let body = start_resp.text().await.unwrap_or_default();
        return Err(format!(
            "Remote server '{}' rejected run ({}): {}",
            server.name, status, body
        ));
    }

    let sse_resp = apply_auth(
        client.get(format!("{}/api/server/runs/{}/events", base, run_id)),
        server,
    )
    .send()
    .await
    .map_err(|e| format!("Failed to attach event stream on '{}': {}", server.name, e))?;

    if !sse_resp.status().is_success() {
        let status = sse_resp.status();
        let body = sse_resp.text().await.unwrap_or_default();
        return Err(format!(
            "Remote server '{}' event stream error ({}): {}",
            server.name, status, body
        ));
    }

    let done_notify = Arc::new(Notify::new());
    {
        let done_c = done_notify.clone();
        let cancel_c = cancel_notify.clone();
        let srv_c = server.clone();
        let rid_c = run_id.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_c.notified() => {
                    let _ = forward_remote_stop(&srv_c, &rid_c).await;
                }
                _ = done_c.notified() => {}
            }
        });
    }

    let mut stream = sse_resp.bytes_stream();
    let mut line_buf = String::new();
    let mut current_event_type = String::new();
    let mut current_data_lines: Vec<String> = Vec::new();
    let mut finished_payload: Option<RemoteRunFinished> = None;

    while let Some(chunk_res) = stream.next().await {
        let chunk = match chunk_res {
            Ok(b) => b,
            Err(e) => {
                done_notify.notify_waiters();
                return Err(format!("Stream read error from '{}': {}", server.name, e));
            }
        };
        line_buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(nl) = line_buf.find('\n') {
            let line = line_buf[..nl].trim_end_matches('\r').to_string();
            line_buf.drain(..=nl);

            if line.is_empty() {
                if !current_data_lines.is_empty() {
                    let data_str = current_data_lines.join("\n");
                    let evt_name = std::mem::take(&mut current_event_type);
                    current_data_lines.clear();

                    if evt_name == "run_finished" {
                        if let Ok(fin) = serde_json::from_str::<RemoteRunFinished>(&data_str) {
                            finished_payload = Some(fin);
                            break;
                        }
                    } else if let Ok(evt) = serde_json::from_str::<HarnessEvent>(&data_str) {
                        if let HarnessEvent::PermissionRequested { ref perm_id, .. } = evt {
                            if let Some(ref st) = state {
                                let mut routes = st.remote_perm_routes.write().await;
                                routes.insert(perm_id.clone(), server.id.clone());
                            }
                        }
                        let _ = event_tx.send(evt).await;
                    }
                } else {
                    current_event_type.clear();
                }
                continue;
            }

            if line.starts_with(':') {
                continue;
            }
            if let Some(ev) = line.strip_prefix("event:") {
                current_event_type = ev.trim().to_string();
            } else if let Some(d) = line.strip_prefix("data:") {
                current_data_lines.push(d.trim_start().to_string());
            }
        }

        if finished_payload.is_some() {
            break;
        }
    }

    done_notify.notify_waiters();

    match finished_payload {
        Some(fin) => {
            if fin.status == "error" {
                Err(fin.error.unwrap_or_else(|| "Remote execution failed".to_string()))
            } else {
                Ok((fin.final_answer, fin.updated_messages))
            }
        }
        None => Err(format!(
            "Remote server '{}' closed event stream before completion",
            server.name
        )),
    }
}
