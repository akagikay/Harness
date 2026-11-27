use deepseek_harness_rs::config::{HarnessConfig, MemoryConfig, PermissionLevel};
use deepseek_harness_rs::engine::context::{
    ChatAttachment, ChatMessage, ConversationContext, ToolCall, ToolFunctionCall,
};
use deepseek_harness_rs::engine::events::HarnessEvent;
use deepseek_harness_rs::engine::loop_runner::AgentLoopRunner;
use deepseek_harness_rs::engine::recovery::{
    diagnose_tool_error, is_idempotent_retryable_tool, parse_and_repair_tool_args, ToolErrorKind,
};
use deepseek_harness_rs::engine::swarm::{SwarmBoard, MAX_BOARD_MESSAGES};
use deepseek_harness_rs::models::capabilities::{infer_model_modalities, resolve_model_capabilities};
use deepseek_harness_rs::models::mock_client::MockClient;
use deepseek_harness_rs::models::openai_client::build_openai_api_messages;
use deepseek_harness_rs::models::{ModelAdapter, ModelTurnResult};
use deepseek_harness_rs::plugins::browser_ui::BrowserUiPlugin;
use deepseek_harness_rs::plugins::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use deepseek_harness_rs::plugins::custom_tool::{
    custom_tools_dir, load_custom_tools_in, save_custom_tool, CustomToolRunner, CustomToolSpec,
};
use deepseek_harness_rs::plugins::mcp::test_mcp_server;
use deepseek_harness_rs::plugins::meta_tools::ManageToolsPlugin;
use deepseek_harness_rs::plugins::{
    coerce_args_for_contract, PluginRegistry, SessionToolContext, ToolPlugin,
};
use deepseek_harness_rs::state::AppState;
use deepseek_harness_rs::web::build_router;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

#[test]
fn test_context_truncation() {
    let ctx = ConversationContext::new(100);
    let large_text = "A".repeat(500);
    let (truncated, was_truncated) = ctx.truncate_tool_output(&large_text);

    assert!(was_truncated);
    assert!(truncated.contains("bytes truncated for resource efficiency"));
    assert!(truncated.len() < large_text.len());
}

#[tokio::test]
async fn test_tool_file_ops() {
    let temp_dir = std::env::temp_dir().join(format!("harness_test_{}", std::process::id()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let cfg = HarnessConfig::default();
    let registry = PluginRegistry::new(&cfg);

    // 1. Write file
    let write_res = registry.execute(
        "write_file",
        json!({
            "path": "test_doc.txt",
            "content": "Line 1: Rust Harness\nLine 2: DeepSeek Engine\nLine 3: High Efficiency"
        }),
        &temp_dir,
    ).await;
    assert!(write_res.is_ok());

    // 2. Read file with line bounds
    let read_res = registry.execute(
        "read_file",
        json!({
            "path": "test_doc.txt",
            "start_line": 2,
            "end_line": 2
        }),
        &temp_dir,
    ).await;
    assert!(read_res.is_ok());
    let read_out = read_res.unwrap();
    assert!(read_out.contains("Line 2: DeepSeek Engine"));
    assert!(!read_out.contains("Line 1"));

    // 3. Edit file
    let edit_res = registry.execute(
        "edit_file",
        json!({
            "path": "test_doc.txt",
            "target": "High Efficiency",
            "replacement": "Ultra High Resource Efficiency"
        }),
        &temp_dir,
    ).await;
    assert!(edit_res.is_ok());

    // 4. List directory
    let list_res = registry.execute(
        "list_dir",
        json!({ "path": "." }),
        &temp_dir,
    ).await;
    assert!(list_res.is_ok());
    assert!(list_res.unwrap().contains("test_doc.txt"));

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_tool_sys_info() {
    let cfg = HarnessConfig::default();
    let registry = PluginRegistry::new(&cfg);
    let res = registry.execute("sys_info", json!({}), &std::path::PathBuf::from(".")).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("System Information:"));
    assert!(out.contains("Harness Process RSS Memory"));
}

#[tokio::test]
async fn test_agent_loop_end_to_end() {
    let mut config = HarnessConfig::default();
    config.permissions.bash_policy = PermissionLevel::Allow;
    let registry = PluginRegistry::new(&config);
    let model = Arc::new(MockClient);

    let runner = AgentLoopRunner::new(
        "test_session_1".into(),
        "Test diagnostics run".into(),
        config,
        registry,
        model,
        None,
    );

    let (tx, mut rx) = mpsc::channel(100);
    let runner_handle = tokio::spawn(async move {
        runner.run(tx).await
    });

    let mut event_types = Vec::new();
    while let Some(evt) = rx.recv().await {
        match evt {
            HarnessEvent::SessionStarted { .. } => event_types.push("SessionStarted"),
            HarnessEvent::ThoughtDelta { .. } => event_types.push("ThoughtDelta"),
            HarnessEvent::ToolCallStarted { .. } => event_types.push("ToolCallStarted"),
            HarnessEvent::ToolCallFinished { .. } => event_types.push("ToolCallFinished"),
            HarnessEvent::MessageDelta { .. } => event_types.push("MessageDelta"),
            HarnessEvent::SessionCompleted { .. } => event_types.push("SessionCompleted"),
            _ => {}
        }
    }

    let final_res = runner_handle.await.unwrap();
    assert!(final_res.is_ok());
    assert!(event_types.contains(&"SessionStarted"));
    assert!(event_types.contains(&"ThoughtDelta"));
    assert!(event_types.contains(&"ToolCallStarted"));
    assert!(event_types.contains(&"ToolCallFinished"));
    assert!(event_types.contains(&"MessageDelta"));
    assert!(event_types.contains(&"SessionCompleted"));
}

#[tokio::test]
async fn test_swarm_board_messaging_and_mentions() {
    let board = SwarmBoard::new();

    let m1 = board
        .post_message("coordinator", None, "Heads up @worker2 only")
        .await;
    assert_eq!(m1.seq, 1);

    // Non-matching mention should time out when waiting for "@worker"
    let wait_miss = tokio::time::timeout(
        Duration::from_millis(40),
        board.wait_for_mention("worker", 0),
    )
    .await;
    assert!(
        wait_miss.is_err(),
        "@worker2 must not accidentally wake @worker"
    );

    // Concurrent waiter wakes immediately when "@worker" is mentioned
    let board_clone = board.clone();
    let waiter = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(2),
            board_clone.wait_for_mention("worker", 1),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(20)).await;
    let m2 = board
        .post_message("reviewer", Some("worker".into()), "Please check @worker")
        .await;
    assert_eq!(m2.seq, 2);

    let msgs = waiter.await.unwrap().expect("wait_for_mention timed out");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].seq, 2);
    assert_eq!(msgs[0].sender, "reviewer");

    // @all wakes any agent
    board.post_message("coordinator", None, "Status @all").await;
    let all_msgs = tokio::time::timeout(
        Duration::from_millis(200),
        board.wait_for_mention("inspector", 2),
    )
    .await
    .expect("@all should wake inspector");
    assert_eq!(all_msgs.len(), 1);
    assert_eq!(all_msgs[0].seq, 3);

    // Subagent lifecycle & bounded history
    let handle = board
        .register_subagent("sub-1", "researcher", "Audit API", None)
        .await;
    assert_eq!(handle.info.status, "running");
    assert_eq!(board.active_subagent_count().await, 1);
    assert!(board.cancel_subagent("sub-1").await);
    assert_eq!(board.active_subagent_count().await, 0);
    assert_eq!(board.get_subagent("sub-1").await.unwrap().info.status, "stopped");

    for i in 0..(MAX_BOARD_MESSAGES + 15) {
        board
            .post_message("stress", None, format!("msg {}", i))
            .await;
    }
    assert_eq!(board.all_messages().await.len(), MAX_BOARD_MESSAGES);
}

#[tokio::test]
async fn test_browser_ui_schema_sandbox_guard_and_http_check() {
    let plugin = BrowserUiPlugin::new();
    assert_eq!(plugin.name(), "browser_ui");

    let schema = plugin.parameter_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "action"));
    assert!(required.iter().any(|v| v == "url"));
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    for expected in ["navigate", "screenshot", "eval", "inspect_dom", "interact", "http_check"] {
        assert!(
            actions.iter().any(|v| v == expected),
            "Missing schema action {}",
            expected
        );
    }

    let cfg = HarnessConfig::default();
    let registry = PluginRegistry::new(&cfg);
    let schemas = registry.get_tools_schema();
    assert!(schemas
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["function"]["name"] == "browser_ui"));

    let temp_dir = std::env::temp_dir().join(format!("harness_browser_test_{}", std::process::id()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    // URL scheme guard
    let bad_scheme = plugin
        .execute(json!({ "action": "http_check", "url": "file:///C:/Windows/win.ini" }), &temp_dir)
        .await;
    assert!(bad_scheme.is_err());

    // Screenshot sandbox guard (path traversal & directory separators rejected pre-launch)
    let traversal = plugin
        .execute(
            json!({
                "action": "screenshot",
                "url": "http://127.0.0.1:9999",
                "screenshot_name": "../escape"
            }),
            &temp_dir,
        )
        .await;
    assert!(traversal.is_err());
    assert!(traversal.unwrap_err().contains("forbidden"));

    let nested_sep = plugin
        .execute(
            json!({
                "action": "screenshot",
                "url": "http://127.0.0.1:9999",
                "screenshot_name": "nested/shot"
            }),
            &temp_dir,
        )
        .await;
    assert!(nested_sep.is_err());

    // Live http_check against ephemeral local Axum server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/",
        axum::routing::get(|| async {
            axum::response::Html(
                "<html><head><title>Harness UI Test</title></head><body><h1 id=\"hdr\">Ready</h1></body></html>",
            )
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let check_res = plugin
        .execute(
            json!({
                "action": "http_check",
                "url": format!("http://{}/", addr),
                "assert_status": 200,
                "assert_text": "Ready",
                "selector": "#hdr"
            }),
            &temp_dir,
        )
        .await
        .expect("http_check failed");

    let json_start = check_res.find("```json\n").expect("missing json fence") + "```json\n".len();
    let json_end = check_res[json_start..].find("\n```").expect("missing closing fence") + json_start;
    let parsed: Value = serde_json::from_str(&check_res[json_start..json_end]).unwrap();
    assert_eq!(parsed["http_status"], 200);
    assert_eq!(parsed["title"], "Harness UI Test");
    assert_eq!(parsed["assertions_passed"], true);
    assert_eq!(parsed["selector_matches"], 1);

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_session_swarm_and_metrics_endpoints() {
    let mut cfg = HarnessConfig::default();
    let temp_dir = std::env::temp_dir().join(format!("harness_web_test_{}", std::process::id()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    cfg.sandbox_root = temp_dir.clone();

    let state = AppState::new(cfg);
    let session_id = "sess-swarm-test".to_string();
    state
        .create_session(session_id.clone(), "Verify swarm".into(), "default".into())
        .await;

    let swarm = state.get_session_swarm(&session_id).await.unwrap();
    swarm
        .register_subagent("sub-alpha", "verifier", "Check routes", None)
        .await;
    swarm
        .add_artifact("art-1", "Summary", "report", ".harness_artifacts/summary.md", "# Ok")
        .await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // /api/metrics
    let metrics_resp = client.get(format!("{}/api/metrics", base)).send().await.unwrap();
    assert_eq!(metrics_resp.status(), 200);
    let metrics: Value = metrics_resp.json().await.unwrap();
    assert!(metrics["process_rss_mb"].as_f64().unwrap() > 0.0);
    assert!(metrics["cpu_cores"].as_u64().unwrap() >= 1);
    assert_eq!(metrics["active_sessions_count"], 1);
    assert_eq!(metrics["active_subagents"], 1);

    // Missing session returns 404
    let missing_resp = client
        .get(format!("{}/api/sessions/does-not-exist/swarm", base))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_resp.status(), 404);

    // Post to swarm board
    let post_resp = client
        .post(format!("{}/api/sessions/{}/swarm/post", base, session_id))
        .json(&json!({
            "sender": "user",
            "target": "@verifier",
            "content": "Run final check"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(post_resp.status(), 200);
    let posted: Value = post_resp.json().await.unwrap();
    assert_eq!(posted["seq"], 1);
    assert_eq!(posted["target"], "verifier");

    // Get session swarm snapshot
    let swarm_resp = client
        .get(format!("{}/api/sessions/{}/swarm", base, session_id))
        .send()
        .await
        .unwrap();
    assert_eq!(swarm_resp.status(), 200);
    let swarm_json: Value = swarm_resp.json().await.unwrap();
    assert_eq!(swarm_json["subagents"].as_array().unwrap().len(), 1);
    assert_eq!(swarm_json["subagents"][0]["id"], "sub-alpha");
    assert_eq!(swarm_json["board_messages"].as_array().unwrap().len(), 1);
    assert_eq!(swarm_json["artifacts"].as_array().unwrap().len(), 1);

    // Stop subagent and verify active_subagents decrements
    let stop_resp = client
        .post(format!(
            "{}/api/sessions/{}/subagents/sub-alpha/stop",
            base, session_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(stop_resp.status(), 200);
    let stop_json: Value = stop_resp.json().await.unwrap();
    assert_eq!(stop_json["status"], "stopped");

    let metrics_after: Value = client
        .get(format!("{}/api/metrics", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(metrics_after["active_subagents"], 0);

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[test]
fn test_all_tools_declare_valid_contracts() {
    let cfg = HarnessConfig::default();
    let registry = PluginRegistry::new(&cfg);
    let contracts = registry.get_contracts();

    let expected_builtins = [
        "bash",
        "read_file",
        "write_file",
        "edit_file",
        "list_dir",
        "grep_search",
        "sys_info",
        "web_search",
        "web_fetch",
        "browser_ui",
        "invoke_subagent",
        "swarm_board",
        "create_artifact",
        "manage_tools",
    ];
    for name in expected_builtins {
        assert!(
            contracts.iter().any(|c| c.name == name),
            "Missing built-in tool contract for '{}'",
            name
        );
    }

    for contract in &contracts {
        assert!(
            !contract.name.trim().is_empty(),
            "Tool contract has empty name"
        );
        assert!(
            !contract.description.trim().is_empty(),
            "Tool '{}' has empty description",
            contract.name
        );
        assert!(
            !contract.category.trim().is_empty(),
            "Tool '{}' has empty category",
            contract.name
        );
        assert!(
            !contract.expected_output.format.trim().is_empty(),
            "Tool '{}' has empty expected_output.format",
            contract.name
        );
        assert!(
            !contract.expected_output.description.trim().is_empty(),
            "Tool '{}' has empty expected_output.description",
            contract.name
        );
        assert!(
            !contract.failure_modes.is_empty(),
            "Tool '{}' must declare at least one failure_mode",
            contract.name
        );
        assert!(
            contract.timeout_secs > 0,
            "Tool '{}' must have timeout_secs > 0",
            contract.name
        );

        let schema = contract.to_json_schema();
        assert_eq!(
            schema["type"], "object",
            "Tool '{}' schema root type must be object",
            contract.name
        );
        let props = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("Tool '{}' schema missing properties object", contract.name));
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("Tool '{}' schema missing required array", contract.name));

        assert_eq!(
            props.len(),
            contract.fields.len(),
            "Tool '{}' schema properties count mismatch",
            contract.name
        );

        for field in &contract.fields {
            let prop = props.get(&field.name).unwrap_or_else(|| {
                panic!(
                    "Tool '{}' schema missing property '{}'",
                    contract.name, field.name
                )
            });
            assert_eq!(
                prop["type"],
                field.field_type.as_json_type(),
                "Tool '{}' field '{}' schema type mismatch",
                contract.name,
                field.name
            );
            let in_required = required
                .iter()
                .any(|v| v.as_str() == Some(field.name.as_str()));
            assert_eq!(
                in_required, field.required,
                "Tool '{}' field '{}' required mismatch",
                contract.name, field.name
            );
        }
    }
}

#[tokio::test]
async fn test_tool_contract_validation_and_error_messages() {
    let contract = ToolContract {
        name: "sample_contract".to_string(),
        description: "Sample contract for validation tests".to_string(),
        category: "test".to_string(),
        permission_level: ToolPermission::ReadOnly,
        fields: vec![
            ToolFieldSpec::required_string(
                "action",
                "Action to perform",
                "Use 'run' or 'check'.",
            )
            .with_enum(["run", "check"])
            .non_empty(),
            ToolFieldSpec::required_int(
                "count",
                "Positive iteration count",
                "Pass an integer between 1 and 10.",
            )
            .with_range(Some(1.0), Some(10.0)),
            ToolFieldSpec::optional_string("note", "Optional note"),
        ],
        expected_output: OutputContract::new("text", "Status line", "ok"),
        failure_modes: vec![ToolFailureMode::new(
            "bad_input",
            "Invalid args",
            "Validation error",
            "Check required fields.",
        )],
        timeout_secs: 15,
        is_custom: false,
    };

    // Missing required field
    let missing_err = contract
        .validate_args(&json!({ "count": 2 }))
        .expect_err("Missing required field should fail");
    assert!(missing_err.contains("Missing required field 'action'"));

    // Wrong field type: integer passed for string
    let int_for_str_err = contract
        .validate_args(&json!({ "action": 123, "count": 2 }))
        .expect_err("Integer for string should fail");
    assert!(int_for_str_err.contains("must be a string"));

    // Wrong field type: string passed for integer
    let str_for_int_err = contract
        .validate_args(&json!({ "action": "run", "count": "2" }))
        .expect_err("String for integer should fail");
    assert!(str_for_int_err.contains("must be an integer"));

    // Invalid enum value
    let enum_err = contract
        .validate_args(&json!({ "action": "invalid", "count": 2 }))
        .expect_err("Invalid enum should fail");
    assert!(enum_err.contains("Invalid value 'invalid'"));
    assert!(enum_err.contains("run, check"));

    // Empty required string
    let empty_err = contract
        .validate_args(&json!({ "action": "   ", "count": 2 }))
        .expect_err("Whitespace-only non_empty string should fail");
    assert!(empty_err.contains("non-empty string"));

    // Numeric range bound
    let range_err = contract
        .validate_args(&json!({ "action": "run", "count": 0 }))
        .expect_err("Below minimum count should fail");
    assert!(range_err.contains("below minimum"));

    // Optional field omission succeeds
    assert!(contract
        .validate_args(&json!({ "action": "run", "count": 5 }))
        .is_ok());

    // Central enforcement through PluginRegistry::execute_with_context
    let cfg = HarnessConfig::default();
    let registry = PluginRegistry::new(&cfg);
    let temp_dir = std::env::temp_dir().join(format!(
        "harness_contract_exec_test_{}",
        std::process::id()
    ));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let bash_missing = registry
        .execute_with_context("bash", json!({}), &temp_dir, None)
        .await
        .expect_err("bash with empty object must fail contract validation");
    assert!(bash_missing.contains("Missing required field 'command'"));

    let bash_wrong_type = registry
        .execute_with_context("bash", json!({ "command": 42 }), &temp_dir, None)
        .await
        .expect_err("bash with integer command must fail contract validation");
    assert!(bash_wrong_type.contains("must be a string"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_custom_tool_lifecycle_and_agent_creation() {
    let temp_dir = std::env::temp_dir().join(format!(
        "harness_custom_tool_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    // 1. Direct CustomToolSpec save & load from .harness/tools/
    let spec = CustomToolSpec {
        contract: ToolContract {
            name: "saved_ping".to_string(),
            description: "Echo a ping target".to_string(),
            category: "custom".to_string(),
            permission_level: ToolPermission::ShellOrNetwork,
            fields: vec![
                ToolFieldSpec::required_string(
                    "target",
                    "Target label",
                    "Provide a non-empty target string.",
                )
                .non_empty(),
                ToolFieldSpec::optional_string("tag", "Optional tag").with_default(json!("v1")),
            ],
            expected_output: OutputContract::new("text", "Echo output", "hello_world_v1"),
            failure_modes: vec![ToolFailureMode::new(
                "missing_target",
                "target missing",
                "Missing required field 'target'",
                "Pass target string.",
            )],
            timeout_secs: 15,
            is_custom: true,
        },
        runner: CustomToolRunner::Shell {
            command_template: "echo hello_{{target}}_{{tag}}".to_string(),
        },
        enabled: true,
    };

    let saved_path = save_custom_tool(&temp_dir, &spec).expect("save_custom_tool failed");
    assert!(saved_path.exists());
    assert!(saved_path.starts_with(custom_tools_dir(&temp_dir)));

    let loaded = load_custom_tools_in(&temp_dir);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].contract.name, "saved_ping");
    assert!(loaded[0].contract.is_custom);

    // 2. Agent creation via ManageToolsPlugin -> hot-register in AppState -> execute -> remove
    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    let state = AppState::new(cfg.clone());
    let registry_snapshot = state.get_registry_snapshot().await;

    let ctx = SessionToolContext {
        session_id: "sess-custom-tool".to_string(),
        agent_id: "main".to_string(),
        parent_id: None,
        depth: 0,
        max_depth: 2,
        state: Some(state.clone()),
        swarm: None,
        event_tx: None,
        config: cfg.clone(),
        registry: registry_snapshot,
        model: Arc::new(MockClient),
        cancel_notify: Arc::new(Notify::new()),
        sandbox_roots: Vec::new(),
    };

    let manage = ManageToolsPlugin;
    let create_out = manage
        .execute_with_context(
            json!({
                "action": "create",
                "name": "greet_custom",
                "description": "Greet a named swarm member via shell",
                "category": "custom",
                "permission_level": "shell_or_network",
                "fields": [
                    {
                        "name": "who",
                        "field_type": "string",
                        "description": "Recipient name",
                        "required": true,
                        "non_empty": true,
                        "error_hint": "Provide a non-empty recipient name."
                    }
                ],
                "expected_output": {
                    "format": "text",
                    "description": "Greeting line",
                    "example": "greet_swarm"
                },
                "failure_modes": [
                    {
                        "code": "missing_who",
                        "condition": "who omitted",
                        "error_message": "Missing required field 'who'",
                        "recovery_hint": "Pass who string."
                    }
                ],
                "runner": {
                    "type": "shell",
                    "command_template": "echo greet_{{who}}"
                }
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("manage_tools create failed");

    let created_json: Value = serde_json::from_str(&create_out).unwrap();
    assert_eq!(created_json["status"], "created");
    assert_eq!(created_json["name"], "greet_custom");
    assert_eq!(created_json["hot_registered"], true);
    assert!(custom_tools_dir(&temp_dir).join("greet_custom.json").exists());

    // Execute newly hot-registered tool with valid args
    let updated_reg = state.get_registry_snapshot().await;
    assert!(updated_reg.get_contract("greet_custom").is_some());
    let exec_ok = updated_reg
        .execute_with_context("greet_custom", json!({ "who": "swarm" }), &temp_dir, None)
        .await
        .expect("custom tool valid execution failed");
    assert!(
        exec_ok.contains("greet_swarm"),
        "Unexpected custom tool output: {}",
        exec_ok
    );

    // Execute newly created tool with invalid args -> rejected by contract validation
    let exec_missing = updated_reg
        .execute_with_context("greet_custom", json!({}), &temp_dir, None)
        .await
        .expect_err("custom tool missing required field must fail");
    assert!(exec_missing.contains("Missing required field 'who'"));

    let exec_wrong_type = updated_reg
        .execute_with_context("greet_custom", json!({ "who": 123 }), &temp_dir, None)
        .await
        .expect_err("custom tool wrong field type must fail");
    assert!(exec_wrong_type.contains("must be a string"));

    // Remove via ManageToolsPlugin
    let remove_out = manage
        .execute_with_context(
            json!({
                "action": "remove",
                "name": "greet_custom"
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("manage_tools remove failed");
    let removed_json: Value = serde_json::from_str(&remove_out).unwrap();
    assert_eq!(removed_json["status"], "removed");
    assert_eq!(removed_json["unregistered"], true);
    assert!(!custom_tools_dir(&temp_dir).join("greet_custom.json").exists());
    assert!(state
        .get_registry_snapshot()
        .await
        .get_contract("greet_custom")
        .is_none());

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_tools_api_contract_endpoints() {
    let temp_dir = std::env::temp_dir().join(format!(
        "harness_tools_api_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. GET /api/tools returns built-in contracts
    let list_resp = client
        .get(format!("{}/api/tools", base))
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);
    let tools: Vec<Value> = list_resp.json().await.unwrap();
    let bash_item = tools
        .iter()
        .find(|t| t["name"] == "bash")
        .expect("bash missing from GET /api/tools");
    assert_eq!(bash_item["is_custom"], false);
    assert_eq!(bash_item["permission_level"], "shell_or_network");
    assert!(!bash_item["fields"].as_array().unwrap().is_empty());
    assert!(!bash_item["failure_modes"].as_array().unwrap().is_empty());
    assert_eq!(bash_item["schema"]["type"], "object");

    // 2. POST /api/tools cannot overwrite built-in tool
    let builtin_conflict = client
        .post(format!("{}/api/tools", base))
        .json(&json!({
            "name": "bash",
            "description": "Try overwrite",
            "runner": { "type": "shell", "command_template": "echo nope" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(builtin_conflict.status(), 400);

    // 3. POST /api/tools creates custom tool
    let create_resp = client
        .post(format!("{}/api/tools", base))
        .json(&json!({
            "name": "api_echo_tool",
            "description": "Custom API echo tool",
            "category": "custom",
            "permission_level": "shell_or_network",
            "fields": [
                {
                    "name": "text",
                    "field_type": "string",
                    "description": "Text to echo",
                    "required": true,
                    "non_empty": true,
                    "error_hint": "Provide non-empty text."
                }
            ],
            "expected_output": {
                "format": "text",
                "description": "Echoed string",
                "example": "hi"
            },
            "failure_modes": [
                {
                    "code": "missing_text",
                    "condition": "text omitted",
                    "error_message": "Missing required field 'text'",
                    "recovery_hint": "Pass text."
                }
            ],
            "runner": {
                "type": "shell",
                "command_template": "echo {{text}}"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    assert_eq!(created["name"], "api_echo_tool");
    assert_eq!(created["is_custom"], true);
    assert!(custom_tools_dir(&temp_dir).join("api_echo_tool.json").exists());

    // 4. GET /api/tools reflects newly created custom tool
    let list_after: Vec<Value> = client
        .get(format!("{}/api/tools", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_after
        .iter()
        .any(|t| t["name"] == "api_echo_tool" && t["is_custom"] == true));

    // 5. POST /api/tools/:name/validate checks valid, invalid, and unknown tools
    let val_ok: Value = client
        .post(format!("{}/api/tools/api_echo_tool/validate", base))
        .json(&json!({ "text": "hello" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(val_ok["valid"], true);

    let val_bad: Value = client
        .post(format!("{}/api/tools/api_echo_tool/validate", base))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(val_bad["valid"], false);
    assert!(val_bad["error"]
        .as_str()
        .unwrap()
        .contains("Missing required field 'text'"));

    let val_unknown = client
        .post(format!("{}/api/tools/no_such_tool_xyz/validate", base))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(val_unknown.status(), 404);

    // 6. DELETE /api/tools/:name removes custom tool, rejects built-ins, 404s when missing
    let del_builtin = client
        .delete(format!("{}/api/tools/bash", base))
        .send()
        .await
        .unwrap();
    assert_eq!(del_builtin.status(), 400);

    let del_ok = client
        .delete(format!("{}/api/tools/api_echo_tool", base))
        .send()
        .await
        .unwrap();
    assert_eq!(del_ok.status(), 200);
    let del_json: Value = del_ok.json().await.unwrap();
    assert_eq!(del_json["status"], "deleted");
    assert_eq!(del_json["name"], "api_echo_tool");
    assert!(!custom_tools_dir(&temp_dir).join("api_echo_tool.json").exists());

    let del_missing = client
        .delete(format!("{}/api/tools/api_echo_tool", base))
        .send()
        .await
        .unwrap();
    assert_eq!(del_missing.status(), 404);

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_steer_queue_undo_and_fork_state() {
    let cfg = HarnessConfig::default();
    let state = AppState::new(cfg);
    let session_id = "ses-state-multi".to_string();

    state
        .create_session(session_id.clone(), "Initial task".into(), "default".into())
        .await;

    // Turn 1
    let (t1, prior1, steer_inbox) = state
        .begin_turn(&session_id, "Turn 1 prompt")
        .await
        .expect("begin_turn 1 failed");
    assert_eq!(t1, 1);
    assert!(prior1.is_empty());

    state
        .emit_session_event(
            &session_id,
            HarnessEvent::MessageDelta {
                delta: "Reply 1".into(),
            },
        )
        .await;
    state
        .save_llm_messages(
            &session_id,
            vec![
                ChatMessage::system("sys"),
                ChatMessage::user("Turn 1 prompt"),
                ChatMessage::assistant(Some("Reply 1".into()), None, None),
            ],
        )
        .await;

    // Turn 2
    let (t2, prior2, _) = state
        .begin_turn(&session_id, "Turn 2 prompt")
        .await
        .expect("begin_turn 2 failed");
    assert_eq!(t2, 2);
    assert_eq!(prior2.len(), 3);

    state
        .emit_session_event(
            &session_id,
            HarnessEvent::MessageDelta {
                delta: "Reply 2".into(),
            },
        )
        .await;
    state
        .save_llm_messages(
            &session_id,
            vec![
                ChatMessage::system("sys"),
                ChatMessage::user("Turn 1 prompt"),
                ChatMessage::assistant(Some("Reply 1".into()), None, None),
                ChatMessage::user("Turn 2 prompt"),
                ChatMessage::assistant(Some("Reply 2".into()), None, None),
            ],
        )
        .await;

    // Enqueue prompts, promote first to steer, remove second
    let (q1, pos1, queue1) = state
        .enqueue_prompt(&session_id, "Queued A".into())
        .await
        .expect("enqueue_prompt 1 failed");
    assert_eq!(pos1, 1);
    assert_eq!(queue1.len(), 1);

    let (q2, pos2, queue2) = state
        .enqueue_prompt(&session_id, "Queued B".into())
        .await
        .expect("enqueue_prompt 2 failed");
    assert_eq!(pos2, 2);
    assert_eq!(queue2.len(), 2);

    let (promoted, remaining_after_promote) = state
        .promote_queued_to_steer(&session_id, &q1.id)
        .await
        .expect("promote_queued_to_steer failed");
    assert_eq!(promoted.content, "Queued A");
    assert_eq!(remaining_after_promote.len(), 1);
    assert_eq!(remaining_after_promote[0].id, q2.id);
    {
        let inbox = steer_inbox.lock().await;
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0], "Queued A");
    }

    let remaining_after_remove = state
        .remove_queued_prompt(&session_id, &q2.id)
        .await
        .expect("remove_queued_prompt failed");
    assert!(remaining_after_remove.is_empty());
    assert!(state
        .list_queued_prompts(&session_id)
        .await
        .unwrap()
        .is_empty());

    // Fork at turn 1 (slices before turn 2) and full fork
    let (fork1_id, fork1_title, fork1_proj) = state
        .fork_session(&session_id, Some(1))
        .await
        .expect("fork_session at turn 1 failed");
    assert!(fork1_id.starts_with("ses_fork_"));
    assert_eq!(fork1_title, "Initial task (Fork)");
    assert_eq!(fork1_proj, "default");

    let (fork_full_id, _, _) = state
        .fork_session(&session_id, None)
        .await
        .expect("full fork_session failed");

    {
        let lock = state.sessions.read().await;
        let f1 = lock.get(&fork1_id).unwrap();
        assert_eq!(f1.status, "completed");
        assert_eq!(f1.checkpoints.len(), 1);
        assert_eq!(f1.next_turn_id, 1);
        assert_eq!(f1.history.len(), 2);
        assert_eq!(f1.llm_messages.len(), 3);

        let f_full = lock.get(&fork_full_id).unwrap();
        assert_eq!(f_full.checkpoints.len(), 2);
        assert_eq!(f_full.next_turn_id, 2);
        assert_eq!(f_full.history.len(), 4);
        assert_eq!(f_full.llm_messages.len(), 5);
    }

    // Re-enqueue something before undo to verify undo clears pending queue & steer inbox
    state
        .enqueue_prompt(&session_id, "Stale queued".into())
        .await
        .unwrap();

    let (restored_prompt, remaining_history) = state
        .undo_to_turn(&session_id, 2)
        .await
        .expect("undo_to_turn 2 failed");
    assert_eq!(restored_prompt, "Turn 2 prompt");
    assert_eq!(remaining_history.len(), 2);

    {
        let lock = state.sessions.read().await;
        let sess = lock.get(&session_id).unwrap();
        assert_eq!(sess.status, "completed");
        assert_eq!(sess.checkpoints.len(), 1);
        assert_eq!(sess.next_turn_id, 1);
        assert_eq!(sess.llm_messages.len(), 3);
        assert!(sess.queued_prompts.is_empty());
    }
    assert!(steer_inbox.lock().await.is_empty());
}

#[tokio::test]
async fn test_session_multi_turn_queue_undo_fork_endpoints() {
    let temp_dir = std::env::temp_dir().join(format!(
        "harness_multi_turn_api_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. Create a session and wait for turn 1 to finish
    let create_resp: Value = client
        .post(format!("{}/api/sessions", base))
        .json(&json!({ "task": "First turn", "mock": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = create_resp["session_id"].as_str().unwrap().to_string();

    for _ in 0..100 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(s) = sessions.iter().find(|s| s["id"] == session_id) {
            if s["status"] != "running" {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // 2. Idle multi-turn continuation via POST /api/sessions/:id/message
    let msg2_resp = client
        .post(format!("{}/api/sessions/{}/message", base, session_id))
        .json(&json!({ "content": "Second turn", "mock": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg2_resp.status(), 200);
    let msg2_json: Value = msg2_resp.json().await.unwrap();
    assert_eq!(msg2_json["status"], "running");
    assert_eq!(msg2_json["turn_id"], 2);

    for _ in 0..100 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(s) = sessions.iter().find(|s| s["id"] == session_id) {
            if s["status"] != "running" {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let hist_after_t2: Vec<Value> = client
        .get(format!("{}/api/sessions/{}/history", base, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(hist_after_t2
        .iter()
        .any(|e| e["type"] == "UserMessage" && e["data"]["turn_id"] == 1 && e["data"]["content"] == "First turn"));
    assert!(hist_after_t2
        .iter()
        .any(|e| e["type"] == "UserMessage" && e["data"]["turn_id"] == 2 && e["data"]["content"] == "Second turn"));

    // 3. Running session queue default, GET /queue, POST /queue/:qid/steer, DELETE /queue/:qid, and mode=steer
    let running_id = "ses-running-api-test".to_string();
    state
        .create_session(running_id.clone(), "Running task".into(), "default".into())
        .await;
    let (_, _, steer_inbox) = state
        .begin_turn(&running_id, "Running task")
        .await
        .unwrap();

    // Queue default (omitting mode)
    let q_default_resp = client
        .post(format!("{}/api/sessions/{}/message", base, running_id))
        .json(&json!({ "content": "Queued prompt 1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(q_default_resp.status(), 200);
    let q_default_json: Value = q_default_resp.json().await.unwrap();
    assert_eq!(q_default_json["status"], "queued");
    assert_eq!(q_default_json["position"], 1);
    let qid1 = q_default_json["queue_id"].as_str().unwrap().to_string();

    // Second queued prompt
    let q2_json: Value = client
        .post(format!("{}/api/sessions/{}/message", base, running_id))
        .json(&json!({ "content": "Queued prompt 2", "mode": "queue" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(q2_json["status"], "queued");
    assert_eq!(q2_json["position"], 2);
    let qid2 = q2_json["queue_id"].as_str().unwrap().to_string();

    // GET /api/sessions/:id/queue
    let get_q_resp: Value = client
        .get(format!("{}/api/sessions/{}/queue", base, running_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(get_q_resp["queue"].as_array().unwrap().len(), 2);

    // Promote first queued prompt to steer
    let promote_resp = client
        .post(format!(
            "{}/api/sessions/{}/queue/{}/steer",
            base, running_id, qid1
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(promote_resp.status(), 200);
    let promote_json: Value = promote_resp.json().await.unwrap();
    assert_eq!(promote_json["status"], "steered");
    assert_eq!(promote_json["queue_id"], qid1);
    assert_eq!(promote_json["content"], "Queued prompt 1");
    assert_eq!(promote_json["queue"].as_array().unwrap().len(), 1);

    // Delete second queued prompt
    let del_q_resp = client
        .delete(format!(
            "{}/api/sessions/{}/queue/{}",
            base, running_id, qid2
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(del_q_resp.status(), 200);
    let del_q_json: Value = del_q_resp.json().await.unwrap();
    assert_eq!(del_q_json["status"], "removed");
    assert!(del_q_json["queue"].as_array().unwrap().is_empty());

    // Direct steer via POST /api/sessions/:id/message with mode="steer"
    let direct_steer_resp = client
        .post(format!("{}/api/sessions/{}/message", base, running_id))
        .json(&json!({ "content": "Direct steer note", "mode": "steer" }))
        .send()
        .await
        .unwrap();
    assert_eq!(direct_steer_resp.status(), 200);
    let direct_steer_json: Value = direct_steer_resp.json().await.unwrap();
    assert_eq!(direct_steer_json["status"], "steered");
    assert_eq!(direct_steer_json["content"], "Direct steer note");

    {
        let inbox = steer_inbox.lock().await;
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[0], "Queued prompt 1");
        assert_eq!(inbox[1], "Direct steer note");
    }

    // 4. Fork at turn 1 and full fork
    let fork_t1_resp = client
        .post(format!("{}/api/sessions/{}/fork", base, session_id))
        .json(&json!({ "turn_id": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(fork_t1_resp.status(), 200);
    let fork_t1_json: Value = fork_t1_resp.json().await.unwrap();
    let fork_id = fork_t1_json["session_id"].as_str().unwrap().to_string();
    assert!(fork_id.starts_with("ses_fork_"));
    assert!(fork_t1_json["title"].as_str().unwrap().contains("(Fork)"));

    let fork_hist: Vec<Value> = client
        .get(format!("{}/api/sessions/{}/history", base, fork_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(fork_hist
        .iter()
        .any(|e| e["type"] == "UserMessage" && e["data"]["turn_id"] == 1));
    assert!(!fork_hist
        .iter()
        .any(|e| e["type"] == "UserMessage" && e["data"]["turn_id"] == 2));

    // 5. Undo to turn 2 on original session
    let undo_resp = client
        .post(format!("{}/api/sessions/{}/undo", base, session_id))
        .json(&json!({ "turn_id": 2 }))
        .send()
        .await
        .unwrap();
    assert_eq!(undo_resp.status(), 200);
    let undo_json: Value = undo_resp.json().await.unwrap();
    assert_eq!(undo_json["status"], "undone");
    assert_eq!(undo_json["restored_prompt"], "Second turn");
    let remaining_hist = undo_json["history"].as_array().unwrap();
    assert!(remaining_hist
        .iter()
        .any(|e| e["type"] == "UserMessage" && e["data"]["turn_id"] == 1));
    assert!(!remaining_hist
        .iter()
        .any(|e| e["type"] == "UserMessage" && e["data"]["turn_id"] == 2));

    // 6. Rename via PATCH /api/sessions/:id
    let rename_resp = client
        .patch(format!("{}/api/sessions/{}", base, session_id))
        .json(&json!({ "title": "Renamed Multi-Turn Session" }))
        .send()
        .await
        .unwrap();
    assert_eq!(rename_resp.status(), 200);
    let rename_json: Value = rename_resp.json().await.unwrap();
    assert_eq!(rename_json["status"], "renamed");
    assert_eq!(rename_json["id"], session_id);
    assert_eq!(rename_json["title"], "Renamed Multi-Turn Session");

    let sessions_after_rename: Vec<Value> = client
        .get(format!("{}/api/sessions", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let renamed_entry = sessions_after_rename
        .iter()
        .find(|s| s["id"] == session_id)
        .unwrap();
    assert_eq!(renamed_entry["task"], "Renamed Multi-Turn Session");

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

fn assert_tool_call_pairs_intact(messages: &[ChatMessage]) {
    let mut i = 0;
    while i < messages.len() {
        let m = &messages[i];
        assert_ne!(
            m.role, "tool",
            "Orphan tool message found at index {} (tool_call_id={:?})",
            i, m.tool_call_id
        );
        if m.role == "assistant" {
            if let Some(ref tcs) = m.tool_calls {
                if !tcs.is_empty() {
                    let expected_ids: Vec<&str> = tcs.iter().map(|tc| tc.id.as_str()).collect();
                    let mut seen_ids: Vec<&str> = Vec::new();
                    let mut j = i + 1;
                    while j < messages.len() && messages[j].role == "tool" {
                        seen_ids.push(
                            messages[j]
                                .tool_call_id
                                .as_deref()
                                .expect("tool message missing tool_call_id"),
                        );
                        j += 1;
                    }
                    assert_eq!(
                        seen_ids, expected_ids,
                        "Assistant tool_calls at index {} torn apart from tool responses",
                        i
                    );
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
}

fn make_tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        r#type: "function".to_string(),
        function: ToolFunctionCall {
            name: name.to_string(),
            arguments: arguments.to_string(),
        },
    }
}

#[test]
fn test_memory_config_defaults_and_pair_safe_compaction() {
    let defaults = MemoryConfig::default();
    assert_eq!(defaults.max_context_tokens, 128_000);
    assert_eq!(defaults.auto_compact_threshold_pct, 78);
    assert_eq!(defaults.preserve_recent_messages, 8);
    assert_eq!(defaults.prune_tool_outputs_over_bytes, 2048);
    assert!(defaults.summarize_compacted_turns);
    assert!(defaults.enable_working_memory);
    assert_eq!(defaults.max_tool_retries, 3);
    assert!(defaults.auto_detect_context);

    // Configure a tight token budget where preserve_recent_messages=3 lands in the middle of tool responses
    let mem = MemoryConfig {
        max_context_tokens: 450,
        auto_compact_threshold_pct: 60, // trigger at 270 tokens
        preserve_recent_messages: 3,
        prune_tool_outputs_over_bytes: 120,
        summarize_compacted_turns: true,
        enable_working_memory: true,
        max_tool_retries: 3,
        auto_detect_context: false,
    };

    let big_output_1 = "HEADER_A\n".to_string() + &"x".repeat(600) + "\nTAIL_A";
    let big_output_2 = "HEADER_B\n".to_string() + &"y".repeat(600) + "\nTAIL_B";

    let messages = vec![
        ChatMessage::system("System prompt for harness."),
        ChatMessage::user("Inspect project files and summarize."),
        ChatMessage::assistant(
            None,
            Some("Long reasoning trace that should be pruned in stage 1.".to_string()),
            Some(vec![
                make_tool_call("call_1", "read_file", r#"{"path":"a.rs"}"#),
                make_tool_call("call_2", "read_file", r#"{"path":"b.rs"}"#),
            ]),
        ),
        ChatMessage::tool("call_1", big_output_1),
        ChatMessage::tool("call_2", big_output_2),
        ChatMessage::assistant(Some("Found A and B.".into()), None, None),
        ChatMessage::user("Now check c.rs and d.rs as well."),
        ChatMessage::assistant(
            None,
            None,
            Some(vec![
                make_tool_call("call_3", "read_file", r#"{"path":"c.rs"}"#),
                make_tool_call("call_4", "read_file", r#"{"path":"d.rs"}"#),
            ]),
        ),
        ChatMessage::tool("call_3", "Contents of c.rs ".repeat(30)),
        ChatMessage::tool("call_4", "Contents of d.rs ".repeat(30)),
        ChatMessage::assistant(Some("Done reading c.rs and d.rs.".into()), None, None),
    ];

    let mut ctx = ConversationContext::from_messages(messages, 8192).with_memory_config(mem.clone());
    let before = ctx.estimate_tokens();
    assert!(before > 270);

    let report = ctx
        .compact_if_needed()
        .expect("compact_if_needed should trigger over threshold");
    assert_eq!(report.before_tokens, before);
    assert!(report.after_tokens < report.before_tokens);
    assert!(report.messages_folded > 0);
    assert_eq!(ctx.compactions_count, 1);

    // Working memory block must be present right after system message
    assert_eq!(ctx.messages[0].role, "system");
    let wm_text = ctx.messages[1].content.as_deref().unwrap_or("");
    assert!(wm_text.starts_with("[COMPRESSED WORKING MEMORY]"));
    assert!(wm_text.contains("Goals:"));
    assert!(wm_text.contains("Tools:"));

    // Crucial invariant: no assistant tool_calls group is ever split from its tool responses
    assert_tool_call_pairs_intact(&ctx.messages);

    // Test force_compact on a context below threshold with > 2 atomic middle groups
    let mem_force = MemoryConfig {
        max_context_tokens: 128_000,
        auto_compact_threshold_pct: 90,
        preserve_recent_messages: 2,
        prune_tool_outputs_over_bytes: 2048,
        summarize_compacted_turns: true,
        enable_working_memory: true,
        max_tool_retries: 3,
        auto_detect_context: false,
    };
    let force_msgs = vec![
        ChatMessage::system("System"),
        ChatMessage::user("Turn 1 request"),
        ChatMessage::assistant(Some("Turn 1 answer".into()), None, None),
        ChatMessage::user("Turn 2 request"),
        ChatMessage::assistant(
            None,
            None,
            Some(vec![make_tool_call("c1", "list_dir", r#"{"path":"."}"#)]),
        ),
        ChatMessage::tool("c1", "src/\nCargo.toml"),
        ChatMessage::user("Turn 3 request"),
        ChatMessage::assistant(Some("Turn 3 answer".into()), None, None),
    ];
    let mut force_ctx = ConversationContext::from_messages(force_msgs, 8192).with_memory_config(mem_force);
    assert!(force_ctx.compact_if_needed().is_none());
    let force_report = force_ctx
        .force_compact()
        .expect("force_compact should fold older groups when > 2 groups exist");
    assert!(force_report.messages_folded >= 1);
    assert!(force_ctx.messages.iter().any(|m| {
        m.content
            .as_deref()
            .is_some_and(|c| c.starts_with("[COMPRESSED WORKING MEMORY]"))
    }));
    assert_tool_call_pairs_intact(&force_ctx.messages);
}

#[tokio::test]
async fn test_google_ai_search_mcp_server_registration_and_coercion() {
    let cfg = HarnessConfig::default();
    let google_srv = cfg
        .mcp_servers
        .iter()
        .find(|s| s.id == "google-ai-search")
        .expect("default config must include google-ai-search MCP server");
    assert!(google_srv.enabled);
    assert_eq!(google_srv.transport, "builtin");
    assert_eq!(google_srv.command, "builtin://google-ai-search");

    // Registry automatically registers mcp_google_ai_search_search
    let registry = PluginRegistry::new(&cfg);
    let contract = registry
        .get_contract("mcp_google_ai_search_search")
        .expect("PluginRegistry::new must register mcp_google_ai_search_search");
    assert_eq!(contract.permission_level, ToolPermission::ReadOnly);

    let schema = contract.to_json_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["max_results"].is_object());
    assert!(schema["properties"]["mode"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "query"));

    // test_mcp_server succeeds immediately for builtin google-ai-search
    let test_res = test_mcp_server(google_srv).await;
    assert!(test_res.success);
    assert_eq!(test_res.server_id, "google-ai-search");
    assert_eq!(test_res.tools.len(), 1);
    assert_eq!(test_res.tools[0].name, "search");

    // Argument coercion: trims whitespace keys and coerces stringified scalars/arrays/objects
    let coerced = coerce_args_for_contract(
        &contract,
        json!({
            " query ": "rust async traits",
            " max_results ": "4",
            "mode": "web"
        }),
    );
    assert_eq!(coerced["query"], "rust async traits");
    assert_eq!(coerced["max_results"], 4);
    assert_eq!(coerced["mode"], "web");
    assert!(contract.validate_args(&coerced).is_ok());

    // Coercion inside PluginRegistry::execute_with_context on read_file with stringified line numbers
    let temp_dir = std::env::temp_dir().join(format!("harness_coerce_test_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    tokio::fs::write(temp_dir.join("sample.txt"), "line1\nline2\nline3\n")
        .await
        .unwrap();

    let read_out = registry
        .execute_with_context(
            "read_file",
            json!({
                " path ": "sample.txt",
                "start_line": "2",
                "end_line": 2.0
            }),
            &temp_dir,
            None,
        )
        .await
        .expect("execute_with_context should coerce stringified & float-int line bounds");
    assert!(read_out.contains("line2"));
    assert!(!read_out.contains("line1"));

    // Additional scalar/compound coercion checks on a synthetic contract
    let synthetic = ToolContract {
        name: "synthetic".into(),
        description: "synthetic".into(),
        category: "test".into(),
        permission_level: ToolPermission::ReadOnly,
        fields: vec![
            ToolFieldSpec::required_bool("flag", "bool flag", "provide boolean"),
            ToolFieldSpec::required_number("ratio", "float ratio", "provide number"),
            ToolFieldSpec::required_array("items", "string list", "provide array"),
            ToolFieldSpec::required_object("meta", "metadata object", "provide object"),
        ],
        expected_output: OutputContract::new("text", "ok", "ok"),
        failure_modes: vec![],
        timeout_secs: 30,
        is_custom: false,
    };
    let coerced_synth = coerce_args_for_contract(
        &synthetic,
        json!({
            "flag": "TRUE",
            "ratio": "0.75",
            "items": r#"["a","b"]"#,
            "meta": r#"{"k":1}"#
        }),
    );
    assert_eq!(coerced_synth["flag"], true);
    assert_eq!(coerced_synth["ratio"], 0.75);
    assert_eq!(coerced_synth["items"], json!(["a", "b"]));
    assert_eq!(coerced_synth["meta"], json!({"k": 1}));
    assert!(synthetic.validate_args(&coerced_synth).is_ok());

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_recovery_helpers_and_agent_loop_integration() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    // 1. parse_and_repair_tool_args
    let repaired = parse_and_repair_tool_args(
        "Here are the args:\n```json\n{\n  \"path\": \"a,b.txt\",\n  \"start_line\": 1,\n}\n```",
    )
    .expect("should strip fences and trailing comma");
    assert_eq!(repaired["path"], "a,b.txt");
    assert_eq!(repaired["start_line"], 1);

    let double_encoded =
        parse_and_repair_tool_args(r#""{\"query\":\"rust\",\"max_results\":3}""#)
            .expect("should unwrap double-encoded JSON object");
    assert_eq!(double_encoded["query"], "rust");
    assert_eq!(double_encoded["max_results"], 3);

    assert!(parse_and_repair_tool_args("").is_err());
    assert!(parse_and_repair_tool_args("not json at all").is_err());

    // 2. diagnose_tool_error classification & retryability
    let google_contract = PluginRegistry::new(&HarnessConfig::default())
        .get_contract("mcp_google_ai_search_search")
        .unwrap();

    let val_diag = diagnose_tool_error(
        "mcp_google_ai_search_search",
        "Validation error: missing required field 'query'",
        Some(&google_contract),
    );
    assert_eq!(val_diag.kind, ToolErrorKind::Validation);
    assert!(!val_diag.is_retryable);
    assert!(val_diag.recovery_hint.contains("query"));

    let net_diag = diagnose_tool_error(
        "mcp_google_ai_search_search",
        "HTTP 503 Service Unavailable: upstream connection reset",
        Some(&google_contract),
    );
    assert_eq!(net_diag.kind, ToolErrorKind::TransientNetwork);
    assert!(net_diag.is_retryable);

    let bash_timeout_diag = diagnose_tool_error("bash", "Tool execution timed out after 30s", None);
    assert_eq!(bash_timeout_diag.kind, ToolErrorKind::Timeout);
    assert!(!bash_timeout_diag.is_retryable);
    assert!(!is_idempotent_retryable_tool("bash"));
    assert!(is_idempotent_retryable_tool("web_search"));
    assert!(is_idempotent_retryable_tool("mcp_google_ai_search_search"));

    let edit_not_found = diagnose_tool_error("edit_file", "Target string not found in file", None);
    assert_eq!(edit_not_found.kind, ToolErrorKind::NotFound);
    assert!(edit_not_found.recovery_hint.contains("read_file"));

    // 3. AgentLoopRunner integration: arg repair + transient retry + circuit breaker + auto-compaction
    struct FlakyWebSearchPlugin {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ToolPlugin for FlakyWebSearchPlugin {
        fn name(&self) -> &str {
            "web_search"
        }
        fn description(&self) -> &str {
            "Mock flaky idempotent search tool"
        }
        fn contract(&self) -> ToolContract {
            ToolContract {
                name: "web_search".into(),
                description: "Mock flaky search".into(),
                category: "web".into(),
                permission_level: ToolPermission::ReadOnly,
                fields: vec![ToolFieldSpec::required_string(
                    "query",
                    "search query",
                    "provide non-empty query",
                )],
                expected_output: OutputContract::new("text", "search results", "hit"),
                failure_modes: vec![],
                timeout_secs: 30,
                is_custom: false,
            }
        }
        async fn execute(&self, args: Value, _sandbox: &std::path::Path) -> Result<String, String> {
            let n = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                Err("503 Service Unavailable: transient connection reset".to_string())
            } else {
                Ok(format!(
                    "Recovered search hit for {}",
                    args["query"].as_str().unwrap_or("")
                ))
            }
        }
    }

    struct ScriptedRecoveryModel {
        step: Arc<AtomicUsize>,
        saw_circuit_breaker: Arc<std::sync::Mutex<bool>>,
    }

    #[async_trait::async_trait]
    impl ModelAdapter for ScriptedRecoveryModel {
        async fn stream_completion(
            &self,
            messages: &[ChatMessage],
            _tools: &Value,
            _event_tx: &mpsc::Sender<HarnessEvent>,
        ) -> Result<ModelTurnResult, String> {
            let cur = self.step.fetch_add(1, Ordering::SeqCst);
            if messages.iter().any(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("Self-Correction Alert"))
            }) {
                *self.saw_circuit_breaker.lock().unwrap() = true;
            }
            match cur {
                // Turn 0: malformed fenced+trailing-comma args for retryable web_search
                0 => Ok(ModelTurnResult {
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![make_tool_call(
                        "tc_ws",
                        "web_search",
                        "```json\n{\"query\": \"rust recovery\",}\n```",
                    )],
                    finish_reason: Some("tool_calls".into()),
                    estimated_tokens: 50,
                    ..Default::default()
                }),
                // Turn 1: repeated failing read_file #1 (non-retryable NotFound)
                1 => Ok(ModelTurnResult {
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![make_tool_call(
                        "tc_rf_1",
                        "read_file",
                        r#"{"path":"does_not_exist_123.txt"}"#,
                    )],
                    finish_reason: Some("tool_calls".into()),
                    estimated_tokens: 50,
                    ..Default::default()
                }),
                // Turn 2: repeated identical read_file #2 -> triggers circuit breaker alert
                2 => Ok(ModelTurnResult {
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![make_tool_call(
                        "tc_rf_2",
                        "read_file",
                        r#"{"path":"does_not_exist_123.txt"}"#,
                    )],
                    finish_reason: Some("tool_calls".into()),
                    estimated_tokens: 50,
                    ..Default::default()
                }),
                // Turn 3: final answer after observing circuit breaker & compaction
                _ => Ok(ModelTurnResult {
                    content: Some("Completed with recovery.".into()),
                    reasoning_content: None,
                    tool_calls: vec![],
                    finish_reason: Some("stop".into()),
                    estimated_tokens: 30,
                    ..Default::default()
                }),
            }
        }
    }

    let temp_dir = std::env::temp_dir().join(format!("harness_recovery_loop_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    // Low token budget so pre-model-call auto_threshold compaction triggers during the multi-step loop
    cfg.memory = MemoryConfig {
        max_context_tokens: 700,
        auto_compact_threshold_pct: 55,
        preserve_recent_messages: 4,
        prune_tool_outputs_over_bytes: 512,
        summarize_compacted_turns: true,
        enable_working_memory: true,
        max_tool_retries: 1,
        auto_detect_context: false,
    };

    let attempts = Arc::new(AtomicUsize::new(0));
    let mut registry = PluginRegistry::new(&cfg);
    registry.register(Arc::new(FlakyWebSearchPlugin {
        attempts: attempts.clone(),
    }));

    let saw_circuit_breaker = Arc::new(std::sync::Mutex::new(false));
    let model = Arc::new(ScriptedRecoveryModel {
        step: Arc::new(AtomicUsize::new(0)),
        saw_circuit_breaker: saw_circuit_breaker.clone(),
    });

    let runner = AgentLoopRunner::new(
        "test_recovery_session".into(),
        "Test recovery and auto compaction".into(),
        cfg,
        registry,
        model,
        None,
    );

    let (tx, mut rx) = mpsc::channel(128);
    let handle = tokio::spawn(async move { runner.run(tx).await });

    let mut events = Vec::new();
    while let Some(evt) = rx.recv().await {
        events.push(evt);
    }
    let final_answer = handle.await.unwrap().expect("runner should succeed");
    assert_eq!(final_answer, "Completed with recovery.");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert!(
        *saw_circuit_breaker.lock().unwrap(),
        "Circuit breaker Self-Correction Alert must be injected after repeated identical failures"
    );

    // Verify ToolRetryScheduled emitted for web_search attempt 2/2
    assert!(events.iter().any(|e| matches!(
        e,
        HarnessEvent::ToolRetryScheduled { tool, attempt: 2, max_attempts: 2, .. } if tool == "web_search"
    )));

    // Verify web_search ultimately finished successfully
    assert!(events.iter().any(|e| matches!(
        e,
        HarnessEvent::ToolCallFinished { tool, success: true, output, .. }
            if tool == "web_search" && output.contains("Recovered search hit for rust recovery")
    )));

    // Verify read_file failure includes [TOOL RECOVERY HINT]
    assert!(events.iter().any(|e| matches!(
        e,
        HarnessEvent::ToolCallFinished { tool, success: false, output, .. }
            if tool == "read_file" && output.contains("[TOOL RECOVERY HINT]")
    )));

    // Verify automatic compaction fired during loop
    assert!(events.iter().any(|e| matches!(
        e,
        HarnessEvent::MemoryCompacted { reason, .. } if reason == "auto_threshold"
    )));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_compact_session_and_config_api_endpoints() {
    let temp_dir = std::env::temp_dir().join(format!("harness_compact_api_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. GET /api/config returns memory defaults and google-ai-search MCP server
    let cfg_json: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_json["memory"]["max_context_tokens"], 128_000);
    assert_eq!(cfg_json["memory"]["auto_compact_threshold_pct"], 78);
    assert_eq!(cfg_json["memory"]["preserve_recent_messages"], 8);
    assert_eq!(cfg_json["memory"]["prune_tool_outputs_over_bytes"], 2048);
    assert_eq!(cfg_json["memory"]["summarize_compacted_turns"], true);
    assert_eq!(cfg_json["memory"]["enable_working_memory"], true);
    assert_eq!(cfg_json["memory"]["max_tool_retries"], 3);
    assert_eq!(cfg_json["memory"]["auto_detect_context"], true);

    let mcp_servers = cfg_json["mcp_servers"].as_array().unwrap();
    let google_mcp = mcp_servers
        .iter()
        .find(|s| s["id"] == "google-ai-search")
        .expect("GET /api/config must expose google-ai-search MCP server");
    assert_eq!(google_mcp["enabled"], true);
    assert_eq!(google_mcp["transport"], "builtin");

    // 2. POST /api/mcp/google-ai-search/test discovers search tool
    let mcp_test_resp = client
        .post(format!("{}/api/mcp/google-ai-search/test", base))
        .send()
        .await
        .unwrap();
    assert_eq!(mcp_test_resp.status(), 200);
    let mcp_test_json: Value = mcp_test_resp.json().await.unwrap();
    assert_eq!(mcp_test_json["success"], true);
    assert_eq!(mcp_test_json["server_id"], "google-ai-search");
    assert_eq!(mcp_test_json["tools"][0]["name"], "search");

    // 3. POST /api/sessions/missing/compact -> 404
    let missing_resp = client
        .post(format!("{}/api/sessions/does_not_exist/compact", base))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_resp.status(), 404);

    // 4. Create a session, seed rich llm_messages + history, and call POST /api/sessions/:id/compact
    let create_resp: Value = client
        .post(format!("{}/api/sessions", base))
        .json(&json!({ "task": "Compact API test", "mock": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = create_resp["session_id"].as_str().unwrap().to_string();

    for _ in 0..80 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if sessions
            .iter()
            .any(|s| s["id"] == session_id && s["status"] != "running")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    {
        let mut lock = state.sessions.write().await;
        let sess = lock.get_mut(&session_id).unwrap();
        sess.llm_messages = vec![
            ChatMessage::system("System"),
            ChatMessage::user("Turn 1"),
            ChatMessage::assistant(Some("Resp 1 ".repeat(80)), None, None),
            ChatMessage::user("Turn 2"),
            ChatMessage::assistant(
                None,
                None,
                Some(vec![make_tool_call("c1", "read_file", r#"{"path":"a"}"#)]),
            ),
            ChatMessage::tool("c1", "Big tool output ".repeat(300)),
            ChatMessage::user("Turn 3"),
            ChatMessage::assistant(Some("Resp 3 ".repeat(40)), None, None),
            ChatMessage::user("Turn 4"),
            ChatMessage::assistant(Some("Resp 4".into()), None, None),
            ChatMessage::user("Turn 5"),
            ChatMessage::assistant(Some("Resp 5".into()), None, None),
            ChatMessage::user("Turn 6"),
            ChatMessage::assistant(Some("Resp 6".into()), None, None),
            ChatMessage::user("Turn 7"),
            ChatMessage::assistant(Some("Resp 7".into()), None, None),
        ];
        for i in 0..14 {
            sess.history.push(HarnessEvent::MessageDelta {
                delta: format!("extra-{}", i),
            });
        }
    }

    let compact_resp = client
        .post(format!("{}/api/sessions/{}/compact", base, session_id))
        .send()
        .await
        .unwrap();
    assert_eq!(compact_resp.status(), 200);
    let compact_json: Value = compact_resp.json().await.unwrap();
    assert_eq!(compact_json["session_id"], session_id);
    assert_eq!(compact_json["status"], "compacted");
    let before_tokens = compact_json["before_tokens"].as_u64().unwrap();
    let after_tokens = compact_json["after_tokens"].as_u64().unwrap();
    assert!(before_tokens > after_tokens);
    assert!(compact_json["messages_folded"].as_u64().unwrap() > 0);
    assert!(compact_json["events_compacted"].as_u64().unwrap() > 0);

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_reassign_project_sessions_preserves_sessions() {
    let cfg = HarnessConfig::default();
    let state = AppState::new(cfg);

    state
        .create_session("ses-a".into(), "Task A".into(), "proj-temp".into())
        .await;
    state
        .create_session("ses-b".into(), "Task B".into(), "proj-temp".into())
        .await;
    state
        .create_session("ses-c".into(), "Task C".into(), "harness".into())
        .await;

    let moved = state
        .reassign_project_sessions("proj-temp", "harness")
        .await;
    assert_eq!(moved, 2);

    let lock = state.sessions.read().await;
    assert_eq!(lock.len(), 3);
    assert_eq!(lock.get("ses-a").unwrap().project_id, "harness");
    assert_eq!(lock.get("ses-b").unwrap().project_id, "harness");
    assert_eq!(lock.get("ses-c").unwrap().project_id, "harness");
}

#[tokio::test]
async fn test_model_reasoning_capabilities_endpoint() {
    let cfg = HarnessConfig::default();
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // Configurable (low, medium, high)
    let gpt_oss: Value = client
        .get(format!("{}/api/models/reasoning?model=openai/gpt-oss-20b", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(gpt_oss["support"], "configurable");
    assert_eq!(gpt_oss["levels"], json!(["low", "medium", "high"]));

    // Configurable including none (Gemini 2.5 Flash allows disabling thinking)
    let gem: Value = client
        .get(format!("{}/api/models/reasoning?model=google/gemini-2.5-flash", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(gem["support"], "configurable");
    assert_eq!(gem["levels"], json!(["none", "low", "medium", "high"]));

    // Configurable including minimal (GPT-5 exposes minimal..high)
    let gpt5: Value = client
        .get(format!("{}/api/models/reasoning?model=openai/gpt-5", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(gpt5["support"], "configurable");
    assert_eq!(gpt5["levels"], json!(["minimal", "low", "medium", "high"]));

    // Fixed always-on reasoning
    let r1: Value = client
        .get(format!("{}/api/models/reasoning?model=deepseek-ai/deepseek-r1", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r1["support"], "fixed");
    assert_eq!(r1["levels"], json!([]));

    // Non-reasoning model (no reasoning levels exposed)
    let gemma: Value = client
        .get(format!("{}/api/models/reasoning?model=google/gemma-3-12b-it", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(gemma["support"], "none");
    assert_eq!(gemma["levels"], json!([]));

    server.abort();
}

#[tokio::test]
async fn test_display_verbosity_default_and_config_normalization() {
    use deepseek_harness_rs::config::normalize_display_verbosity;

    let default_cfg = HarnessConfig::default();
    assert_eq!(default_cfg.display_verbosity, "compact");
    assert_eq!(normalize_display_verbosity("compact"), "compact");
    assert_eq!(normalize_display_verbosity("  BALANCED "), "balanced");
    assert_eq!(normalize_display_verbosity("Verbose"), "verbose");
    assert_eq!(normalize_display_verbosity("unknown-mode"), "compact");
    assert_eq!(normalize_display_verbosity(""), "compact");

    let mut cfg = HarnessConfig::default();
    cfg.mock_mode = true;
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // GET /api/config returns default "compact"
    let mut cfg_json: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_json["display_verbosity"], "compact");

    // POST /api/config normalizes uppercase/mixed valid verbosity
    cfg_json["display_verbosity"] = json!("  VERBOSE ");
    let updated_verbose: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_verbose["display_verbosity"], "verbose");

    // POST /api/config normalizes invalid verbosity back to "compact"
    cfg_json["display_verbosity"] = json!("noisy_invalid");
    let updated_invalid: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_invalid["display_verbosity"], "compact");

    // Persisted value reflects normalized "compact"
    let after_get: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_get["display_verbosity"], "compact");

    server.abort();
}

#[tokio::test]
async fn test_send_mode_default_and_config_normalization() {
    use deepseek_harness_rs::config::normalize_send_mode;

    let default_cfg = HarnessConfig::default();
    assert_eq!(default_cfg.send_mode, "queue");
    assert!(!default_cfg.voice_activity_detection);
    assert_eq!(normalize_send_mode("queue"), "queue");
    assert_eq!(normalize_send_mode("  STEER "), "steer");
    assert_eq!(normalize_send_mode("unknown"), "queue");
    assert_eq!(normalize_send_mode(""), "queue");

    let mut cfg = HarnessConfig::default();
    cfg.mock_mode = true;
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let mut cfg_json: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_json["send_mode"], "queue");
    assert_eq!(cfg_json["voice_activity_detection"], false);

    cfg_json["send_mode"] = json!("  STEER ");
    cfg_json["voice_activity_detection"] = json!(true);
    let updated_steer: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_steer["send_mode"], "steer");
    assert_eq!(updated_steer["voice_activity_detection"], true);

    cfg_json["send_mode"] = json!("invalid_mode");
    let updated_invalid: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_invalid["send_mode"], "queue");

    server.abort();
}

#[tokio::test]
async fn test_slash_command_system_directive_and_no_bogus_user_message() {
    let temp_dir = std::env::temp_dir().join(format!("harness_slash_cmd_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    let state = AppState::new(cfg);

    // 1. Unit-level state check: empty prompt + slash_command emits SystemCommand + system ChatMessage, NO UserMessage
    let (bcast_tx, _cancel) = state
        .create_session("ses_unit_slash".into(), "/plan".into(), "harness".into())
        .await;
    let _rx = bcast_tx.subscribe();
    let (turn_id, prior_msgs, _steer) = state
        .begin_turn_with_command("ses_unit_slash", "", Some("plan"), Some("design auth"))
        .await
        .expect("begin_turn_with_command should succeed");
    assert_eq!(turn_id, 1);
    assert!(
        prior_msgs.iter().any(|m| {
            m.role == "system"
                && m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("[SYSTEM COMMAND DIRECTIVE /plan]") && c.contains("design auth"))
        }),
        "Expected ChatMessage::system directive in llm_messages, got: {:?}",
        prior_msgs
    );
    assert!(
        !prior_msgs.iter().any(|m| m.role == "user"),
        "Empty prose slash command must not inject a bogus user ChatMessage"
    );
    {
        let lock = state.sessions.read().await;
        let sess = lock.get("ses_unit_slash").unwrap();
        assert!(sess.history.iter().any(|e| matches!(
            e,
            HarnessEvent::SystemCommand { command, args, icon, .. }
            if command == "plan" && args == "design auth" && icon == "plan"
        )));
        assert!(
            !sess.history.iter().any(|e| matches!(e, HarnessEvent::UserMessage { .. })),
            "Empty prose slash command must not emit UserMessage event"
        );
    }

    // 2. End-to-end HTTP test: POST /api/sessions and POST /api/sessions/:id/message
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 2a. Pure slash payload (task = "", slash_command = "plan", slash_args = "api schema")
    let create_resp: Value = client
        .post(format!("{}/api/sessions", base))
        .json(&json!({
            "task": "",
            "slash_command": "plan",
            "slash_args": "api schema",
            "mock": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ses_id = create_resp["session_id"].as_str().unwrap().to_string();

    // Wait for mock turn completion
    for _ in 0..80 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if sessions
            .iter()
            .any(|s| s["id"] == ses_id && s["status"] != "running")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let history_1: Vec<Value> = client
        .get(format!("{}/api/sessions/{}/history", base, ses_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        history_1.iter().any(|e| {
            e["type"] == "SystemCommand"
                && e["data"]["command"] == "plan"
                && e["data"]["args"] == "api schema"
        }),
        "Expected SystemCommand in history: {:?}",
        history_1
    );
    assert!(
        !history_1.iter().any(|e| e["type"] == "UserMessage"),
        "Must NOT emit UserMessage when slash command has empty prose: {:?}",
        history_1
    );

    // Verify llm_messages retained the system directive alongside base system prompt
    {
        let lock = state.sessions.read().await;
        let sess = lock.get(&ses_id).unwrap();
        assert!(sess.llm_messages.iter().any(|m| {
            m.role == "system"
                && m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("[SYSTEM COMMAND DIRECTIVE /plan]"))
        }));
    }

    // 2b. Follow-up message with leading `/review src/lib.rs` (slash_command omitted) -> SystemCommand, no UserMessage
    let msg_resp = client
        .post(format!("{}/api/sessions/{}/message", base, ses_id))
        .json(&json!({
            "content": "/review src/lib.rs",
            "mock": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg_resp.status(), 200);

    for _ in 0..80 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if sessions
            .iter()
            .any(|s| s["id"] == ses_id && s["status"] != "running")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // 2c. Follow-up message with explicit slash_command + non-empty prose -> BOTH SystemCommand and UserMessage
    let msg_with_prose = client
        .post(format!("{}/api/sessions/{}/message", base, ses_id))
        .json(&json!({
            "content": "Focus on auth middleware",
            "slash_command": "debug",
            "slash_args": "token expiry",
            "mock": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg_with_prose.status(), 200);

    for _ in 0..80 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if sessions
            .iter()
            .any(|s| s["id"] == ses_id && s["status"] != "running")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // 2d. Plain Unix path `/etc/hosts` must NOT be parsed as a slash command
    let path_msg = client
        .post(format!("{}/api/sessions/{}/message", base, ses_id))
        .json(&json!({
            "content": "/etc/hosts check",
            "mock": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(path_msg.status(), 200);

    for _ in 0..80 {
        let sessions: Vec<Value> = client
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if sessions
            .iter()
            .any(|s| s["id"] == ses_id && s["status"] != "running")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let history_final: Vec<Value> = client
        .get(format!("{}/api/sessions/{}/history", base, ses_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let sys_cmds: Vec<&Value> = history_final
        .iter()
        .filter(|e| e["type"] == "SystemCommand")
        .collect();
    assert_eq!(sys_cmds.len(), 3, "Expected plan, review, debug SystemCommand events");
    assert_eq!(sys_cmds[1]["data"]["command"], "review");
    assert_eq!(sys_cmds[1]["data"]["args"], "src/lib.rs");
    assert_eq!(sys_cmds[2]["data"]["command"], "debug");
    assert_eq!(sys_cmds[2]["data"]["args"], "token expiry");

    let user_msgs: Vec<&Value> = history_final
        .iter()
        .filter(|e| e["type"] == "UserMessage")
        .collect();
    assert_eq!(user_msgs.len(), 2, "Expected UserMessage only for prose + /etc/hosts");
    assert_eq!(user_msgs[0]["data"]["content"], "Focus on auth middleware");
    assert_eq!(user_msgs[1]["data"]["content"], "/etc/hosts check");

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_swarm_tags_compact_digests_and_swarm_endpoint() {
    let temp_dir = std::env::temp_dir().join(format!("harness_swarm_tags_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    let state = AppState::new(cfg.clone());
    let session_id = "ses_swarm_tags_1".to_string();
    let (bcast_tx, cancel_notify) = state
        .create_session(session_id.clone(), "Swarm tags test".into(), "harness".into())
        .await;
    let swarm = state.get_or_create_swarm(&session_id).await;
    let registry_snapshot = state.get_registry_snapshot().await;

    let ctx = SessionToolContext {
        session_id: session_id.clone(),
        agent_id: "coordinator".to_string(),
        parent_id: None,
        depth: 0,
        max_depth: 2,
        state: Some(state.clone()),
        swarm: Some(swarm.clone()),
        event_tx: None,
        config: cfg.clone(),
        registry: registry_snapshot.clone(),
        model: Arc::new(MockClient),
        cancel_notify,
        sandbox_roots: Vec::new(),
    };
    let _ = bcast_tx;

    // 1. Create (job) tag via swarm_board
    let create_job_res = registry_snapshot
        .execute_with_context(
            "swarm_board",
            json!({
                "action": "create_job",
                "job_tag": "auth-audit",
                "title": "Auth Audit",
                "description": "Verify token handling and session middleware",
                "status": "open"
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("create_job should succeed");
    assert!(create_job_res.contains("auth-audit"));

    // 2. Create (member specialist) tag via swarm_board
    let create_spec_res = registry_snapshot
        .execute_with_context(
            "swarm_board",
            json!({
                "action": "create_specialist",
                "specialist": "security_reviewer",
                "title": "Security Reviewer",
                "specialty": "Check OWASP vulnerabilities and permission guards strictly.",
                "read_only": true
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("create_specialist should succeed");
    assert!(create_spec_res.contains("security_reviewer"));

    // 3. Invoke subagent with a long multi-line prompt and job_tag + specialist -> compact digests & tagged SubagentInfo
    let long_prompt = format!(
        "Line 1: Inspect auth module\nLine 2: Check edge cases\n{}",
        "detail ".repeat(80)
    );
    let invoke_res = registry_snapshot
        .execute_with_context(
            "invoke_subagent",
            json!({
                "role": "auditor",
                "prompt": long_prompt,
                "job_tag": "auth-audit",
                "specialist": "security_reviewer",
                "wait": true
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("invoke_subagent with tags should succeed");
    assert!(invoke_res.contains("auditor"));

    let subagents = swarm.list_subagents().await;
    assert_eq!(subagents.len(), 1);
    let sub = &subagents[0];
    assert_eq!(sub.job_tag.as_deref(), Some("auth-audit"));
    assert_eq!(sub.specialist.as_deref(), Some("security_reviewer"));
    assert!(
        sub.task.chars().count() <= 220 && !sub.task.contains('\n'),
        "SubagentInfo.task must be collapsed single-line digest <= 220 chars, got len={} task={:?}",
        sub.task.chars().count(),
        sub.task
    );
    assert!(
        sub.summary.chars().count() <= 220 && !sub.summary.contains('\n'),
        "SubagentInfo.summary must be collapsed single-line digest <= 220 chars, got len={} summary={:?}",
        sub.summary.chars().count(),
        sub.summary
    );

    // Job auto-transitions from "open" -> "in_progress" on invocation
    let job_after_spawn = swarm.get_job("auth-audit").await.unwrap();
    assert_eq!(job_after_spawn.status, "in_progress");

    // 4. assign_job to existing subagent updates tags & posts compact tagged board message
    let assign_res = registry_snapshot
        .execute_with_context(
            "swarm_board",
            json!({
                "action": "assign_job",
                "job_tag": "release-signoff",
                "title": "Release Signoff",
                "status": "done",
                "target": sub.id,
                "content": format!("Long assignment note:\n{}", "word ".repeat(80))
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("assign_job should succeed");
    assert!(assign_res.contains("release-signoff"));

    let updated_sub = swarm.get_subagent(&sub.id).await.unwrap();
    assert_eq!(updated_sub.info.job_tag.as_deref(), Some("release-signoff"));

    let board_msgs = swarm.all_messages().await;
    assert!(!board_msgs.is_empty());
    for msg in &board_msgs {
        assert!(
            msg.content.chars().count() <= 220 && !msg.content.contains('\n'),
            "SwarmMessage.content must be compact <= 220 chars single-line: {:?}",
            msg.content
        );
    }

    // 5. Verify GET /api/sessions/:id/swarm JSON shape exposes jobs, specialists, subagents, board_messages
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let swarm_resp = client
        .get(format!("http://{}/api/sessions/{}/swarm", addr, session_id))
        .send()
        .await
        .unwrap();
    assert_eq!(swarm_resp.status(), 200);
    let swarm_json: Value = swarm_resp.json().await.unwrap();

    assert!(swarm_json["subagents"].as_array().unwrap().len() == 1);
    assert_eq!(swarm_json["subagents"][0]["job_tag"], "release-signoff");
    assert_eq!(swarm_json["subagents"][0]["specialist"], "security_reviewer");
    assert!(!swarm_json["board_messages"].as_array().unwrap().is_empty());

    let jobs = swarm_json["jobs"].as_array().unwrap();
    assert!(jobs.iter().any(|j| j["id"] == "auth-audit" && j["status"] == "in_progress"));
    assert!(jobs.iter().any(|j| j["id"] == "release-signoff" && j["status"] == "done"));

    let specialists = swarm_json["specialists"].as_array().unwrap();
    assert!(specialists.iter().any(|s| {
        s["id"] == "security_reviewer"
            && s["name"] == "Security Reviewer"
            && s["default_read_only"] == true
    }));

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_browser_ui_debugger_actions_and_schema() {
    let plugin = BrowserUiPlugin::new();
    let schema = plugin.parameter_schema();
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    for expected in [
        "navigate",
        "snapshot",
        "debug_audit",
        "storage",
        "screenshot",
        "eval",
        "inspect_dom",
        "interact",
        "http_check",
    ] {
        assert!(
            actions.iter().any(|v| v == expected),
            "Missing browser_ui action '{}' in schema",
            expected
        );
    }

    let props = schema["properties"].as_object().unwrap();
    for field in [
        "preset",
        "full_page",
        "element_selector",
        "steps",
        "actions",
        "storage_op",
        "storage_type",
        "storage_key",
    ] {
        assert!(
            props.contains_key(field),
            "Missing browser_ui property '{}' in schema",
            field
        );
    }

    let presets = schema["properties"]["preset"]["enum"].as_array().unwrap();
    for preset in ["desktop", "laptop", "tablet", "mobile"] {
        assert!(presets.iter().any(|v| v == preset));
    }

    let contract = plugin.contract();
    assert!(contract.fields.iter().any(|f| f.name == "steps"));
    assert!(contract.fields.iter().any(|f| f.name == "storage_op"));

    // Verify invalid URL scheme fails on snapshot / debug_audit / storage too
    let temp_dir = std::env::temp_dir().join(format!("harness_browser_dbg_{}", std::process::id()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    for act in ["snapshot", "debug_audit", "storage"] {
        let res = plugin
            .execute(json!({ "action": act, "url": "ftp://example.com" }), &temp_dir)
            .await;
        assert!(res.is_err(), "Expected scheme guard error for action={}", act);
    }
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

struct MultiStepUnlimitedModel {
    step: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl ModelAdapter for MultiStepUnlimitedModel {
    async fn stream_completion(
        &self,
        _messages: &[ChatMessage],
        _tools: &Value,
        _event_tx: &mpsc::Sender<HarnessEvent>,
    ) -> Result<ModelTurnResult, String> {
        let idx = self.step.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if idx < 4 {
            Ok(ModelTurnResult {
                content: None,
                reasoning_content: None,
                tool_calls: vec![make_tool_call(
                    &format!("unlimited_call_{}", idx),
                    "sys_info",
                    "{}",
                )],
                finish_reason: Some("tool_calls".into()),
                estimated_tokens: 40,
                ..Default::default()
            })
        } else {
            Ok(ModelTurnResult {
                content: Some("Completed after 5 iterations cleanly.".to_string()),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                estimated_tokens: 30,
                ..Default::default()
            })
        }
    }
}

#[tokio::test]
async fn test_unlimited_autonomous_turns_and_max_tool_retries_default() {
    let default_cfg = HarnessConfig::default();
    assert_eq!(default_cfg.max_iterations, 0);
    assert_eq!(default_cfg.memory.max_tool_retries, 3);
    assert!(default_cfg.memory.auto_detect_context);

    // Verify AgentLoopRunner runs past old bounds without hitting an iteration cap and emits max_steps: 0
    let mut cfg = HarnessConfig::default();
    cfg.max_iterations = 0;
    let registry = PluginRegistry::new(&cfg);
    let model = Arc::new(MultiStepUnlimitedModel {
        step: std::sync::atomic::AtomicUsize::new(0),
    });

    let runner = AgentLoopRunner::new(
        "test_unlimited_turns".into(),
        "Run 5 steps".into(),
        cfg.clone(),
        registry,
        model,
        None,
    );
    let (tx, mut rx) = mpsc::channel(128);
    let handle = tokio::spawn(async move { runner.run(tx).await });

    let mut steps_seen = Vec::new();
    let mut warnings = Vec::new();
    let mut completed_answer = None;
    while let Some(evt) = rx.recv().await {
        match evt {
            HarnessEvent::IterationStep { step, max_steps } => {
                steps_seen.push((step, max_steps));
            }
            HarnessEvent::Warning { message } => {
                warnings.push(message);
            }
            HarnessEvent::SessionCompleted { final_answer, .. } => {
                completed_answer = Some(final_answer);
            }
            _ => {}
        }
    }

    let final_res = handle.await.unwrap().expect("runner should complete cleanly");
    assert!(final_res.contains("Completed after 5 iterations"));
    assert_eq!(
        completed_answer.as_deref(),
        Some("Completed after 5 iterations cleanly.")
    );
    assert_eq!(steps_seen.len(), 5);
    assert!(steps_seen.iter().all(|(_, max_steps)| *max_steps == 0));
    assert!(
        !warnings.iter().any(|w| w.to_lowercase().contains("max iterations")),
        "Unexpected max iterations warning: {:?}",
        warnings
    );

    // Verify POST /api/config clamps max_tool_retries to 0..=10 (not 2)
    let state = AppState::new(cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let mut cfg_json: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_json["max_iterations"], 0);
    assert_eq!(cfg_json["memory"]["max_tool_retries"], 3);

    cfg_json["memory"]["max_tool_retries"] = json!(7);
    let updated_7: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_7["memory"]["max_tool_retries"], 7);

    cfg_json["memory"]["max_tool_retries"] = json!(99);
    let updated_clamped: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_clamped["memory"]["max_tool_retries"], 10);

    server.abort();
}

#[tokio::test]
async fn test_auto_context_window_detection_and_smart_compaction() {
    use deepseek_harness_rs::models::capabilities::{
        effective_max_context_tokens, infer_model_context_window,
    };

    // 1. Capabilities context window inference + auto_detect toggle
    assert_eq!(infer_model_context_window("google/gemini-2.5-flash", None), 1_048_576);
    assert_eq!(infer_model_context_window("anthropic/claude-sonnet-4", None), 200_000);
    assert_eq!(infer_model_context_window("openai/gpt-5", None), 400_000);
    assert_eq!(
        infer_model_context_window("custom/model", Some(&json!({ "context_length": 524_288 }))),
        524_288
    );
    assert_eq!(
        effective_max_context_tokens("google/gemini-2.5-flash", 64_000, true, None),
        1_048_576
    );
    assert_eq!(
        effective_max_context_tokens("google/gemini-2.5-flash", 64_000, false, None),
        64_000
    );

    // 2. Endpoint integration: GET /api/models/reasoning + GET/POST /api/config
    let mut cfg = HarnessConfig::default();
    cfg.model = "google/gemini-2.5-flash".to_string();
    cfg.memory.auto_detect_context = true;
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let reasoning_gem: Value = client
        .get(format!("{}/api/models/reasoning?model=google/gemini-2.5-flash", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reasoning_gem["support"], "configurable");
    assert_eq!(reasoning_gem["max_context_tokens"], 1_048_576);

    let mut cfg_json: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_json["memory"]["max_context_tokens"], 1_048_576);

    // Switching model with auto_detect_context=true updates max_context_tokens automatically
    cfg_json["model"] = json!("anthropic/claude-sonnet-4");
    let after_claude: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_claude["memory"]["max_context_tokens"], 200_000);

    // Disabling auto_detect_context preserves manual max_context_tokens
    cfg_json["memory"]["auto_detect_context"] = json!(false);
    cfg_json["memory"]["max_context_tokens"] = json!(65_536);
    let after_manual: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_manual["memory"]["max_context_tokens"], 65_536);

    server.abort();

    // 3. Smart compaction: deduplicates identical earlier read-only call and merges existing working memory
    let mem = MemoryConfig {
        max_context_tokens: 300,
        auto_compact_threshold_pct: 50,
        preserve_recent_messages: 3,
        prune_tool_outputs_over_bytes: 100,
        summarize_compacted_turns: true,
        enable_working_memory: true,
        max_tool_retries: 3,
        auto_detect_context: false,
    };
    let msgs = vec![
        ChatMessage::system("Base system prompt."),
        ChatMessage::system("[COMPACTED WORKING MEMORY]\nGoals: Initial audit"),
        ChatMessage::user("Read src/lib.rs"),
        ChatMessage::assistant(
            None,
            None,
            Some(vec![make_tool_call("old_read", "read_file", r#"{"path":"src/lib.rs"}"#)]),
        ),
        ChatMessage::tool("old_read", "Old huge file content ".repeat(40)),
        ChatMessage::user("Re-read src/lib.rs after edits"),
        ChatMessage::assistant(
            None,
            None,
            Some(vec![make_tool_call("new_read", "read_file", r#"{"path":"src/lib.rs"}"#)]),
        ),
        ChatMessage::tool("new_read", "Latest short content"),
    ];
    let mut ctx = ConversationContext::from_messages(msgs, 8192).with_memory_config(mem);
    let report = ctx.compact_if_needed().expect("smart compaction should trigger");
    assert!(report.after_tokens < report.before_tokens);
    assert_tool_call_pairs_intact(&ctx.messages);
    let wm_count = ctx
        .messages
        .iter()
        .filter(|m| {
            m.content
                .as_deref()
                .is_some_and(|c| c.contains("WORKING MEMORY]"))
        })
        .count();
    assert_eq!(wm_count, 1, "Working memory blocks must merge rather than stack");
}

#[test]
fn test_google_search_model_prompt_wraps_system_prompt_as_user_instruction() {
    use deepseek_harness_rs::models::google_search_client::{
        build_google_search_prompt, is_google_search_model, parse_google_search_response,
        EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID,
    };

    assert_eq!(EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID, "Google-Search-Model");
    assert!(is_google_search_model("Google-Search-Model"));
    assert!(is_google_search_model("google-search-model"));
    assert!(is_google_search_model("  experimental/google-search-model "));
    assert!(is_google_search_model("EXPERIMENTAL/GOOGLE-SEARCH-MODEL"));
    assert!(!is_google_search_model("gpt-4o"));
    assert!(!is_google_search_model("google/gemini-2.5-flash"));

    let messages = vec![
        ChatMessage::system("SYSTEM RULE 1: You are a strict Rust verification engineer."),
        ChatMessage::system("SYSTEM RULE 2: Cite exact file paths and never invent output."),
        ChatMessage::user("List files in src/models."),
        ChatMessage::assistant(
            Some("I will list src/models first.".into()),
            None,
            Some(vec![make_tool_call(
                "call_gs_0",
                "list_dir",
                r#"{"path":"src/models"}"#,
            )]),
        ),
        ChatMessage::tool("call_gs_0", "mod.rs\ngoogle_search_client.rs"),
        ChatMessage::user("Now read mod.rs and summarize the exports."),
    ];

    let tools = json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file from the workspace",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }
    ]);

    let prompt = build_google_search_prompt(&messages, &tools);

    // Special instruction + wrapped system prompt section
    assert!(prompt.contains("authoritative System Prompt"));
    let sys_open = prompt
        .find("\n--- BEGIN SYSTEM PROMPT ---\n")
        .expect("missing opening --- BEGIN SYSTEM PROMPT --- block");
    let sys_close = prompt
        .rfind("\n--- END SYSTEM PROMPT ---")
        .expect("missing closing --- END SYSTEM PROMPT --- block");
    assert!(sys_open < sys_close);

    let system_block = &prompt[sys_open..sys_close];
    assert!(system_block.contains("SYSTEM RULE 1: You are a strict Rust verification engineer."));
    assert!(system_block.contains("SYSTEM RULE 2: Cite exact file paths and never invent output."));
    assert!(system_block.contains("AVAILABLE TOOLS:"));
    assert!(system_block.contains("- read_file(path: string): Read a file from the workspace"));
    assert!(system_block.contains("<harness_tool_call>"));

    // User request + multi-turn history are included alongside the wrapped system prompt block
    let before_sys = &prompt[..sys_open];
    assert!(before_sys.contains("User Request:\nNow read mod.rs and summarize the exports."));
    assert!(before_sys.contains("[CONVERSATION HISTORY]"));
    assert!(before_sys.contains("User: List files in src/models."));
    assert!(before_sys.contains("Assistant: I will list src/models first."));
    assert!(before_sys.contains("<harness_tool_call>{\"arguments\":{\"path\":\"src/models\"},\"name\":\"list_dir\"}</harness_tool_call>"));
    assert!(before_sys.contains("Tool Result (call_gs_0): mod.rs\ngoogle_search_client.rs"));

    // Response parsing with tool calls + prose
    let raw_response = "Checking `src/models/mod.rs` now.\n<harness_tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"src/models/mod.rs\"}}</harness_tool_call>\nDone queuing tool.";
    let (prose, calls) = parse_google_search_response(raw_response, true);
    assert_eq!(
        prose.as_deref(),
        Some("Checking `src/models/mod.rs` now.\n\nDone queuing tool.")
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_gs_1");
    assert_eq!(calls[0].r#type, "function");
    assert_eq!(calls[0].function.name, "read_file");
    let parsed_args: Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
    assert_eq!(parsed_args, json!({ "path": "src/models/mod.rs" }));

    // Fenced + multiple calls
    let fenced = "<harness_tool_call>```json\n{\"name\":\"sys_info\",\"arguments\":{}}\n```</harness_tool_call>";
    let (fenced_prose, fenced_calls) = parse_google_search_response(fenced, true);
    assert_eq!(fenced_prose, None);
    assert_eq!(fenced_calls.len(), 1);
    assert_eq!(fenced_calls[0].function.name, "sys_info");
    assert_eq!(fenced_calls[0].function.arguments, "{}");

    // Disabled tools mode keeps tags as plain prose
    let (no_tool_prose, no_tool_calls) = parse_google_search_response(raw_response, false);
    assert!(no_tool_calls.is_empty());
    assert_eq!(no_tool_prose.as_deref(), Some(raw_response));
}

#[test]
fn test_google_search_model_captcha_detection_and_capabilities() {
    use deepseek_harness_rs::models::capabilities::{
        effective_max_context_tokens, infer_model_context_window, infer_model_reasoning_spec,
        ReasoningSupport,
    };
    use deepseek_harness_rs::models::google_search_client::detect_google_captcha_or_block;

    // CAPTCHA / anti-bot gates must be detected cleanly
    assert!(detect_google_captcha_or_block(
        "https://www.google.com/sorry/index?continue=https://www.google.com/search",
        "About this page"
    )
    .is_some());
    assert!(detect_google_captcha_or_block(
        "https://www.google.com/search?udm=50&hl=en",
        "Our systems have detected unusual traffic from your computer network."
    )
    .is_some());
    assert!(detect_google_captcha_or_block(
        "https://www.google.com/search?udm=50&hl=en",
        "Please verify you are human to continue."
    )
    .is_some());

    // Clean AI Mode page must not false-positive
    assert!(detect_google_captcha_or_block(
        "https://www.google.com/search?udm=50&hl=en",
        "AI Mode response: A CAPTCHA is a challenge-response test used to distinguish human users from bots."
    )
    .is_none());

    // Capabilities & context window
    let spec = infer_model_reasoning_spec("Google-Search-Model", None);
    assert_eq!(spec.support, ReasoningSupport::None);
    assert!(spec.levels.is_empty());
    assert_eq!(spec.default_level, None);
    assert_eq!(spec.source, "experimental_builtin");

    let exp_spec = infer_model_reasoning_spec("experimental/google-search-model", None);
    assert_eq!(exp_spec.support, ReasoningSupport::None);
    assert_eq!(exp_spec.source, "experimental_builtin");

    assert_eq!(infer_model_context_window("Google-Search-Model", None), 32_768);
    assert_eq!(
        infer_model_context_window("experimental/google-search-model", None),
        32_768
    );
    assert_eq!(
        effective_max_context_tokens("Google-Search-Model", 128_000, true, None),
        32_768
    );
}

#[tokio::test]
async fn test_google_search_model_web_endpoints() {
    let mut cfg = HarnessConfig::default();
    cfg.api_base = String::new();
    cfg.api_key = String::new();
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. GET /api/models/reasoning?model=Google-Search-Model
    let reasoning_resp = client
        .get(format!(
            "{}/api/models/reasoning?model=Google-Search-Model",
            base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(reasoning_resp.status(), reqwest::StatusCode::OK);
    let reasoning_json: Value = reasoning_resp.json().await.unwrap();
    assert_eq!(reasoning_json["experimental"], true);
    assert_eq!(reasoning_json["support"], "none");
    assert_eq!(reasoning_json["levels"], json!([]));
    assert_eq!(reasoning_json["source"], "experimental_builtin");
    assert_eq!(reasoning_json["max_context_tokens"], 32_768);

    // 2. GET /api/models/remote with empty api_base returns built-in experimental model
    let remote_resp = client
        .get(format!("{}/api/models/remote", base))
        .send()
        .await
        .unwrap();
    assert_eq!(remote_resp.status(), reqwest::StatusCode::OK);
    let remote_models: Vec<Value> = remote_resp.json().await.unwrap();
    let gs_entry = remote_models
        .iter()
        .find(|m| m["id"] == "Google-Search-Model")
        .expect("Google-Search-Model must be returned even with empty api_base");
    assert_eq!(gs_entry["experimental"], true);
    assert_eq!(gs_entry["owned_by"], "google-search-ai");
    assert_eq!(gs_entry["reasoning"]["support"], "none");
    assert_eq!(gs_entry["max_context_tokens"], 32_768);

    // 3. POST /api/config switching to Google-Search-Model auto-syncs context window to 32768
    let mut cfg_json: Value = client
        .get(format!("{}/api/config", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    cfg_json["model"] = json!("Google-Search-Model");
    cfg_json["memory"]["auto_detect_context"] = json!(true);
    let updated_cfg: Value = client
        .post(format!("{}/api/config", base))
        .json(&cfg_json)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_cfg["model"], "Google-Search-Model");
    assert_eq!(updated_cfg["memory"]["max_context_tokens"], 32_768);

    server.abort();
}

#[tokio::test]
async fn test_browser_ui_local_workspace_file_and_relative_url_guards() {
    let plugin = BrowserUiPlugin::new();
    let root = std::env::temp_dir().join(format!(
        "harness_browser_local_guard_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&root).await;
    let sandbox = root.join("workspace");
    let outside = root.join("outside");
    tokio::fs::create_dir_all(sandbox.join("sub")).await.unwrap();
    tokio::fs::create_dir_all(&outside).await.unwrap();

    tokio::fs::write(
        sandbox.join("index.html"),
        "<!doctype html><html><head><title>Workspace Home</title></head><body><h1 id=\"app\">Hello Local</h1></body></html>",
    )
    .await
    .unwrap();
    tokio::fs::write(
        sandbox.join("sub").join("page.html"),
        "<!doctype html><html><head><title>Sub Page</title></head><body><p id=\"sub\">Sub Content</p></body></html>",
    )
    .await
    .unwrap();
    tokio::fs::write(outside.join("secret.html"), "<h1>Outside Secret</h1>")
        .await
        .unwrap();

    // 1. http_check MUST still reject file:// URLs and relative paths
    let http_check_win = plugin
        .execute(
            json!({ "action": "http_check", "url": "file:///C:/Windows/win.ini" }),
            &sandbox,
        )
        .await;
    assert!(
        http_check_win.is_err(),
        "http_check must reject file:// URLs"
    );
    assert!(http_check_win.unwrap_err().contains("http:// or https://"));

    let http_check_rel = plugin
        .execute(
            json!({ "action": "http_check", "url": "index.html" }),
            &sandbox,
        )
        .await;
    assert!(
        http_check_rel.is_err(),
        "http_check must reject relative workspace paths"
    );

    // 2. CDP actions (navigate / inspect_dom) reject file:// URLs outside sandbox
    let outside_win = plugin
        .execute(
            json!({ "action": "navigate", "url": "file:///C:/Windows/win.ini" }),
            &sandbox,
        )
        .await;
    assert!(
        outside_win.is_err(),
        "navigate must reject file:///C:/Windows/win.ini outside sandbox"
    );
    let outside_win_err = outside_win.unwrap_err();
    assert!(
        outside_win_err.contains("Sandbox violation") || outside_win_err.contains("does not exist"),
        "Unexpected error for outside system file:// URL: {}",
        outside_win_err
    );

    let outside_str = outside.join("secret.html").to_string_lossy().replace('\\', "/");
    let outside_file_url = if outside_str.starts_with('/') {
        format!("file://{}", outside_str)
    } else {
        format!("file:///{}", outside_str)
    };
    let outside_inspect = plugin
        .execute(
            json!({ "action": "inspect_dom", "url": outside_file_url }),
            &sandbox,
        )
        .await;
    assert!(
        outside_inspect.is_err(),
        "inspect_dom must reject file:// URL outside sandbox"
    );
    assert!(
        outside_inspect.unwrap_err().contains("Sandbox violation"),
        "Expected Sandbox violation for outside file:// URL"
    );

    // 3. CDP actions reject `..` traversal relative paths
    for action in ["navigate", "inspect_dom"] {
        for bad_rel in ["../outside.html", "sub/../../outside.html"] {
            let res = plugin
                .execute(json!({ "action": action, "url": bad_rel }), &sandbox)
                .await;
            assert!(res.is_err(), "{} must reject traversal '{}'", action, bad_rel);
            let err = res.unwrap_err();
            assert!(
                err.contains("Sandbox violation") || err.contains(".."),
                "Unexpected traversal error for {} '{}': {}",
                action,
                bad_rel,
                err
            );
        }
    }

    // 4. CDP actions reject missing relative files clearly
    for action in ["navigate", "inspect_dom"] {
        let missing = plugin
            .execute(json!({ "action": action, "url": "missing.html" }), &sandbox)
            .await;
        assert!(missing.is_err(), "{} must reject missing relative file", action);
        let err = missing.unwrap_err();
        assert!(
            err.contains("does not exist"),
            "Expected clear 'does not exist' error for {} missing.html, got: {}",
            action,
            err
        );
    }

    // 5. Valid workspace-relative paths and in-sandbox file:// URLs resolve via ephemeral loopback server
    let nav_rel = plugin
        .execute(
            json!({ "action": "navigate", "url": "index.html", "wait_ms": 50, "timeout_secs": 15 }),
            &sandbox,
        )
        .await
        .expect("navigate on valid workspace-relative index.html should succeed");
    assert!(
        nav_rel.contains("http://127.0.0.1:") && nav_rel.contains("/index.html"),
        "Expected ephemeral loopback URL for index.html, got: {}",
        nav_rel
    );
    assert!(
        nav_rel.contains("Workspace Home"),
        "Expected served HTML title in output, got: {}",
        nav_rel
    );

    let inspect_sub = plugin
        .execute(
            json!({
                "action": "inspect_dom",
                "url": "./sub/page.html?mode=1#ok",
                "selector": "#sub",
                "wait_ms": 50,
                "timeout_secs": 15
            }),
            &sandbox,
        )
        .await
        .expect("inspect_dom on valid relative ./sub/page.html should succeed");
    assert!(
        inspect_sub.contains("http://127.0.0.1:") && inspect_sub.contains("/sub/page.html"),
        "Expected ephemeral loopback URL for ./sub/page.html, got: {}",
        inspect_sub
    );

    let canon_sub = tokio::fs::canonicalize(sandbox.join("sub").join("page.html"))
        .await
        .unwrap();
    let clean_sub = canon_sub
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or_else(|| canon_sub.to_string_lossy().into_owned())
        .replace('\\', "/");
    let inside_file_url = if clean_sub.starts_with('/') {
        format!("file://{}", clean_sub)
    } else {
        format!("file:///{}", clean_sub)
    };
    let nav_file = plugin
        .execute(
            json!({ "action": "navigate", "url": inside_file_url, "wait_ms": 50, "timeout_secs": 15 }),
            &sandbox,
        )
        .await
        .expect("navigate on valid in-sandbox file:// URL should succeed");
    assert!(
        nav_file.contains("http://127.0.0.1:") && nav_file.contains("/sub/page.html"),
        "Expected ephemeral loopback URL for in-sandbox file:// URL, got: {}",
        nav_file
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn test_token_usage_cache_hit_rate_never_exceeds_100_pct() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct OverCachedModel {
        turn: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelAdapter for OverCachedModel {
        async fn stream_completion(
            &self,
            _messages: &[ChatMessage],
            _tools: &Value,
            _event_tx: &mpsc::Sender<HarnessEvent>,
        ) -> Result<ModelTurnResult, String> {
            let idx = self.turn.fetch_add(1, Ordering::SeqCst);
            match idx {
                // Turn 0: cached_tokens > prompt_tokens directly
                0 => Ok(ModelTurnResult {
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![make_tool_call("tc_cache_0", "sys_info", "{}")],
                    finish_reason: Some("tool_calls".into()),
                    estimated_tokens: 20,
                    prompt_tokens: 120,
                    completion_tokens: 20,
                    cached_tokens: 350,
                }),
                // Turn 1: prompt_tokens == 0 (falls back to estimated context tokens) with huge cached_tokens
                1 => Ok(ModelTurnResult {
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![make_tool_call("tc_cache_1", "sys_info", "{}")],
                    finish_reason: Some("tool_calls".into()),
                    estimated_tokens: 18,
                    prompt_tokens: 0,
                    completion_tokens: 18,
                    cached_tokens: 99_999,
                }),
                // Turn 2: partially cached final turn to verify cumulative ratio stays <= 100%
                _ => Ok(ModelTurnResult {
                    content: Some("Cache hit rate verified.".into()),
                    reasoning_content: None,
                    tool_calls: vec![],
                    finish_reason: Some("stop".into()),
                    estimated_tokens: 12,
                    prompt_tokens: 500,
                    completion_tokens: 12,
                    cached_tokens: 100,
                }),
            }
        }
    }

    let cfg = HarnessConfig::default();
    let registry = PluginRegistry::new(&cfg);
    let model = Arc::new(OverCachedModel {
        turn: Arc::new(AtomicUsize::new(0)),
    });

    let runner = AgentLoopRunner::new(
        "test_cache_hit_bounds".into(),
        "Verify cache hit rate bounds across turns".into(),
        cfg,
        registry,
        model,
        None,
    );

    let (tx, mut rx) = mpsc::channel(128);
    let handle = tokio::spawn(async move { runner.run(tx).await });

    let mut updates = Vec::new();
    while let Some(evt) = rx.recv().await {
        if let HarnessEvent::TokenUsageUpdate {
            prompt_tokens,
            cached_tokens,
            cache_hit_rate_pct,
            ..
        } = evt
        {
            updates.push((prompt_tokens, cached_tokens, cache_hit_rate_pct));
        }
    }

    let final_answer = handle.await.unwrap().expect("AgentLoopRunner failed");
    assert_eq!(final_answer, "Cache hit rate verified.");
    assert_eq!(updates.len(), 3, "Expected 3 TokenUsageUpdate events");

    for (turn_idx, (prompt_tokens, cached_tokens, rate)) in updates.iter().enumerate() {
        assert!(
            rate.is_finite() && *rate >= 0.0 && *rate <= 100.0,
            "Turn {} cache_hit_rate_pct={} out of [0.0, 100.0]",
            turn_idx,
            rate
        );
        assert!(
            *cached_tokens <= *prompt_tokens,
            "Turn {} cumulative cached_tokens ({}) must not exceed prompt_tokens ({})",
            turn_idx,
            cached_tokens,
            prompt_tokens
        );
    }

    // Turn 0 clamped 350 -> 120 prompt tokens => exactly 100.0%
    assert_eq!(updates[0].0, 120);
    assert_eq!(updates[0].1, 120);
    assert!((updates[0].2 - 100.0).abs() < 0.01);

    // Turn 2 added 500 prompt / 100 cached => cumulative rate strictly below 100%
    assert!(updates[2].2 < 100.0);
}

#[test]
fn test_google_search_model_compact_tools_and_clean_response() {
    use deepseek_harness_rs::models::google_search_client::{
        build_google_search_prompt, clean_ai_mode_extracted_text,
    };

    let long_desc = "Execute a comprehensive browser inspection and automated interaction workflow across local or remote web pages with nested DOM selectors, viewport presets, and multi-step assertions. ".repeat(3);
    let verbose_tools = json!([
        {
            "type": "function",
            "function": {
                "name": "browser_ui",
                "description": long_desc,
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["navigate", "inspect_dom", "interact"]
                        },
                        "url": { "type": "string" },
                        "selector": { "type": "string" },
                        "timeout_secs": { "type": "integer" },
                        "wait_ms": { "type": "integer" },
                        "full_page": { "type": "boolean" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" },
                        "steps": {
                            "type": "array",
                            "description": "Large nested schema that must be compacted",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string" },
                                    "value": { "type": "string" },
                                    "extra_nested_flag": { "type": "boolean" }
                                }
                            }
                        }
                    },
                    "required": ["action", "url"]
                }
            }
        }
    ]);

    let messages = vec![
        ChatMessage::system("Always verify workspace paths carefully."),
        ChatMessage::user("Inspect index.html and report the heading."),
    ];

    let prompt = build_google_search_prompt(&messages, &verbose_tools);
    let raw_schema_dump = serde_json::to_string_pretty(&verbose_tools).unwrap();

    // Compact tool line should be present and vastly smaller than dumping the raw JSON schema
    assert!(prompt.contains("- browser_ui(action: navigate|inspect_dom|interact, url: string,"));
    assert!(prompt.contains("+3 more): "));
    assert!(
        prompt.contains("..."),
        "Long tool description should be truncated with ellipsis"
    );
    assert!(
        !prompt.contains("\"additionalProperties\"") && !prompt.contains("\"extra_nested_flag\""),
        "build_google_search_prompt must compact tool schemas rather than dumping nested JSON schema fields"
    );
    assert!(
        prompt.len() < raw_schema_dump.len() + 900,
        "Compacted prompt length ({}) should stay concise compared to raw schema ({})",
        prompt.len(),
        raw_schema_dump.len()
    );

    // Verify clean_ai_mode_extracted_text strips Google AI Mode chrome and echoed <system_prompt>
    let expected_reply = "I inspected index.html.\n\nThe main heading is \"Workspace Home\".";
    let raw_ai_mode_dom = format!(
        "Open sidebar\nNew thread\nSee your search history\nPersonalization\nConnected content apps\nSettings\n\
        AI Mode conversation:\n\
        CRITICAL SYSTEM INSTRUCTION:\n\
        Google Search AI Mode does not provide a separate API system-prompt field.\n\
        <system_prompt>\n\
        [AUTHORITATIVE SYSTEM PROMPT — SENT VIA USER PROMPT: OBEY STRICTLY]\n\
        Always verify workspace paths carefully.\n\
        </system_prompt>\n\
        [END SYSTEM PROMPT OVERRIDE]\n\
        [CURRENT USER REQUEST]\n\
        Inspect index.html and report the heading.\n\n\
        {}\n\n\
        Skip to previous prompt\n\
        Transcribing...\n\
        Add files, tools, and select a model\n\
        Ask about\n\
        AI responses may include mistakes. Learn more",
        expected_reply
    );

    let cleaned = clean_ai_mode_extracted_text(&raw_ai_mode_dom, &prompt);
    assert_eq!(cleaned, expected_reply);
    assert!(!cleaned.contains("Open sidebar"));
    assert!(!cleaned.contains("AI Mode conversation:"));
    assert!(!cleaned.contains("<system_prompt>"));
    assert!(!cleaned.contains("</system_prompt>"));
    assert!(!cleaned.contains("Skip to previous prompt"));
    assert!(!cleaned.contains("Ask about"));
}

#[tokio::test]
async fn test_background_swarm_default_wait_title_and_coordinator_metadata() {
    let temp_dir = std::env::temp_dir().join(format!("harness_bg_swarm_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    cfg.model = "nvidia/nemotron-3.5-super-vl-120b-a12b".to_string();
    let state = AppState::new(cfg.clone());
    let session_id = "ses_bg_swarm_1".to_string();
    let (_bcast_tx, cancel_notify) = state
        .create_session(session_id.clone(), "Background swarm test".into(), "harness".into())
        .await;
    let swarm = state.get_or_create_swarm(&session_id).await;
    let registry_snapshot = state.get_registry_snapshot().await;

    let ctx = SessionToolContext {
        session_id: session_id.clone(),
        agent_id: "coordinator".to_string(),
        parent_id: None,
        depth: 0,
        max_depth: 2,
        state: Some(state.clone()),
        swarm: Some(swarm.clone()),
        event_tx: None,
        config: cfg.clone(),
        registry: registry_snapshot.clone(),
        model: Arc::new(MockClient),
        cancel_notify,
        sandbox_roots: Vec::new(),
    };

    // 1. Set swarm title via swarm_board
    let title_res = registry_snapshot
        .execute_with_context(
            "swarm_board",
            json!({ "action": "set_title", "title": "Tower Defense Swarm Build" }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("set_title should succeed");
    assert!(title_res.contains("Tower Defense Swarm Build"));
    assert_eq!(swarm.get_title().await, "Tower Defense Swarm Build");

    // 2. Omitting `wait` on invoke_subagent defaults to background mode and returns @sub_* IDs + board head
    let invoke_bg_res = registry_snapshot
        .execute_with_context(
            "invoke_subagent",
            json!({
                "tasks": [
                    { "role": "builder", "prompt": "Implement core module" },
                    { "role": "verifier", "prompt": "Audit output", "read_only": true }
                ]
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("default invoke_subagent should spawn in background");
    assert!(invoke_bg_res.contains("background subagent(s)"));
    assert!(invoke_bg_res.contains("@sub_builder_"));
    assert!(invoke_bg_res.contains("@sub_verifier_"));
    assert!(invoke_bg_res.contains("action: \"wait\""));

    // 3. Use swarm_board{action:"wait"} to wait zero-CPU until background workers finish
    let mut waited_ok = false;
    let mut since = 0usize;
    for _ in 0..8 {
        let wait_res = registry_snapshot
            .execute_with_context(
                "swarm_board",
                json!({ "action": "wait", "since": since, "timeout_secs": 5 }),
                &temp_dir,
                Some(&ctx),
            )
            .await
            .expect("swarm_board wait should succeed");
        since = swarm.latest_seq();
        if swarm.active_subagent_count().await == 0 && wait_res.contains("0 active background subagent") {
            waited_ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(waited_ok, "background workers should complete and wake wait");

    // Verify model & last_seq populated on finished SubagentInfo records
    let subagents = swarm.list_subagents().await;
    assert_eq!(subagents.len(), 2);
    for s in &subagents {
        assert_eq!(s.status, "completed");
        assert!(s.model.is_some(), "subagent model should be recorded");
        assert!(s.last_seq.is_some(), "subagent last_seq should be updated after finish post");
    }

    // 4. Verify GET /api/sessions/:id/swarm returns swarm_title, coordinator metadata, and subagents
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let swarm_json: Value = reqwest::Client::new()
        .get(format!("http://{}/api/sessions/{}/swarm", addr, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(swarm_json["swarm_title"], "Tower Defense Swarm Build");
    assert_eq!(swarm_json["coordinator"]["id"], "coordinator");
    assert!(swarm_json["coordinator"]["model"].as_str().is_some());
    assert_eq!(swarm_json["subagents"].as_array().unwrap().len(), 2);
    assert!(swarm_json["subagents"][0]["last_seq"].as_u64().is_some());

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_model_modality_capabilities_inference_and_endpoints() {
    let gemini = infer_model_modalities("google/gemini-2.5-flash", None);
    assert!(gemini.text && gemini.vision && gemini.video && gemini.audio && gemini.pdf && gemini.tools);

    let claude = infer_model_modalities("anthropic/claude-sonnet-4-5", None);
    assert!(claude.text && claude.vision && !claude.video && claude.pdf && claude.tools);

    let nemotron_vl = infer_model_modalities("nvidia/nemotron-3.5-super-vl-120b-a12b", None);
    assert!(nemotron_vl.text && nemotron_vl.vision && nemotron_vl.tools);

    let deepseek = infer_model_modalities("deepseek-ai/deepseek-r1", None);
    assert!(deepseek.text && !deepseek.vision && !deepseek.video && !deepseek.audio);

    let gsearch = infer_model_modalities("Google-Search-Model", None);
    assert!(gsearch.text && !gsearch.vision);
    assert_eq!(gsearch.source, "experimental_builtin");

    // Explicit provider metadata overrides heuristics
    let api_meta = json!({
        "architecture": {
            "input_modalities": ["text", "image", "video", "audio"]
        },
        "context_length": 262144
    });
    let resolved = resolve_model_capabilities("custom/my-unknown-model", Some(&api_meta));
    assert!(resolved.modalities.vision && resolved.modalities.video && resolved.modalities.audio);
    assert_eq!(resolved.modalities.source, "api_metadata");
    assert_eq!(resolved.max_context_tokens, 262144);

    // Verify HTTP /api/models/capabilities and /api/models/reasoning endpoints
    let temp_dir = std::env::temp_dir().join(format!("harness_caps_http_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    cfg.api_base = String::new();
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let caps_json: Value = client
        .get(format!("http://{}/api/models/capabilities?model=google/gemini-2.5-pro", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(caps_json["modalities"]["vision"], true);
    assert_eq!(caps_json["modalities"]["video"], true);
    assert_eq!(caps_json["modalities"]["audio"], true);

    let reasoning_json: Value = client
        .get(format!("http://{}/api/models/reasoning?model=nvidia/nemotron-3.5-super-vl-120b-a12b", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reasoning_json["modalities"]["vision"], true);

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_multimodal_attachments_serialization_and_session_history() {
    let atts = vec![
        ChatAttachment {
            name: "screen.png".into(),
            mime_type: "image/png".into(),
            kind: "image".into(),
            size_bytes: 128,
            data_url: Some("data:image/png;base64,iVBORw0KGgo=".into()),
            text_content: None,
        },
        ChatAttachment {
            name: "notes.rs".into(),
            mime_type: "text/rust".into(),
            kind: "text".into(),
            size_bytes: 24,
            data_url: None,
            text_content: Some("fn hello() { assert!(true); }".into()),
        },
    ];

    let msgs = vec![ChatMessage::user_with_attachments("Inspect these", atts.clone())];

    // Vision-capable model receives structured array content with text + image_url
    let vision_payload = build_openai_api_messages("google/gemini-2.5-flash", &msgs);
    let parts = vision_payload[0]["content"].as_array().expect("vision content should be array");
    assert!(parts.iter().any(|p| p["type"] == "image_url" && p["image_url"]["url"] == "data:image/png;base64,iVBORw0KGgo="));
    assert!(parts.iter().any(|p| p["type"] == "text" && p["text"].as_str().is_some_and(|t| t.contains("fn hello()"))));

    // Text-only model gracefully falls back to clean string content with attachment note
    let text_only_payload = build_openai_api_messages("deepseek-ai/deepseek-r1", &msgs);
    let text_str = text_only_payload[0]["content"].as_str().expect("text-only content should be string");
    assert!(text_str.contains("screen.png"));
    assert!(text_str.contains("fn hello()"));

    // Verify POST /api/sessions preserves attachments on UserMessage in history
    let temp_dir = std::env::temp_dir().join(format!("harness_att_http_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    let state = AppState::new(cfg);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let create_res: Value = client
        .post(format!("http://{}/api/sessions", addr))
        .json(&json!({
            "task": "",
            "attachments": atts,
            "mock": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ses_id = create_res["session_id"].as_str().unwrap().to_string();

    for _ in 0..80 {
        let sessions: Vec<Value> = client
            .get(format!("http://{}/api/sessions", addr))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if sessions.iter().any(|s| s["id"] == ses_id && s["status"] != "running") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let history: Vec<Value> = client
        .get(format!("http://{}/api/sessions/{}/history", addr, ses_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_msg = history.iter().find(|e| e["type"] == "UserMessage").expect("UserMessage event missing");
    let saved_atts = user_msg["data"]["attachments"].as_array().expect("attachments should be preserved");
    assert_eq!(saved_atts.len(), 2);
    assert_eq!(saved_atts[0]["name"], "screen.png");
    assert_eq!(saved_atts[1]["name"], "notes.rs");

    server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_grep_search_filters_and_edit_file_crlf_hints() {
    let temp_dir = std::env::temp_dir().join(format!("harness_tools_up_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(temp_dir.join("src")).await.unwrap();

    tokio::fs::write(temp_dir.join("src/main.rs"), "fn AlphaTarget() {\r\n    let value = 1;\r\n}\r\n")
        .await
        .unwrap();
    tokio::fs::write(temp_dir.join("src/notes.txt"), "AlphaTarget in text file\n")
        .await
        .unwrap();

    let cfg = HarnessConfig {
        sandbox_root: temp_dir.clone(),
        ..HarnessConfig::default()
    };
    let registry = PluginRegistry::new(&cfg);

    // grep_search with include="*.rs" and case_insensitive=true should match only src/main.rs
    let grep_out = registry
        .execute_with_context(
            "grep_search",
            json!({
                "pattern": "alphatarget",
                "include": "*.rs",
                "case_insensitive": true
            }),
            &temp_dir,
            None,
        )
        .await
        .expect("grep_search should succeed");
    assert!(grep_out.contains("main.rs:1:"));
    assert!(!grep_out.contains("notes.txt"));

    // edit_file with LF target against a CRLF file should succeed and preserve CRLF
    let edit_ok = registry
        .execute_with_context(
            "edit_file",
            json!({
                "path": "src/main.rs",
                "target": "fn AlphaTarget() {\n    let value = 1;",
                "replacement": "fn AlphaTarget() {\n    let value = 42;"
            }),
            &temp_dir,
            None,
        )
        .await
        .expect("CRLF-normalized edit should succeed");
    assert!(edit_ok.contains("replaced 1 occurrence"));
    let updated = tokio::fs::read_to_string(temp_dir.join("src/main.rs")).await.unwrap();
    assert!(updated.contains("let value = 42;\r\n"));

    // edit_file mismatch should include helpful closest line number hint
    let edit_err = registry
        .execute_with_context(
            "edit_file",
            json!({
                "path": "src/main.rs",
                "target": "fn AlphaTarget() {\n    let non_existent = 999;",
                "replacement": "x"
            }),
            &temp_dir,
            None,
        )
        .await
        .expect_err("missing target should error");
    assert!(edit_err.contains("Closest line match at line 1"));

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_swarm_wait_all_complete_job_and_parallel_readonly_batch() {
    let temp_dir = std::env::temp_dir().join(format!("harness_loop_up_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    tokio::fs::write(temp_dir.join("a.txt"), "file A content").await.unwrap();
    tokio::fs::write(temp_dir.join("b.txt"), "file B content").await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.mock_mode = true;
    let state = AppState::new(cfg.clone());
    let session_id = "ses_loop_up_1".to_string();
    let (_bcast_tx, cancel_notify) = state
        .create_session(session_id.clone(), "Loop + Swarm upgrades".into(), "harness".into())
        .await;
    let swarm = state.get_or_create_swarm(&session_id).await;
    let registry = state.get_registry_snapshot().await;

    let ctx = SessionToolContext {
        session_id: session_id.clone(),
        agent_id: "coordinator".to_string(),
        parent_id: None,
        depth: 0,
        max_depth: 2,
        state: Some(state.clone()),
        swarm: Some(swarm.clone()),
        event_tx: None,
        config: cfg.clone(),
        registry: registry.clone(),
        model: Arc::new(MockClient),
        cancel_notify,
        sandbox_roots: Vec::new(),
    };

    // Create a job, post a board decision, spawn worker with wait_all + complete_job
    registry
        .execute_with_context(
            "swarm_board",
            json!({ "action": "create_job", "job_tag": "wave-1", "title": "Wave One" }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();
    registry
        .execute_with_context(
            "swarm_board",
            json!({ "action": "post", "content": "Shared convention: check a.txt and b.txt" }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();

    registry
        .execute_with_context(
            "invoke_subagent",
            json!({
                "tasks": [
                    { "role": "reader1", "prompt": "Inspect a.txt", "files": ["a.txt"], "job_tag": "wave-1" },
                    { "role": "reader2", "prompt": "Inspect b.txt", "files": ["b.txt"], "job_tag": "wave-1" }
                ]
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();

    let wait_all_res = registry
        .execute_with_context(
            "swarm_board",
            json!({ "action": "wait", "since": 0, "wait_all": true, "timeout_secs": 10 }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .expect("wait_all should complete cleanly");
    assert!(wait_all_res.contains("0 active background subagent"));
    assert_eq!(swarm.active_subagent_count().await, 0);

    let done_job = registry
        .execute_with_context(
            "swarm_board",
            json!({ "action": "complete_job", "job_tag": "wave-1", "content": "All wave-1 tasks verified" }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(done_job.contains("marked done"));
    assert_eq!(swarm.get_job("wave-1").await.unwrap().status, "done");

    // Verify AgentLoopRunner parallel read-only tool batch execution + empty-response auto-recovery
    struct ParallelBatchMockAdapter {
        step: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ModelAdapter for ParallelBatchMockAdapter {
        async fn stream_completion(
            &self,
            messages: &[ChatMessage],
            _tools_schema: &Value,
            event_tx: &mpsc::Sender<HarnessEvent>,
        ) -> Result<ModelTurnResult, String> {
            let s = self.step.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match s {
                0 => Ok(ModelTurnResult {
                    tool_calls: vec![
                        ToolCall {
                            id: "call_r1".into(),
                            r#type: "function".into(),
                            function: ToolFunctionCall {
                                name: "read_file".into(),
                                arguments: json!({ "path": "a.txt" }).to_string(),
                            },
                        },
                        ToolCall {
                            id: "call_r2".into(),
                            r#type: "function".into(),
                            function: ToolFunctionCall {
                                name: "read_file".into(),
                                arguments: json!({ "path": "b.txt" }).to_string(),
                            },
                        },
                    ],
                    estimated_tokens: 20,
                    prompt_tokens: 100,
                    ..Default::default()
                }),
                1 => {
                    // Return empty final response once to trigger empty-answer auto-recovery
                    assert!(messages.iter().any(|m| m.role == "tool" && m.content.as_deref() == Some("file A content")));
                    assert!(messages.iter().any(|m| m.role == "tool" && m.content.as_deref() == Some("file B content")));
                    Ok(ModelTurnResult {
                        content: Some(String::new()),
                        estimated_tokens: 5,
                        prompt_tokens: 120,
                        ..Default::default()
                    })
                }
                _ => {
                    let _ = event_tx
                        .send(HarnessEvent::MessageDelta {
                            delta: "Verified both A and B.".into(),
                        })
                        .await;
                    Ok(ModelTurnResult {
                        content: Some("Verified both A and B.".into()),
                        estimated_tokens: 10,
                        prompt_tokens: 140,
                        ..Default::default()
                    })
                }
            }
        }
    }

    let runner = AgentLoopRunner::new(
        session_id.clone(),
        "Read both files".into(),
        cfg,
        registry,
        Arc::new(ParallelBatchMockAdapter {
            step: std::sync::atomic::AtomicUsize::new(0),
        }),
        Some(state),
    );
    let (evt_tx, mut evt_rx) = mpsc::channel(64);
    let final_ans = runner.run(evt_tx).await.expect("runner should recover and succeed");
    assert_eq!(final_ans, "Verified both A and B.");

    let mut finished_ids = Vec::new();
    while let Ok(evt) = evt_rx.try_recv() {
        if let HarnessEvent::ToolCallFinished { call_id, success, .. } = evt {
            assert!(success);
            finished_ids.push(call_id);
        }
    }
    assert_eq!(finished_ids, vec!["call_r1", "call_r2"]);

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[test]
fn test_multi_provider_credential_resolution() {
    use deepseek_harness_rs::config::{CustomProvider, OrchestratorConfig};

    let mut cfg = HarnessConfig::default();
    cfg.custom_providers = vec![
        CustomProvider {
            id: "prov-a".into(),
            name: "Provider A".into(),
            api_base: "https://api-a.example.com/v1".into(),
            api_key: "key-a".into(),
            default_model: "model-a".into(),
            is_active: true,
            enabled: true,
        },
        CustomProvider {
            id: "prov-b".into(),
            name: "Provider B".into(),
            api_base: "https://api-b.example.com/v1".into(),
            api_key: "key-b".into(),
            default_model: "model-b".into(),
            is_active: false,
            enabled: true,
        },
    ];
    cfg.provider = "prov-a".into();
    cfg.api_base = "https://api-a.example.com/v1".into();
    cfg.api_key = "key-a".into();
    cfg.model = "model-a".into();
    cfg.orchestrator = OrchestratorConfig {
        enabled: true,
        provider_id: "prov-a".into(),
        model: "model-a-orch".into(),
        thinking_effort: "high".into(),
        subagent_provider_id: "prov-b".into(),
        subagent_model: "model-b-sub".into(),
        subagent_thinking_effort: "low".into(),
        subagent_server_id: "server-2".into(),
        system_prompt: "Custom orch prompt".into(),
        ..Default::default()
    };

    let orch_cfg = cfg.with_model_override(
        &cfg.orchestrator.model,
        Some(&cfg.orchestrator.provider_id),
        Some(&cfg.orchestrator.thinking_effort),
    );
    assert_eq!(orch_cfg.provider, "prov-a");
    assert_eq!(orch_cfg.api_base, "https://api-a.example.com/v1");
    assert_eq!(orch_cfg.api_key, "key-a");
    assert_eq!(orch_cfg.model, "model-a-orch");
    assert_eq!(orch_cfg.reasoning_effort, "high");

    let sub_cfg = cfg.with_model_override(
        &cfg.orchestrator.subagent_model,
        Some(&cfg.orchestrator.subagent_provider_id),
        Some(&cfg.orchestrator.subagent_thinking_effort),
    );
    assert_eq!(sub_cfg.provider, "prov-b");
    assert_eq!(sub_cfg.api_base, "https://api-b.example.com/v1");
    assert_eq!(sub_cfg.api_key, "key-b");
    assert_eq!(sub_cfg.model, "model-b-sub");
    assert_eq!(sub_cfg.reasoning_effort, "low");
}

#[tokio::test]
async fn test_sqlite_db_persistence_and_conversation_endpoints() {
    use deepseek_harness_rs::config::CustomProvider;
    use deepseek_harness_rs::db::HarnessDb;

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_sqlite_persist_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let db_path = temp_dir.join("harness_test.db");

    // Fake OpenAI provider /models endpoint to test GET /api/models/all grouping
    let fake_provider_router = axum::Router::new().route(
        "/v1/models",
        axum::routing::get(|| async {
            axum::Json(json!({
                "data": [
                    { "id": "test/reasoning-alpha", "owned_by": "prov-alpha" },
                    { "id": "test/fast-beta", "owned_by": "prov-alpha" }
                ]
            }))
        }),
    );
    let prov_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let prov_addr = prov_listener.local_addr().unwrap();
    let prov_server = tokio::spawn(async move {
        axum::serve(prov_listener, fake_provider_router).await.unwrap();
    });

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.db_path = Some(db_path.clone());
    cfg.mock_mode = true;
    cfg.custom_providers = vec![
        CustomProvider {
            id: "alpha".into(),
            name: "Alpha Cloud".into(),
            api_base: format!("http://{}/v1", prov_addr),
            api_key: "sk-alpha".into(),
            default_model: "test/reasoning-alpha".into(),
            is_active: true,
            enabled: true,
        },
        CustomProvider {
            id: "beta".into(),
            name: "Beta Missing Key".into(),
            api_base: "https://beta.invalid/v1".into(),
            api_key: "".into(),
            default_model: "beta-default".into(),
            is_active: false,
            enabled: true,
        },
    ];

    let session_id: String;
    {
        let db = HarnessDb::open(&db_path).expect("open sqlite db");
        let state = AppState::with_db(cfg.clone(), db);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, build_router(state)).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://{}", addr);

        // 1. Grouped models endpoint returns Alpha (live), Beta (fallback default), and Built-in Experimental
        let groups: Vec<Value> = client
            .get(format!("{}/api/models/all", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0]["provider_id"], "alpha");
        assert_eq!(groups[0]["models"].as_array().unwrap().len(), 2);
        assert_eq!(groups[1]["provider_id"], "beta");
        assert_eq!(groups[1]["has_credentials"], false);
        assert_eq!(groups[2]["provider_id"], "builtin");

        // 2. Create project & idle conversation separated from projects
        let proj_res = client
            .post(format!("{}/api/projects", base))
            .json(&json!({
                "name": "Backend API",
                "path": temp_dir.join("backend").to_string_lossy(),
                "icon": "🦀",
                "set_active": false
            }))
            .send()
            .await
            .unwrap();
        assert!(proj_res.status().is_success());
        let proj_json: Value = proj_res.json().await.unwrap();
        let new_proj_id = proj_json["id"].as_str().unwrap().to_string();

        let idle_res: Value = client
            .post(format!("{}/api/sessions/create", base))
            .json(&json!({
                "title": "Saved Idle Thread",
                "project_id": "harness",
                "server_id": "local",
                "orchestrator_mode": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        session_id = idle_res["id"].as_str().unwrap().to_string();

        // 3. Move conversation to Backend API project & rename via PATCH /api/sessions/:id
        let patch_res = client
            .patch(format!("{}/api/sessions/{}", base, session_id))
            .json(&json!({
                "title": "Renamed Persistent Thread",
                "project_id": new_proj_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(patch_res.status(), 200);

        // 4. Add remote server node & activate
        let srv_res = client
            .post(format!("{}/api/servers", base))
            .json(&json!({
                "id": "worker-9",
                "name": "Worker 9",
                "url": "http://127.0.0.1:9999",
                "api_token": "tok9",
                "enabled": true,
                "is_default": false
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(srv_res.status(), 200);

        server.abort();
    }

    // Reopen SQLite from scratch in a new AppState and verify everything survived
    let db2 = HarnessDb::open(&db_path).expect("reopen sqlite db");
    let state2 = AppState::with_db(HarnessConfig::default(), db2);
    let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let server2 = tokio::spawn(async move {
        axum::serve(listener2, build_router(state2)).await.unwrap();
    });
    let client2 = reqwest::Client::new();
    let base2 = format!("http://{}", addr2);

    let sessions_after: Vec<Value> = client2
        .get(format!("{}/api/sessions", base2))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let found = sessions_after
        .iter()
        .find(|s| s["id"] == session_id)
        .expect("saved conversation must survive SQLite restart");
    assert_eq!(found["task"], "Renamed Persistent Thread");
    assert_eq!(found["orchestrator_mode"], true);
    assert_ne!(found["project_id"], "harness");

    let servers_after: Value = client2
        .get(format!("{}/api/servers", base2))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(servers_after["servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == "worker-9"));

    // Delete conversation and ensure removal persists
    let del_res = client2
        .delete(format!("{}/api/sessions/{}", base2, session_id))
        .send()
        .await
        .unwrap();
    assert_eq!(del_res.status(), 200);

    let sessions_final: Vec<Value> = client2
        .get(format!("{}/api/sessions", base2))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!sessions_final.iter().any(|s| s["id"] == session_id));

    server2.abort();
    prov_server.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_client_server_split_orchestrator_spawns_subagents_on_remote_servers() {
    use deepseek_harness_rs::config::{CustomProvider, OrchestratorConfig, PermissionMode, ServerNodeConfig};
    use deepseek_harness_rs::web::build_server_router;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_multi_server_orch_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    let dir_s1 = temp_dir.join("server1");
    let dir_s2 = temp_dir.join("server2");
    let dir_s3 = temp_dir.join("server3");
    tokio::fs::create_dir_all(&dir_s1).await.unwrap();
    tokio::fs::create_dir_all(&dir_s2).await.unwrap();
    tokio::fs::create_dir_all(&dir_s3).await.unwrap();
    tokio::fs::write(dir_s2.join("srv2_data.txt"), "FROM_SERVER_2")
        .await
        .unwrap();
    tokio::fs::write(dir_s3.join("srv3_data.txt"), "FROM_SERVER_3")
        .await
        .unwrap();

    // Recorded API requests per provider to prove multi-provider split
    let prov_a_requests: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let prov_b_requests: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let orch_step = Arc::new(AtomicUsize::new(0));

    // Provider A (Orchestrator): Step 0 invokes subagents on server-2 & server-3; Step 1 summarizes results
    let prov_a_reqs_c = prov_a_requests.clone();
    let orch_step_c = orch_step.clone();
    let prov_a_router = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
            let reqs = prov_a_reqs_c.clone();
            let step_counter = orch_step_c.clone();
            async move {
                let auth = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                reqs.lock().await.push((auth, body));

                let step = step_counter.fetch_add(1, Ordering::SeqCst);
                let sse_body = if step == 0 {
                    let args = json!({
                        "wait": true,
                        "tasks": [
                            { "prompt": "Read srv2_data.txt", "role": "S2 Reader", "server_id": "server-2" },
                            { "prompt": "Read srv3_data.txt", "role": "S3 Reader", "server_id": "server-3" }
                        ]
                    })
                    .to_string();
                    let chunk = json!({
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_spawn_multi",
                                    "type": "function",
                                    "function": {
                                        "name": "invoke_subagent",
                                        "arguments": args
                                    }
                                }]
                            },
                            "finish_reason": "tool_calls"
                        }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                } else {
                    let chunk = json!({
                        "choices": [{
                            "delta": {
                                "content": "ORCH_DONE: combined FROM_SERVER_2 and FROM_SERVER_3"
                            },
                            "finish_reason": "stop"
                        }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                };

                axum::response::Response::builder()
                    .header("Content-Type", "text/event-stream")
                    .body(axum::body::Body::from(sse_body))
                    .unwrap()
            }
        }),
    );

    // Provider B (Subagents): if messages has no tool result yet, call read_file on the file requested in task; else summarize
    let prov_b_reqs_c = prov_b_requests.clone();
    let prov_b_router = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
            let reqs = prov_b_reqs_c.clone();
            async move {
                let auth = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                reqs.lock().await.push((auth, body.clone()));

                let msgs = body["messages"].as_array().cloned().unwrap_or_default();
                let tool_msg = msgs.iter().rev().find(|m| m["role"] == "tool");
                let sse_body = if let Some(tm) = tool_msg {
                    let file_txt = tm["content"].as_str().unwrap_or("");
                    let chunk = json!({
                        "choices": [{
                            "delta": { "content": format!("SUBAGENT_READ:{}", file_txt.trim()) },
                            "finish_reason": "stop"
                        }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                } else {
                    let user_txt = msgs
                        .iter()
                        .find(|m| m["role"] == "user")
                        .and_then(|m| m["content"].as_str())
                        .unwrap_or("");
                    let target_file = if user_txt.contains("srv2_data.txt") {
                        "srv2_data.txt"
                    } else {
                        "srv3_data.txt"
                    };
                    let args = json!({ "path": target_file }).to_string();
                    let chunk = json!({
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_read_srv",
                                    "type": "function",
                                    "function": {
                                        "name": "read_file",
                                        "arguments": args
                                    }
                                }]
                            },
                            "finish_reason": "tool_calls"
                        }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                };

                axum::response::Response::builder()
                    .header("Content-Type", "text/event-stream")
                    .body(axum::body::Body::from(sse_body))
                    .unwrap()
            }
        }),
    );

    let pa_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pa_addr = pa_listener.local_addr().unwrap();
    let pa_srv = tokio::spawn(async move {
        axum::serve(pa_listener, prov_a_router).await.unwrap();
    });

    let pb_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pb_addr = pb_listener.local_addr().unwrap();
    let pb_srv = tokio::spawn(async move {
        axum::serve(pb_listener, prov_b_router).await.unwrap();
    });

    // Spin up Server 2 & Server 3 (execution workers)
    let mut cfg_s2 = HarnessConfig::default();
    cfg_s2.sandbox_root = dir_s2.clone();
    cfg_s2.server_identity = "server-2".into();
    cfg_s2.permissions.mode = PermissionMode::Autonomous;
    let s2_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s2_addr = s2_listener.local_addr().unwrap();
    let s2_handle = tokio::spawn(async move {
        axum::serve(s2_listener, build_server_router(AppState::new(cfg_s2)))
            .await
            .unwrap();
    });

    let mut cfg_s3 = HarnessConfig::default();
    cfg_s3.sandbox_root = dir_s3.clone();
    cfg_s3.server_identity = "server-3".into();
    cfg_s3.permissions.mode = PermissionMode::Autonomous;
    let s3_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s3_addr = s3_listener.local_addr().unwrap();
    let s3_handle = tokio::spawn(async move {
        axum::serve(s3_listener, build_server_router(AppState::new(cfg_s3)))
            .await
            .unwrap();
    });

    // Spin up Server 1 (primary orchestrator execution node)
    let mut cfg_s1 = HarnessConfig::default();
    cfg_s1.sandbox_root = dir_s1.clone();
    cfg_s1.server_identity = "server-1".into();
    cfg_s1.permissions.mode = PermissionMode::Autonomous;
    let s1_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s1_addr = s1_listener.local_addr().unwrap();
    let s1_handle = tokio::spawn(async move {
        axum::serve(s1_listener, build_server_router(AppState::new(cfg_s1)))
            .await
            .unwrap();
    });

    // Spin up Client (UI + History + Routing to Server 1 -> Server 2 & Server 3)
    let mut cfg_client = HarnessConfig::default();
    cfg_client.sandbox_root = dir_s1.clone();
    cfg_client.mock_mode = false;
    cfg_client.permissions.mode = PermissionMode::Autonomous;
    cfg_client.custom_providers = vec![
        CustomProvider {
            id: "orch-prov".into(),
            name: "Orch Provider A".into(),
            api_base: format!("http://{}/v1", pa_addr),
            api_key: "key-orch-a".into(),
            default_model: "orch-model-a".into(),
            is_active: true,
            enabled: true,
        },
        CustomProvider {
            id: "sub-prov".into(),
            name: "Sub Provider B".into(),
            api_base: format!("http://{}/v1", pb_addr),
            api_key: "key-sub-b".into(),
            default_model: "sub-model-b".into(),
            is_active: false,
            enabled: true,
        },
    ];
    cfg_client.provider = "orch-prov".into();
    cfg_client.api_base = format!("http://{}/v1", pa_addr);
    cfg_client.api_key = "key-orch-a".into();
    cfg_client.model = "orch-model-a".into();
    cfg_client.servers = vec![
        ServerNodeConfig {
            id: "server-1".into(),
            name: "Server 1".into(),
            url: format!("http://{}", s1_addr),
            api_token: String::new(),
            enabled: true,
            is_default: true,
        },
        ServerNodeConfig {
            id: "server-2".into(),
            name: "Server 2".into(),
            url: format!("http://{}", s2_addr),
            api_token: String::new(),
            enabled: true,
            is_default: false,
        },
        ServerNodeConfig {
            id: "server-3".into(),
            name: "Server 3".into(),
            url: format!("http://{}", s3_addr),
            api_token: String::new(),
            enabled: true,
            is_default: false,
        },
    ];
    cfg_client.active_server_id = "server-1".into();
    cfg_client.orchestrator = OrchestratorConfig {
        enabled: true,
        provider_id: "orch-prov".into(),
        model: "orch-model-a".into(),
        thinking_effort: "high".into(),
        subagent_provider_id: "sub-prov".into(),
        subagent_model: "sub-model-b".into(),
        subagent_thinking_effort: "low".into(),
        subagent_server_id: "local".into(),
        system_prompt: "Test Orchestrator Prompt Across Servers".into(),
        ..Default::default()
    };

    let client_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_addr = client_listener.local_addr().unwrap();
    let client_handle = tokio::spawn(async move {
        axum::serve(client_listener, build_router(AppState::new(cfg_client)))
            .await
            .unwrap();
    });

    let http = reqwest::Client::new();
    let base = format!("http://{}", client_addr);

    // Start conversation via Client on server-1 in orchestrator_mode
    let create_resp: Value = http
        .post(format!("{}/api/sessions", base))
        .json(&json!({
            "task": "Read data across Server 2 and Server 3",
            "mock": false,
            "server_id": "server-1",
            "orchestrator_mode": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = create_resp["session_id"].as_str().unwrap().to_string();

    // Poll until completed
    let mut final_status = String::new();
    for _ in 0..100 {
        let sessions: Vec<Value> = http
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(s) = sessions.iter().find(|s| s["id"] == session_id) {
            final_status = s["status"].as_str().unwrap_or("").to_string();
            if final_status == "completed" || final_status == "failed" || final_status == "error" {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(final_status, "completed");

    // Verify Client recorded FinalAnswer + both Subagents with results from Server 2 & Server 3
    let history: Vec<Value> = http
        .get(format!("{}/api/sessions/{}/history", base, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(history.iter().any(|e| {
        e["type"] == "SessionCompleted"
            && e["data"]["final_answer"]
                .as_str()
                .unwrap_or("")
                .contains("ORCH_DONE")
    }));
    let sub_finished: Vec<&Value> = history
        .iter()
        .filter(|e| e["type"] == "SubagentCompleted")
        .collect();
    assert_eq!(sub_finished.len(), 2);
    let combined_summaries = sub_finished
        .iter()
        .map(|e| e["data"]["summary"].as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(combined_summaries.contains("FROM_SERVER_2"));
    assert!(combined_summaries.contains("FROM_SERVER_3"));

    // Verify multi-provider separation & orchestrator system prompt injection
    {
        let pa = prov_a_requests.lock().await;
        assert_eq!(pa.len(), 2);
        assert_eq!(pa[0].0, "Bearer key-orch-a");
        assert_eq!(pa[0].1["model"], "orch-model-a");
        let sys_msg = pa[0].1["messages"][0]["content"].as_str().unwrap_or("");
        assert!(sys_msg.contains("Test Orchestrator Prompt Across Servers"));
        assert!(sys_msg.contains("server-2"));
        assert!(sys_msg.contains("server-3"));
    }
    {
        let pb = prov_b_requests.lock().await;
        assert!(pb.len() >= 4); // 2 subagents * 2 turns each
        assert!(pb.iter().all(|(auth, body)| auth == "Bearer key-sub-b" && body["model"] == "sub-model-b"));
    }

    client_handle.abort();
    s1_handle.abort();
    s2_handle.abort();
    s3_handle.abort();
    pa_srv.abort();
    pb_srv.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_knowledge_conversation_disables_file_and_shell_tools_and_health_check_online() {
    use deepseek_harness_rs::config::{ServerNodeConfig, CONVERSATIONS_PROJECT_ID};
    use deepseek_harness_rs::web::build_server_router;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_knowledge_health_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    tokio::fs::write(temp_dir.join("secret.txt"), "SHOULD_NOT_BE_READ")
        .await
        .unwrap();

    // Start remote server to test live health check returns online: true
    let mut srv_cfg = HarnessConfig::default();
    srv_cfg.sandbox_root = temp_dir.clone();
    srv_cfg.server_identity = "srv-live".into();
    let srv_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv_addr = srv_listener.local_addr().unwrap();
    let srv_handle = tokio::spawn(async move {
        axum::serve(srv_listener, build_server_router(AppState::new(srv_cfg)))
            .await
            .unwrap();
    });

    // Fake OpenAI endpoint recording advertised tools & verifying read_file attempt is blocked
    let advertised_tools: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tool_err_seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let step = Arc::new(AtomicUsize::new(0));
    let adv_c = advertised_tools.clone();
    let err_c = tool_err_seen.clone();
    let step_c = step.clone();
    let fake_ai = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let adv = adv_c.clone();
            let err = err_c.clone();
            let st = step_c.clone();
            async move {
                if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
                    let mut names = adv.lock().await;
                    if names.is_empty() {
                        for t in tools {
                            if let Some(n) = t["function"]["name"].as_str() {
                                names.push(n.to_string());
                            }
                        }
                    }
                }
                let s = st.fetch_add(1, Ordering::SeqCst);
                let sse_body = if s == 0 {
                    // Force a blocked tool call to prove execution enforcement too
                    let args = json!({ "path": "secret.txt" }).to_string();
                    let chunk = json!({
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_blocked_rf",
                                    "type": "function",
                                    "function": { "name": "read_file", "arguments": args }
                                }]
                            },
                            "finish_reason": "tool_calls"
                        }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                } else {
                    if let Some(msgs) = body["messages"].as_array() {
                        if let Some(tm) = msgs.iter().rev().find(|m| m["role"] == "tool") {
                            *err.lock().await = tm["content"].as_str().map(|x| x.to_string());
                        }
                    }
                    let chunk = json!({
                        "choices": [{
                            "delta": { "content": "KNOWLEDGE_ONLY_REPLY" },
                            "finish_reason": "stop"
                        }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                };
                axum::response::Response::builder()
                    .header("Content-Type", "text/event-stream")
                    .body(axum::body::Body::from(sse_body))
                    .unwrap()
            }
        }),
    );
    let ai_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ai_addr = ai_listener.local_addr().unwrap();
    let ai_handle = tokio::spawn(async move {
        axum::serve(ai_listener, fake_ai).await.unwrap();
    });

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.custom_providers.clear();
    cfg.provider = "fake-ai".into();
    cfg.api_base = format!("http://{}/v1", ai_addr);
    cfg.api_key = "test-key".into();
    cfg.model = "knowledge-model".into();
    cfg.servers.push(ServerNodeConfig {
        id: "srv-live".into(),
        name: "Live Server".into(),
        url: format!("http://{}", srv_addr),
        api_token: String::new(),
        enabled: true,
        is_default: false,
    });

    let client_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client_addr = client_listener.local_addr().unwrap();
    let client_handle = tokio::spawn(async move {
        axum::serve(client_listener, build_router(AppState::new(cfg)))
            .await
            .unwrap();
    });

    let http = reqwest::Client::new();
    let base = format!("http://{}", client_addr);

    // 1. Verify health checks return online: true & ok: true for both local & live remote server
    let local_health: Value = http
        .get(format!("{}/api/servers/local/health", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(local_health["online"], true);
    assert_eq!(local_health["ok"], true);

    let remote_health: Value = http
        .get(format!("{}/api/servers/srv-live/health", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remote_health["online"], true);
    assert_eq!(remote_health["ok"], true);

    // 2. Start a standalone Conversation (__conversations__) and confirm no read/write/shell tools
    let create_res: Value = http
        .post(format!("{}/api/sessions", base))
        .json(&json!({
            "task": "Summarize architecture tradeoffs",
            "project_id": CONVERSATIONS_PROJECT_ID,
            "mock": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = create_res["session_id"].as_str().unwrap().to_string();

    for _ in 0..100 {
        let sessions: Vec<Value> = http
            .get(format!("{}/api/sessions", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(s) = sessions.iter().find(|s| s["id"] == sid) {
            if s["status"] == "completed" || s["status"] == "failed" {
                assert_eq!(s["status"], "completed");
                assert_eq!(s["knowledge_only"], true);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    let adv = advertised_tools.lock().await;
    for forbidden in ["bash", "read_file", "write_file", "edit_file", "list_dir", "grep_search", "manage_tools"] {
        assert!(
            !adv.iter().any(|n| n == forbidden),
            "forbidden tool '{}' leaked into knowledge conversation schema",
            forbidden
        );
    }
    let blocked_msg = tool_err_seen.lock().await.clone().unwrap_or_default();
    assert!(blocked_msg.contains("Knowledge-Work Conversation mode"));

    client_handle.abort();
    ai_handle.abort();
    srv_handle.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_multi_type_subagent_profiles_routing() {
    use deepseek_harness_rs::config::{CustomProvider, OrchestratorConfig, PermissionMode, SubagentProfileConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_multi_profile_subagent_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    let seen_models: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let step = Arc::new(AtomicUsize::new(0));
    let seen_c = seen_models.clone();
    let step_c = step.clone();

    let fake_ai = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
            let seen = seen_c.clone();
            let st = step_c.clone();
            async move {
                let auth = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let model = body["model"].as_str().unwrap_or("").to_string();
                seen.lock().await.push((model.clone(), auth));

                let sse_body = if model == "lead-model" {
                    let s = st.fetch_add(1, Ordering::SeqCst);
                    if s == 0 {
                        let args = json!({
                            "wait": true,
                            "tasks": [
                                { "role": "Researcher", "specialist": "research", "prompt": "Research topic" },
                                { "role": "Coder", "specialist": "coding", "prompt": "Write code" }
                            ]
                        })
                        .to_string();
                        let chunk = json!({
                            "choices": [{
                                "delta": {
                                    "tool_calls": [{
                                        "index": 0,
                                        "id": "call_multi_prof",
                                        "type": "function",
                                        "function": { "name": "invoke_subagent", "arguments": args }
                                    }]
                                },
                                "finish_reason": "tool_calls"
                            }]
                        });
                        format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                    } else {
                        let chunk = json!({
                            "choices": [{ "delta": { "content": "PROFILES_OK" }, "finish_reason": "stop" }]
                        });
                        format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                    }
                } else {
                    let chunk = json!({
                        "choices": [{ "delta": { "content": format!("DONE_BY_{}", model) }, "finish_reason": "stop" }]
                    });
                    format!("data: {}\n\ndata: [DONE]\n\n", chunk)
                };
                axum::response::Response::builder()
                    .header("Content-Type", "text/event-stream")
                    .body(axum::body::Body::from(sse_body))
                    .unwrap()
            }
        }),
    );

    let ai_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ai_addr = ai_listener.local_addr().unwrap();
    let ai_handle = tokio::spawn(async move {
        axum::serve(ai_listener, fake_ai).await.unwrap();
    });

    let base_url = format!("http://{}/v1", ai_addr);
    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.permissions.mode = PermissionMode::Autonomous;
    cfg.custom_providers = vec![
        CustomProvider {
            id: "p1".into(),
            name: "Provider 1".into(),
            api_base: base_url.clone(),
            api_key: "key-p1".into(),
            default_model: "research-model".into(),
            is_active: true,
            enabled: true,
        },
        CustomProvider {
            id: "p2".into(),
            name: "Provider 2".into(),
            api_base: base_url.clone(),
            api_key: "key-p2".into(),
            default_model: "coding-model".into(),
            is_active: false,
            enabled: true,
        },
    ];
    cfg.provider = "p1".into();
    cfg.api_base = base_url;
    cfg.api_key = "key-p1".into();
    cfg.model = "lead-model".into();
    cfg.orchestrator = OrchestratorConfig {
        enabled: true,
        provider_id: "p1".into(),
        model: "lead-model".into(),
        thinking_effort: "high".into(),
        subagent_mode: "multi".into(),
        subagent_profiles: vec![
            SubagentProfileConfig {
                id: "research".into(),
                name: "Research".into(),
                description: "Web and reference research".into(),
                provider_id: "p1".into(),
                model: "research-model".into(),
                thinking_effort: "low".into(),
                server_id: "local".into(),
                read_only: true,
            },
            SubagentProfileConfig {
                id: "coding".into(),
                name: "Coding".into(),
                description: "Implementation worker".into(),
                provider_id: "p2".into(),
                model: "coding-model".into(),
                thinking_effort: "high".into(),
                server_id: "local".into(),
                read_only: false,
            },
        ],
        ..Default::default()
    };

    let state = AppState::new(cfg.clone());
    let registry = state.get_registry_snapshot().await;
    let model = Arc::new(deepseek_harness_rs::models::openai_client::OpenAiClient::new(cfg.clone()));
    let runner = AgentLoopRunner::new(
        "ses-multi-prof".into(),
        "Run research and coding profiles".into(),
        cfg,
        registry,
        model,
        Some(state),
    );
    let (evt_tx, _evt_rx) = mpsc::channel(64);
    let ans = runner.run(evt_tx).await.expect("runner ok");
    assert_eq!(ans, "PROFILES_OK");

    let recorded = seen_models.lock().await;
    assert!(recorded.contains(&("research-model".to_string(), "Bearer key-p1".to_string())));
    assert!(recorded.contains(&("coding-model".to_string(), "Bearer key-p2".to_string())));

    ai_handle.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_project_defaults_and_conversation_config_persistence() {
    use deepseek_harness_rs::config::ProjectDefaults;
    use deepseek_harness_rs::db::HarnessDb;

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_proj_defaults_test_{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let db_path = temp_dir.join("proj_defaults.db");
    let extra_dir = temp_dir.join("extra-folder");
    tokio::fs::create_dir_all(&extra_dir).await.unwrap();

    let mut cfg = HarnessConfig::default();
    cfg.sandbox_root = temp_dir.clone();
    cfg.db_path = Some(db_path.clone());
    // Mock mode forces the model to "mock"; keep it off so project model defaults
    // can be asserted.
    cfg.mock_mode = false;

    let db = HarnessDb::open(&db_path).expect("open sqlite db");
    let state = AppState::with_db(cfg, db);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let http = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. Set project defaults on the active default project via PATCH /api/projects/:id/defaults
    let projects: Value = http
        .get(format!("{}/api/projects", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let active_pid = projects[0]["id"].as_str().expect("a default project").to_string();
    let defaults_payload = json!({
        "name": "Harness Pro",
        "icon": "🦀",
        "extra_paths": [extra_dir.to_string_lossy().to_string()],
        "defaults": ProjectDefaults {
            provider_id: "deepseek".into(),
            model: "deepseek-reasoner".into(),
            reasoning_effort: "extra_high".into(),
            server_id: "local".into(),
            orchestrator_mode: Some(true),
        }
    });
    let upd_proj: Value = http
        .patch(format!("{}/api/projects/{}/defaults", base, active_pid))
        .json(&defaults_payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(upd_proj["name"], "Harness Pro");
    assert_eq!(upd_proj["defaults"]["model"], "deepseek-reasoner");
    assert_eq!(upd_proj["defaults"]["reasoning_effort"], "extra_high");
    assert_eq!(upd_proj["defaults"]["orchestrator_mode"], true);
    let extra_paths = upd_proj["extra_paths"].as_array().expect("extra_paths array");
    assert_eq!(extra_paths.len(), 1, "extra folder should be persisted");
    assert!(
        extra_paths[0].as_str().unwrap_or("").contains("extra-folder"),
        "expected the extra folder path, got {:?}",
        extra_paths[0]
    );

    // 2. Create idle conversation inside 'harness' and verify it inherits project defaults & stores reasoning_effort
    let conv: Value = http
        .post(format!("{}/api/sessions/create", base))
        .json(&json!({
            "title": "Inherited Chat",
            "project_id": active_pid
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = conv["id"].as_str().unwrap().to_string();
    assert_eq!(conv["reasoning_effort"], "extra_high");
    assert_eq!(conv["orchestrator_mode"], true);
    assert_eq!(conv["model"], "deepseek-reasoner");
    assert_eq!(conv["provider_id"], "deepseek");

    // 2b. Regression: the UI sends empty strings for untouched fields. Those must
    // mean "use the project default", not "clear the override".
    let conv_empty: Value = http
        .post(format!("{}/api/sessions/create", base))
        .json(&json!({
            "title": "Empty Overrides",
            "project_id": active_pid,
            "model": "",
            "provider_id": "",
            "reasoning_effort": ""
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(conv_empty["model"], "deepseek-reasoner");
    assert_eq!(conv_empty["provider_id"], "deepseek");
    assert_eq!(conv_empty["reasoning_effort"], "extra_high");

    // 3. Update chat-specific model/provider/effort via PATCH /api/sessions/:id
    let patch_res = http
        .patch(format!("{}/api/sessions/{}", base, cid))
        .json(&json!({
            "model": "gpt-oss-120b",
            "provider_id": "openrouter",
            "reasoning_effort": "low",
            "orchestrator_mode": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_res.status(), 200);

    let list: Vec<Value> = http
        .get(format!("{}/api/sessions", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entry = list.iter().find(|s| s["id"] == cid).unwrap();
    assert_eq!(entry["model"], "gpt-oss-120b");
    assert_eq!(entry["provider_id"], "openrouter");
    assert_eq!(entry["reasoning_effort"], "low");
    assert_eq!(entry["orchestrator_mode"], false);

    srv.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_filesystem_browse_mkdir_and_project_directory_flow() {
    use deepseek_harness_rs::config::Project;
    use deepseek_harness_rs::db::HarnessDb;

    let temp_dir = std::env::temp_dir().join(format!(
        "harness_browse_test_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let db_path = temp_dir.join("harness.db");

    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.db_path = Some(db_path.clone());
    config.projects = vec![Project {
        id: "harness".to_string(),
        name: "Harness".to_string(),
        path: temp_dir.clone(),
        icon: "\u{1F4C1}".to_string(),
        created_at: chrono::Local::now().to_rfc3339(),
        is_active: true,
        defaults: Default::default(),
        extra_paths: Vec::new(),
    }];
    config.active_project_id = "harness".to_string();

    let db = HarnessDb::open(&db_path).expect("open sqlite db");
    let state = AppState::with_db(config, db);

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base = format!("http://{}", addr);
    let http = reqwest::Client::new();

    // 1. Browsing a non-existent child path without strict falls back to existing ancestor
    let nonexistent = temp_dir.join("missing-sub").join("deep");
    let fb_res = http
        .get(format!("{}/api/filesystem/browse", base))
        .query(&[("path", nonexistent.display().to_string())])
        .send()
        .await
        .unwrap();
    assert_eq!(fb_res.status(), 200);
    let fb_json: Value = fb_res.json().await.unwrap();
    assert!(fb_json["notice"].as_str().unwrap_or("").contains("does not exist yet"));
    assert!(fb_json["entries"].is_array());

    // 2. Browsing a non-existent path with strict=true returns 400 JSON error
    let strict_res = http
        .get(format!("{}/api/filesystem/browse", base))
        .query(&[
            ("path", nonexistent.display().to_string()),
            ("strict", "true".to_string()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(strict_res.status(), 400);
    let strict_err: Value = strict_res.json().await.unwrap();
    assert!(strict_err["error"].as_str().unwrap_or("").contains("does not exist"));

    // 3. mkdir rejects empty parent_path and succeeds on valid parent_path
    let bad_mkdir = http
        .post(format!("{}/api/filesystem/mkdir", base))
        .json(&json!({ "parent_path": "", "name": "foo" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_mkdir.status(), 400);
    let bad_err: Value = bad_mkdir.json().await.unwrap();
    assert!(bad_err["error"].as_str().unwrap_or("").contains("empty"));

    let ok_mkdir: Value = http
        .post(format!("{}/api/filesystem/mkdir", base))
        .json(&json!({
            "parent_path": temp_dir.display().to_string(),
            "name": "workspace-a"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ok_mkdir["success"], true);
    let created_dir = temp_dir.join("workspace-a");
    assert!(created_dir.is_dir());

    // 4. Duplicate project names generate distinct IDs instead of overwriting
    let p1: Value = http
        .post(format!("{}/api/projects", base))
        .json(&json!({
            "name": "Demo App",
            "path": created_dir.display().to_string(),
            "icon": "🚀"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p1["id"], "demo-app");

    let dir_b = temp_dir.join("workspace-b");
    let p2: Value = http
        .post(format!("{}/api/projects", base))
        .json(&json!({
            "name": "Demo App",
            "path": dir_b.display().to_string(),
            "icon": "⚡"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p2["id"], "demo-app-2");
    assert!(dir_b.is_dir());

    // 5. Updating project defaults can update the project's workspace path
    let dir_c = temp_dir.join("workspace-c");
    let upd: Value = http
        .patch(format!("{}/api/projects/demo-app-2/defaults", base))
        .json(&json!({
            "name": "Demo App Renamed",
            "path": dir_c.display().to_string(),
            "defaults": {}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(upd["name"], "Demo App Renamed");
    assert!(upd["path"].as_str().unwrap().contains("workspace-c"));
    assert!(dir_c.is_dir());

    srv.abort();
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

// ============================================================================
// Security hardening: secret redaction + auth middleware
// ============================================================================

#[tokio::test]
async fn test_config_secrets_are_redacted_and_preserved_on_repost() {
    use deepseek_harness_rs::secret_redaction::SECRET_REDACTED;
    let temp_dir = std::env::temp_dir().join(format!("harness-sec-redact-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.api_key = "sk-live-secret".to_string();
    config.search.tavily_api_key = "tv-live-secret".to_string();
    config.auth_token = String::new();
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // GET must never leak the stored keys.
    let got: Value = client.get(format!("{}/api/config", base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(got["api_key"], SECRET_REDACTED);
    assert_eq!(got["search"]["tavily_api_key"], SECRET_REDACTED);
    assert!(!got.to_string().contains("sk-live-secret"));

    // Re-POSTing the redacted snapshot must not wipe the stored key.
    let resp = client.post(format!("{}/api/config", base)).json(&got).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(state.config.read().await.api_key, "sk-live-secret");
    assert_eq!(state.config.read().await.search.tavily_api_key, "tv-live-secret");

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_auth_token_required_when_configured() {
    let temp_dir = std::env::temp_dir().join(format!("harness-sec-auth-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.host = "127.0.0.1".to_string();
    config.auth_token = "test-token-123".to_string();
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // No token -> 401
    let anon = client.get(format!("{}/api/config", base)).send().await.unwrap();
    assert_eq!(anon.status(), 401);
    // Wrong token -> 401
    let wrong = client
        .get(format!("{}/api/config", base))
        .header("Authorization", "Bearer nope")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    // Correct token -> 200
    let ok = client
        .get(format!("{}/api/config", base))
        .header("Authorization", "Bearer test-token-123")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_dns_rebinding_host_header_is_rejected() {
    let temp_dir = std::env::temp_dir().join(format!("harness-sec-host-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.host = "127.0.0.1".to_string();
    config.auth_token = String::new();
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/config", addr))
        .header("Host", "evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}
// ============================================================================
// Encryption at rest
// ============================================================================

#[test]
fn test_credentials_are_encrypted_at_rest_and_migrate() {
    let dir = std::env::temp_dir().join(format!("harness-enc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("harness.db");

    let mut config = HarnessConfig::default();
    config.sandbox_root = dir.clone();
    config.api_key = "sk-super-secret-value".to_string();
    config.search.tavily_api_key = "tv-secret-value".to_string();
    config.custom_providers.push(deepseek_harness_rs::config::CustomProvider {
        id: "acme".into(),
        name: "Acme".into(),
        api_base: "https://acme.example/v1".into(),
        api_key: "acme-secret-value".into(),
        default_model: "m".into(),
        is_active: false,
        enabled: true,
    });

    {
        let db = deepseek_harness_rs::db::HarnessDb::open(&db_path).unwrap();
        db.save_config(&config).unwrap();
    }

    // The raw database file must not contain any secret in plaintext.
    let raw = std::fs::read(&db_path).unwrap();
    let text = String::from_utf8_lossy(&raw);
    for secret in ["sk-super-secret-value", "tv-secret-value", "acme-secret-value"] {
        assert!(!text.contains(secret), "database leaked {secret} in plaintext");
    }
    // A key file is created next to the database.
    assert!(dir.join("harness.key").is_file(), "master key file should exist");

    // Re-opening decrypts and restores every credential.
    let db2 = deepseek_harness_rs::db::HarnessDb::open(&db_path).unwrap();
    let loaded = db2.load_config().unwrap().unwrap();
    assert_eq!(loaded.api_key, "sk-super-secret-value");
    assert_eq!(loaded.search.tavily_api_key, "tv-secret-value");
    assert_eq!(
        loaded.custom_providers.iter().find(|p| p.id == "acme").unwrap().api_key,
        "acme-secret-value"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================================
// Auto-update helpers
// ============================================================================

#[test]
fn test_update_version_and_checksum_helpers() {
    use deepseek_harness_rs::updater::is_newer;
    assert!(is_newer("0.4.0", "0.3.0"));
    assert!(is_newer("v0.4.1", "0.4.0"));
    assert!(!is_newer("0.3.0", "0.3.0"));
    assert!(!is_newer("0.3.0", "0.4.0"));
    assert!(!is_newer("not-a-version", "0.3.0"));
}

#[tokio::test]
async fn test_security_status_endpoint() {
    let temp_dir = std::env::temp_dir().join(format!("harness-sec-status-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.host = "127.0.0.1".to_string();
    config.auth_token = String::new();
    config.auto_update_check = false;
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{}/api/security/status", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // Security headers must be present on every response.
    assert_eq!(resp.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
    assert_eq!(resp.headers().get("referrer-policy").unwrap(), "no-referrer");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["loopback_only"], true);
    assert_eq!(body["cors"], "same-origin");

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// Automations: "GitHub Repo Watcher" end-to-end
// ============================================================================

#[tokio::test]
async fn test_github_repo_watcher_automation_create_list_and_run() {
    let temp_dir = std::env::temp_dir().join(format!("harness-auto-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.mock_mode = true;
    config.projects = vec![deepseek_harness_rs::config::Project {
        id: "harness".to_string(),
        name: "Harness".to_string(),
        path: temp_dir.clone(),
        icon: "\u{1F4C1}".to_string(),
        created_at: chrono::Local::now().to_rfc3339(),
        is_active: true,
        defaults: Default::default(),
        extra_paths: Vec::new(),
    }];
    config.active_project_id = "harness".to_string();
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. Create the automation (cron: every 10 minutes).
    let created: Value = client
        .post(format!("{}/api/automations", base))
        .json(&serde_json::json!({
            "id": "github-repo-watcher",
            "name": "GitHub Repo Watcher",
            "description": "Watches a GitHub repository for new issues and summarises them.",
            "prompt": "Check the watched GitHub repository for new issues and pull requests, then summarise anything new.",
            "trigger": { "kind": "cron", "expr": "*/10 * * * *" },
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["id"], "github-repo-watcher");
    assert_eq!(created["trigger"]["kind"], "cron");

    // 2. It must appear in the list with a readable trigger label.
    let listed: Value = client
        .get(format!("{}/api/automations", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = listed["automations"].as_array().unwrap();
    let watcher = items
        .iter()
        .find(|a| a["id"] == "github-repo-watcher")
        .expect("watcher automation should be listed");
    assert_eq!(watcher["name"], "GitHub Repo Watcher");
    assert_eq!(watcher["trigger_label"], "Cron: */10 * * * *");
    assert_eq!(watcher["enabled"], true);

    // 3. Persisted in config, not just returned.
    assert!(state
        .config
        .read()
        .await
        .automations
        .iter()
        .any(|a| a.id == "github-repo-watcher"));

    // 4. Trigger it now. A run is a *log entry*, never a conversation.
    let run: Value = client
        .post(format!("{}/api/automations/github-repo-watcher/run", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(run["status"], "started");
    let run_id = run["run_id"].as_str().unwrap().to_string();
    assert!(
        run_id.starts_with("auto-github-repo-watcher-"),
        "run ids are namespaced by automation: {run_id}"
    );
    assert!(run["session_id"].is_null(), "runs must not be sessions");

    // The run produced a log row rather than a conversation. A run is async, so
    // poll for its terminal state instead of assuming it already finished.
    let mut log_rows: Vec<Value> = Vec::new();
    for _ in 0..120 {
        let logs: Value = client
            .get(format!("{}/api/automations/github-repo-watcher/logs", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        log_rows = logs["logs"].as_array().unwrap().clone();
        let done = log_rows
            .iter()
            .find(|l| l["run_id"] == Value::from(run_id.as_str()))
            .and_then(|l| l["status"].as_str())
            .map(|s| s != "running")
            .unwrap_or(false);
        if done {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!log_rows.is_empty(), "the run must be logged");
    let entry = log_rows
        .iter()
        .find(|l| l["run_id"] == Value::from(run_id.as_str()))
        .expect("the run must be in the log");
    assert_eq!(entry["status"], "success", "run error: {}", entry["error"]);
    assert!(
        entry["output"].as_str().map(|s| !s.trim().is_empty()).unwrap_or(false),
        "the final answer is logged"
    );
    assert!(entry["finished_at"].is_string());
    // No conversation was created for the run.
    let sessions: Value = client
        .get(format!("{}/api/sessions", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_list = sessions
        .as_array()
        .cloned()
        .or_else(|| sessions.get("sessions").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();
    assert!(
        session_list.iter().all(|s| s["id"] != Value::from(run_id.as_str())),
        "an automation run must never appear as a conversation"
    );

    // 4b. Editing an existing automation in place.
    let edited: Value = client
        .patch(format!("{}/api/automations/github-repo-watcher", base))
        .json(&serde_json::json!({ "description": "Watches and triages new issues." }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edited["status"], "updated");
    assert_eq!(edited["automation"]["description"], "Watches and triages new issues.");
    assert_eq!(
        edited["automation"]["trigger"]["kind"], "cron",
        "untouched fields are preserved"
    );
    assert_eq!(edited["automation"]["run_count"], 1, "run history survives an edit");

    // An invalid edit is rejected and changes nothing.
    let bad_edit = client
        .patch(format!("{}/api/automations/github-repo-watcher", base))
        .json(&serde_json::json!({ "trigger": { "kind": "cron", "expr": "nope" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_edit.status(), 400);

    // The automation recorded the run.
    let after = state
        .config
        .read()
        .await
        .automations
        .iter()
        .find(|a| a.id == "github-repo-watcher")
        .cloned()
        .unwrap();
    assert_eq!(after.run_count, 1);
    assert!(after.last_run_at.is_some());

    // 5. Invalid cron is rejected.
    let bad = client
        .post(format!("{}/api/automations", base))
        .json(&serde_json::json!({
            "name": "Bad",
            "prompt": "x",
            "trigger": { "kind": "cron", "expr": "not a cron" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // 6. Delete removes it.
    let del = client
        .delete(format!("{}/api/automations/github-repo-watcher", base))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);
    assert!(!state
        .config
        .read()
        .await
        .automations
        .iter()
        .any(|a| a.id == "github-repo-watcher"));

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_automation_tool_creates_and_runs() {
    let temp_dir = std::env::temp_dir().join(format!("harness-auto-tool-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.mock_mode = true;
    let state = AppState::new(config);

    let ctx = deepseek_harness_rs::plugins::SessionToolContext {
        session_id: "s".into(),
        agent_id: "main".into(),
        parent_id: None,
        depth: 0,
        max_depth: 2,
        state: Some(state.clone()),
        swarm: None,
        event_tx: None,
        config: state.config.read().await.clone(),
        registry: deepseek_harness_rs::plugins::PluginRegistry::new(&state.config.read().await.clone()),
        model: Arc::new(MockClient),
        cancel_notify: Arc::new(Notify::new()),
        sandbox_roots: Vec::new(),
    };

    let plugin = deepseek_harness_rs::plugins::automation_tool::ManageAutomationsPlugin;
    use deepseek_harness_rs::plugins::ToolPlugin;

    // Create via the tool (the agent-facing path).
    let out = plugin
        .execute_with_context(
            serde_json::json!({
                "action": "create",
                "name": "GitHub Repo Watcher",
                "prompt": "Summarise new issues in the watched repo.",
                "trigger": "interval",
                "every": "30m",
                "description": "Watches a repo"
            }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(out.contains("github-repo-watcher"), "unexpected: {out}");

    let stored = state
        .config
        .read()
        .await
        .automations
        .iter()
        .find(|a| a.id == "github-repo-watcher")
        .cloned()
        .expect("tool must persist the automation");
    assert_eq!(stored.name, "GitHub Repo Watcher");
    assert_eq!(
        stored.trigger,
        deepseek_harness_rs::automation::AutomationTrigger::Interval { every_secs: 1800 }
    );

    // Run it through the tool.
    let run = plugin
        .execute_with_context(
            serde_json::json!({ "action": "run", "id": "github-repo-watcher" }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(run.contains("started"), "unexpected: {run}");

    // Disable and list.
    let disabled = plugin
        .execute_with_context(
            serde_json::json!({ "action": "disable", "id": "github-repo-watcher" }),
            &temp_dir,
            Some(&ctx),
        )
        .await
        .unwrap();
    assert!(disabled.contains("disabled"));
    assert!(!state
        .config
        .read()
        .await
        .automations
        .iter()
        .find(|a| a.id == "github-repo-watcher")
        .unwrap()
        .enabled);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
// ============================================================================
// Defaults: no providers, project folder beside the executable, onboarding
// ============================================================================

#[test]
fn test_no_default_providers_and_default_project_folder() {
    let cfg = HarnessConfig::default();
    assert!(
        cfg.custom_providers.is_empty(),
        "the harness must ship with zero providers"
    );
    assert_eq!(cfg.provider, "");
    assert_eq!(cfg.api_key, "");
    assert_eq!(cfg.api_base, "");

    // The default project is a `Project` folder beside the executable.
    assert_eq!(cfg.projects.len(), 1);
    let proj = &cfg.projects[0];
    assert_eq!(proj.id, "project");
    assert_eq!(proj.name, "Project");
    assert!(proj.path.ends_with("Project"), "path was {}", proj.path.display());
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    assert_eq!(proj.path, exe_dir.join("Project"));
    assert_eq!(cfg.active_project_id, "project");
    assert!(!cfg.onboarded, "a fresh install starts un-onboarded");
}

#[test]
fn test_shipped_default_provider_detection() {
    use deepseek_harness_rs::config::{is_shipped_default_provider, CustomProvider};
    let preset = |id: &str, base: &str, key: &str| CustomProvider {
        id: id.into(),
        name: id.into(),
        api_base: base.into(),
        api_key: key.into(),
        default_model: "m".into(),
        is_active: false,
        enabled: true,
    };
    // Untouched presets are recognised (so they can be purged).
    assert!(is_shipped_default_provider(&preset("ollama", "http://localhost:11434/v1", "ollama")));
    assert!(is_shipped_default_provider(&preset("nvidia", "https://integrate.api.nvidia.com/v1", "")));
    // A real user credential keeps the provider.
    assert!(!is_shipped_default_provider(&preset("nvidia", "https://integrate.api.nvidia.com/v1", "nvapi-real")));
    // A user-defined provider is never purged.
    assert!(!is_shipped_default_provider(&preset("my-local", "http://localhost:1234/v1", "")));
}

#[tokio::test]
async fn test_onboarding_endpoints_and_completion() {
    let temp_dir = std::env::temp_dir().join(format!("harness-onb-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.auto_update_check = false;
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let before: Value = client
        .get(format!("{}/api/onboarding", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["onboarded"], false);
    assert_eq!(before["provider_configured"], false);
    assert_eq!(before["project_name"], "Project");

    let done = client
        .post(format!("{}/api/onboarding/complete", base))
        .json(&serde_json::json!({ "skipped": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(done.status(), 200);

    let after: Value = client
        .get(format!("{}/api/onboarding", base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["onboarded"], true);
    assert!(state.config.read().await.onboarded);

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_filesystem_rename_and_delete() {
    let temp_dir = std::env::temp_dir().join(format!("harness-fsops-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file = temp_dir.join("before.txt");
    std::fs::write(&file, "hello").unwrap();
    let folder = temp_dir.join("sub");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("inner.txt"), "x").unwrap();

    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.auto_update_check = false;
    let state = AppState::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // Rename a file.
    let r = client
        .post(format!("{}/api/filesystem/rename", base))
        .json(&serde_json::json!({ "path": file.display().to_string(), "new_name": "after.txt" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(temp_dir.join("after.txt").is_file());
    assert!(!file.exists());

    // Rename rejects path separators in the new name.
    let bad = client
        .post(format!("{}/api/filesystem/rename", base))
        .json(&serde_json::json!({ "path": temp_dir.join("after.txt").display().to_string(), "new_name": "a/b" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // Delete a folder recursively.
    let d = client
        .post(format!("{}/api/filesystem/delete", base))
        .json(&serde_json::json!({ "path": folder.display().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(d.status(), 200);
    assert!(!folder.exists());

    // Deleting a missing path is a 404.
    let missing = client
        .post(format!("{}/api/filesystem/delete", base))
        .json(&serde_json::json!({ "path": temp_dir.join("nope").display().to_string() }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_ui_assets_are_served_without_browser_caching() {
    // Embedded assets change when the binary updates. If a browser reuses an
    // old app.js or index.html, the UI can break until a manual hard refresh.
    let temp_dir = std::env::temp_dir().join(format!("harness-ui-cache-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.auth_token = String::new();
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    for path in ["/", "/static/app.js", "/static/style.css"] {
        let resp = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(resp.status(), 200, "{path} should be served");
        let cache = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            cache.contains("no-store"),
            "{path} must not be cached, got Cache-Control: '{cache}'"
        );
    }

    // The shell must keep the top-level modals and the fixed automation button.
    let html = client.get(format!("{base}/")).send().await.unwrap().text().await.unwrap();
    assert!(html.contains("id=\"project-settings-modal\""));
    assert!(html.contains("id=\"file-browser-modal\""));
    assert!(html.contains("id=\"btn-sidebar-new-automation\""));
    // Automatic update progress drawer + restart overlay must ship with the UI.
    assert!(html.contains("id=\"update-drawer\""));
    assert!(html.contains("id=\"update-restart-overlay\""));

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_update_download_streams_progress_and_verifies_checksum() {
    use deepseek_harness_rs::updater::{download_with_progress, ReleaseAsset, UpdateInfo};

    let payload: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
    let digest = deepseek_harness_rs::crypto::sha256_hex(&payload);
    let good_sums = format!("{digest}  harness.exe\n");
    let bad_sums = format!("{}  harness.exe\n", "0".repeat(64));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{}", addr);
    let router = axum::Router::new()
        .route(
            "/release/harness.exe",
            axum::routing::get({
                let body = payload.clone();
                move || async move { body }
            }),
        )
        .route(
            "/release/SHA256SUMS",
            axum::routing::get({
                let body = good_sums.clone();
                move || async move { body }
            }),
        )
        .route(
            "/release/BAD-SHA256SUMS",
            axum::routing::get({
                let body = bad_sums.clone();
                move || async move { body }
            }),
        );
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mk_info = |sums_name: &str| UpdateInfo {
        current_version: "1.0.0".into(),
        latest_version: "9.9.9".into(),
        available: true,
        html_url: String::new(),
        notes: String::new(),
        published_at: String::new(),
        asset: Some(ReleaseAsset {
            name: "harness.exe".into(),
            url: format!("{base}/release/harness.exe"),
            size: payload.len() as u64,
        }),
        checksums: Some(ReleaseAsset {
            name: sums_name.into(),
            url: format!("{base}/release/{sums_name}"),
            size: 0,
        }),
    };

    let dest = std::env::temp_dir().join(format!("harness-upd-dl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();

    // 1) A good download reports streamed progress and writes the payload.
    let mut calls = 0usize;
    let mut last_done = 0u64;
    let path = download_with_progress(&mk_info("SHA256SUMS"), &dest, |done, total| {
        calls += 1;
        last_done = done;
        assert!(done <= total.unwrap_or(u64::MAX), "progress must not exceed the total");
    })
    .await
    .expect("verified download must succeed");
    assert!(calls >= 2, "progress must fire at least on start and at the end");
    assert_eq!(last_done, payload.len() as u64, "final progress equals the size");
    assert_eq!(std::fs::read(&path).unwrap(), payload, "payload must be intact");
    assert!(
        !dest.join("harness.exe.part").exists(),
        "no .part leftovers after a successful download"
    );

    // 2) A wrong published digest must abort the install.
    let mismatch = download_with_progress(&mk_info("BAD-SHA256SUMS"), &dest, |_, _| {}).await;
    let err = mismatch.expect_err("a checksum mismatch must fail").to_string();
    assert!(err.contains("Checksum mismatch"), "got: {err}");
    assert!(
        !dest.join("harness.exe.part").exists(),
        "a rejected payload must never be staged under its final name"
    );

    // 3) A missing checksums asset must also fail closed.
    let missing = download_with_progress(&mk_info("NOPE"), &dest, |_, _| {}).await;
    assert!(missing.is_err(), "an unavailable checksums asset must fail the install");

    srv.abort();
    let _ = std::fs::remove_dir_all(&dest);
}

#[tokio::test]
async fn test_update_request_retries_transient_github_failures() {
    use deepseek_harness_rs::updater::get_with_retry;
    use axum::response::IntoResponse;
    use std::sync::atomic::{AtomicU32, Ordering};

    let hits = Arc::new(AtomicU32::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let router = axum::Router::new().route(
        "/flaky",
        axum::routing::get({
            let hits = Arc::clone(&hits);
            move || {
                let hits = Arc::clone(&hits);
                async move {
                    let n = hits.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 3 {
                        // GitHub returns these under load; a single one must not
                        // abort the update.
                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "busy").into_response()
                    } else {
                        (axum::http::StatusCode::OK, "ok").into_response()
                    }
                }
            }
        }),
    );
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let resp = get_with_retry(&format!("http://{addr}/flaky"), None)
        .await
        .expect("transient failures must be retried");
    assert_eq!(resp.status(), 200, "must eventually succeed");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        3,
        "the first two 503s must be retried"
    );

    // A permanent failure still gives up, with a readable message.
    hits.store(0, Ordering::SeqCst);
    let resp = get_with_retry(&format!("http://{addr}/missing"), None)
        .await
        .expect("the response is returned, even as an error status");
    assert_eq!(resp.status(), 404);

    srv.abort();
}

#[tokio::test]
async fn test_update_status_endpoint_exposes_phase() {
    let temp_dir = std::env::temp_dir().join(format!("harness-upd-status-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut config = HarnessConfig::default();
    config.sandbox_root = temp_dir.clone();
    config.auth_token = String::new();
    let state = AppState::new(config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    let srv = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let got: Value = client
        .get(format!("{base}/api/update/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // The UI polls this to render the progress drawer; the phase is required.
    let phase = got["phase"].as_str().unwrap_or_default();
    assert!(
        [
            "idle",
            "checking",
            "available",
            "downloading",
            "verifying",
            "waiting_for_idle",
            "installing",
            "restarting",
            "up_to_date",
            "failed"
        ]
        .contains(&phase),
        "unexpected update phase: {phase}"
    );
    assert!(got["current_version"].as_str().is_some());

    srv.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}