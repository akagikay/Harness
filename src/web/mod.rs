pub mod handlers;
pub mod security;

use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post},
    Router,
};
use tower_http::compression::CompressionLayer;

use std::sync::Arc;

use crate::state::AppState;
use security::HttpSecurity;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Embedded UI assets
        .route("/", get(handlers::get_index))
        .route("/static/style.css", get(handlers::get_style))
        .route("/static/app.js", get(handlers::get_script))
        // API routes
        .route("/api/sessions", get(handlers::list_sessions).post(handlers::create_session))
        .route("/api/sessions/create", post(handlers::create_idle_conversation))
        .route(
            "/api/sessions/:id",
            patch(handlers::update_session_meta_handler).delete(handlers::delete_session_handler),
        )
        .route("/api/sessions/:id/events", get(handlers::stream_session_events))
        .route("/api/sessions/:id/history", get(handlers::get_session_history))
        .route("/api/sessions/:id/stop", post(handlers::stop_session))
        .route("/api/sessions/:id/compact", post(handlers::compact_session))
        .route("/api/sessions/:id/message", post(handlers::send_session_message))
        .route("/api/sessions/:id/queue", get(handlers::get_session_queue))
        .route("/api/sessions/:id/queue/:queue_id", delete(handlers::delete_queued_prompt))
        .route("/api/sessions/:id/queue/:queue_id/steer", post(handlers::steer_queued_prompt))
        .route("/api/sessions/:id/undo", post(handlers::undo_session_turn))
        .route("/api/sessions/:id/fork", post(handlers::fork_session))
        .route("/api/sessions/:id/swarm", get(handlers::get_session_swarm))
        .route("/api/sessions/:id/swarm/post", post(handlers::post_session_swarm))
        .route("/api/sessions/:id/subagents/:sub_id/stop", post(handlers::stop_subagent))
        .route("/api/artifacts/*path", get(handlers::get_artifact))
        .route("/api/workspace/tree", get(handlers::get_workspace_tree))
        .route("/api/workspace/file", get(handlers::get_workspace_file))
        .route("/api/skills", get(handlers::list_skills).post(handlers::save_skill))
        .route("/api/skills/:name", delete(handlers::delete_skill))
        .route("/api/filesystem/browse", get(handlers::browse_filesystem))
        .route("/api/filesystem/mkdir", post(handlers::create_directory))
        .route("/api/filesystem/rename", post(handlers::rename_path))
        .route("/api/filesystem/delete", post(handlers::delete_path))
        .route("/api/permissions/:perm_id/decision", post(handlers::decide_permission))
        .route("/api/metrics", get(handlers::get_metrics))
        .route("/api/config", get(handlers::get_config).post(handlers::update_config))
        .route("/api/db/optimize", post(handlers::db_optimize))
        .route("/api/update/check", get(handlers::check_update))
        .route("/api/update/status", get(handlers::update_status_handler))
        .route("/api/update/apply", post(handlers::apply_update))
        .route("/api/security/status", get(handlers::security_status))
        .route("/api/security/token", post(handlers::rotate_auth_token))
        .route(
            "/api/security/privileged-shell",
            post(handlers::set_privileged_shell),
        )
        .route("/api/onboarding", get(handlers::onboarding_state))
        .route(
            "/api/onboarding/complete",
            post(handlers::complete_onboarding),
        )
        .route(
            "/api/automations",
            get(handlers::list_automations).post(handlers::save_automation),
        )
        .route("/api/automation/logs", get(handlers::automation_logs_handler))
        .route(
            "/api/automations/:id",
            patch(handlers::update_automation_handler).delete(handlers::delete_automation),
        )
        .route("/api/automations/:id/logs", get(handlers::automation_logs_for_handler))
        .route("/api/automations/:id/run", post(handlers::run_automation_now))
        .route("/api/automations/:id/stop", post(handlers::stop_automation_handler))
        .route("/api/models/remote", get(handlers::list_remote_models).post(handlers::list_remote_models_post))
        .route("/api/models/all", get(handlers::list_all_provider_models))
        .route("/api/models/reasoning", get(handlers::get_model_reasoning))
        .route("/api/models/capabilities", get(handlers::get_model_capabilities))
        .route("/api/tools", get(handlers::list_tools).post(handlers::create_custom_tool))
        .route("/api/tools/:name", delete(handlers::delete_custom_tool_handler))
        .route("/api/tools/:name/validate", post(handlers::validate_tool_args))
        .route("/api/mcp", get(handlers::list_mcp_servers).post(handlers::save_mcp_server))
        .route("/api/mcp/:id", delete(handlers::delete_mcp_server))
        .route("/api/mcp/:id/test", post(handlers::test_mcp_server_endpoint))
        .route("/api/search/test", post(handlers::test_search))
        .route("/api/providers/active", post(handlers::activate_provider))
        .route("/api/projects", get(handlers::list_projects).post(handlers::save_project))
        .route("/api/projects/:id/active", post(handlers::set_active_project))
        .route("/api/projects/:id/defaults", patch(handlers::update_project_defaults))
        .route("/api/projects/:id", delete(handlers::delete_project))
        // Client Server-Directory & Remote Execution Server APIs
        .route("/api/servers", get(handlers::list_servers).post(handlers::save_server))
        .route("/api/servers/:id", delete(handlers::delete_server))
        .route("/api/servers/:id/active", post(handlers::set_active_server))
        .route("/api/servers/:id/health", get(handlers::check_server_health))
        .route("/api/server/info", get(handlers::server_info))
        .route("/api/server/runs", post(handlers::start_server_run))
        .route("/api/server/runs/:run_id/events", get(handlers::stream_server_run_events))
        .route("/api/server/runs/:run_id/stop", post(handlers::stop_server_run))
        .route("/api/server/runs/:run_id/steer", post(handlers::steer_server_run))
        // High efficiency layers & 32MB attachment upload allowance.
        // No permissive CORS: the UI is same-origin, so cross-origin reads are denied.
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(
            security_for(&state),
            security::security_middleware,
        ))
        .layer(middleware::from_fn(security::security_headers))
        .with_state(state)
}

/// Pure Execution Server Router (no Web UI / client history DB endpoints — tools & runs only).
pub fn build_server_router(state: AppState) -> Router {
    Router::new()
        .route("/api/server/info", get(handlers::server_info))
        .route("/api/server/runs", post(handlers::start_server_run))
        .route("/api/server/runs/:run_id/events", get(handlers::stream_server_run_events))
        .route("/api/server/runs/:run_id/stop", post(handlers::stop_server_run))
        .route("/api/server/runs/:run_id/steer", post(handlers::steer_server_run))
        .route("/api/permissions/:perm_id/decision", post(handlers::decide_permission))
        .route("/api/tools", get(handlers::list_tools).post(handlers::create_custom_tool))
        .route("/api/tools/:name", delete(handlers::delete_custom_tool_handler))
        .route("/api/tools/:name/validate", post(handlers::validate_tool_args))
        .route("/api/workspace/tree", get(handlers::get_workspace_tree))
        .route("/api/workspace/file", get(handlers::get_workspace_file))
        .route("/api/filesystem/browse", get(handlers::browse_filesystem))
        .route("/api/filesystem/mkdir", post(handlers::create_directory))
        .route("/api/filesystem/rename", post(handlers::rename_path))
        .route("/api/filesystem/delete", post(handlers::delete_path))
        .route("/api/metrics", get(handlers::get_metrics))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(
            security_for(&state),
            security::security_middleware,
        ))
        .layer(middleware::from_fn(security::security_headers))
        .with_state(state)
}

/// Derive HTTP security settings from the live config + bind host.
///
/// The auth token is re-read **per request** (see
/// [`security::HttpSecurity::with_live_token`]): a rotation through
/// `POST /api/config` must take effect at once, otherwise a leaked token stays
/// valid until the next restart and `/api/security/status` reports a rotation
/// that never happened.
fn security_for(state: &AppState) -> HttpSecurity {
    let snapshot = match state.config.try_read() {
        Ok(conf) => HttpSecurity::from_config(&conf.auth_token, &conf.host),
        // Router construction can race a write; the live source below re-reads.
        Err(_) => HttpSecurity::from_config("", "127.0.0.1"),
    };
    let config = Arc::clone(&state.config);
    snapshot.with_live_token(Arc::new(move || live_auth_token(&config)))
}

/// Token currently in force, read live from the config row.
///
/// Config wins over `HARNESS_AUTH_TOKEN` exactly as at startup. `None` means the
/// value could not be read (a writer holds the lock); the middleware then falls
/// back to the snapshot token rather than silently dropping auth.
fn live_auth_token(config: &tokio::sync::RwLock<crate::config::HarnessConfig>) -> Option<String> {
    let conf = config.try_read().ok()?;
    let token = conf.auth_token.trim().to_string();
    if !token.is_empty() {
        return Some(token);
    }
    Some(
        std::env::var("HARNESS_AUTH_TOKEN")
            .unwrap_or_default()
            .trim()
            .to_string(),
    )
}
