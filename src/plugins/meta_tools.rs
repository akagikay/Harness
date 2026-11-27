use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::custom_tool::{
    delete_custom_tool, load_custom_tools_in, save_custom_tool, CustomToolSpec, DynamicCustomTool,
};
use super::{is_builtin_tool_name, SessionToolContext, ToolPlugin};

/// Meta-tool enabling the agent to inspect tool contracts and author/remove modular custom tools at runtime.
pub struct ManageToolsPlugin;

#[async_trait]
impl ToolPlugin for ManageToolsPlugin {
    fn name(&self) -> &str {
        "manage_tools"
    }

    fn description(&self) -> &str {
        "Inspect tool contracts or dynamically create, list, and remove project-local custom tools stored in .harness/tools/."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: "manage_tools".to_string(),
            description: self.description().to_string(),
            category: "meta".to_string(),
            permission_level: ToolPermission::Meta,
            fields: vec![
                ToolFieldSpec::required_string(
                    "action",
                    "Operation to perform: list, inspect, create, or remove.",
                    "Use one of: list, inspect, create, remove.",
                )
                .with_enum(["list", "inspect", "create", "remove"])
                .non_empty(),
                ToolFieldSpec::optional_string(
                    "name",
                    "Tool name (required for inspect, create, and remove; ASCII snake_case recommended).",
                )
                .with_error_hint("Provide a valid snake_case tool name like 'check_health'."),
                ToolFieldSpec::optional_string(
                    "description",
                    "Clear description of what the custom tool does (required for create).",
                ),
                ToolFieldSpec::optional_string(
                    "category",
                    "Logical category badge for the tool (default: 'custom').",
                )
                .with_default(json!("custom")),
                ToolFieldSpec::optional_string(
                    "permission_level",
                    "Permission tier for create: read_only, workspace_write, shell_or_network, high_risk, or meta.",
                )
                .with_enum([
                    "read_only",
                    "workspace_write",
                    "shell_or_network",
                    "high_risk",
                    "meta",
                ])
                .with_default(json!("shell_or_network")),
                ToolFieldSpec::optional_array(
                    "fields",
                    "Array of ToolFieldSpec objects defining input parameters, types, required flags, and error_hints.",
                ),
                ToolFieldSpec::optional_object(
                    "expected_output",
                    "OutputContract object { format, description, example } describing expected output.",
                ),
                ToolFieldSpec::optional_array(
                    "failure_modes",
                    "Array of ToolFailureMode objects { code, condition, error_message, recovery_hint }.",
                ),
                ToolFieldSpec::optional_int(
                    "timeout_secs",
                    "Maximum execution timeout in seconds (1..=600, default: 30).",
                )
                .with_range(Some(1.0), Some(600.0))
                .with_default(json!(30)),
                ToolFieldSpec::optional_object(
                    "runner",
                    "Execution runner for create: { type: 'shell', command_template } or { type: 'http', method, url_template, headers, body_template }.",
                ),
                ToolFieldSpec::optional_bool(
                    "enabled",
                    "Whether the custom tool is active upon creation (default: true).",
                )
                .with_default(json!(true)),
            ],
            expected_output: OutputContract::new(
                "json",
                "JSON object containing the requested tool list, full ToolContract, or creation/removal status.",
                r#"{"status":"created","name":"check_health","hot_registered":true}"#,
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_name",
                    "action is inspect, create, or remove without 'name'",
                    "Missing required parameter 'name' for action",
                    "Provide 'name' as an ASCII snake_case identifier.",
                ),
                ToolFailureMode::new(
                    "builtin_protected",
                    "Attempting to create or remove a built-in harness tool",
                    "Cannot overwrite or delete built-in tool",
                    "Choose a distinct custom tool name that does not collide with built-in tools.",
                ),
                ToolFailureMode::new(
                    "invalid_spec",
                    "Missing runner/description or invalid field contract on create",
                    "Invalid custom tool payload",
                    "Include description and runner ({ type: 'shell', command_template } or { type: 'http', url_template }).",
                ),
                ToolFailureMode::new(
                    "tool_not_found",
                    "inspect or remove targets an unknown tool name",
                    "Tool not found",
                    "Call manage_tools with action='list' first to see available tools.",
                ),
            ],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        self.execute_with_context(args, sandbox, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required field 'action'".to_string())?;

        match action {
            "list" => handle_list(sandbox, ctx).await,
            "inspect" => handle_inspect(&args, sandbox, ctx).await,
            "create" => handle_create(args, sandbox, ctx).await,
            "remove" => handle_remove(&args, sandbox, ctx).await,
            other => Err(format!(
                "Unknown manage_tools action '{}'. Expected one of: list, inspect, create, remove.",
                other
            )),
        }
    }
}

async fn collect_contracts(sandbox: &Path, ctx: Option<&SessionToolContext>) -> Vec<ToolContract> {
    let mut contracts = if let Some(c) = ctx {
        if let Some(ref st) = c.state {
            let reg = st.registry.read().await;
            reg.get_contracts()
        } else {
            c.registry.get_contracts()
        }
    } else {
        let mut list = vec![ManageToolsPlugin.contract()];
        for spec in load_custom_tools_in(sandbox) {
            list.push(spec.contract);
        }
        list
    };

    // Include disk custom tools not yet in the snapshot if no AppState was attached.
    if ctx.and_then(|c| c.state.as_ref()).is_none() {
        for spec in load_custom_tools_in(sandbox) {
            if !contracts.iter().any(|c| c.name == spec.contract.name) {
                contracts.push(spec.contract);
            }
        }
    }

    contracts.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    contracts
}

async fn handle_list(sandbox: &Path, ctx: Option<&SessionToolContext>) -> Result<String, String> {
    let contracts = collect_contracts(sandbox, ctx).await;
    let summaries: Vec<Value> = contracts
        .iter()
        .map(|c| {
            let required_fields: Vec<&str> = c
                .fields
                .iter()
                .filter(|f| f.required)
                .map(|f| f.name.as_str())
                .collect();
            json!({
                "name": c.name,
                "category": c.category,
                "permission_level": c.permission_level.as_str(),
                "is_custom": c.is_custom,
                "timeout_secs": c.timeout_secs,
                "description": c.description,
                "required_fields": required_fields,
                "field_count": c.fields.len()
            })
        })
        .collect();

    serde_json::to_string_pretty(&json!({
        "count": summaries.len(),
        "tools": summaries
    }))
    .map_err(|e| format!("Failed to serialize tool list: {}", e))
}

async fn handle_inspect(
    args: &Value,
    sandbox: &Path,
    ctx: Option<&SessionToolContext>,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required field 'name' for action 'inspect'.".to_string())?;

    let contracts = collect_contracts(sandbox, ctx).await;
    let contract = contracts
        .into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("Tool '{}' not found. Use action='list' to inspect registered tools.", name))?;

    serde_json::to_string_pretty(&contract)
        .map_err(|e| format!("Failed to serialize contract for '{}': {}", name, e))
}

async fn handle_create(
    mut args: Value,
    sandbox: &Path,
    ctx: Option<&SessionToolContext>,
) -> Result<String, String> {
    if let Some(obj) = args.as_object_mut() {
        obj.remove("action");
    }

    let spec = CustomToolSpec::from_json_value(args)?;
    let tool_name = spec.contract.name.clone();

    let builtin_in_state = if let Some(c) = ctx {
        if let Some(ref st) = c.state {
            st.registry.read().await.is_builtin(&tool_name)
        } else {
            c.registry.is_builtin(&tool_name)
        }
    } else {
        false
    };

    if builtin_in_state || is_builtin_tool_name(&tool_name) {
        return Err(format!(
            "Cannot overwrite built-in tool '{}'. Choose a different custom tool name.",
            tool_name
        ));
    }

    let saved_path = save_custom_tool(sandbox, &spec)?;

    let mut hot_registered = false;
    if let Some(c) = ctx {
        if let Some(ref st) = c.state {
            let mut reg = st.registry.write().await;
            if spec.enabled {
                reg.register(Arc::new(DynamicCustomTool::new(spec.clone())));
            } else {
                reg.unregister(&tool_name);
            }
            hot_registered = true;
        }
    }

    serde_json::to_string_pretty(&json!({
        "status": "created",
        "name": tool_name,
        "path": saved_path.display().to_string(),
        "enabled": spec.enabled,
        "hot_registered": hot_registered,
        "contract": spec.contract
    }))
    .map_err(|e| format!("Failed to serialize create result: {}", e))
}

async fn handle_remove(
    args: &Value,
    sandbox: &Path,
    ctx: Option<&SessionToolContext>,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing required field 'name' for action 'remove'.".to_string())?;

    let builtin_in_state = if let Some(c) = ctx {
        if let Some(ref st) = c.state {
            st.registry.read().await.is_builtin(name)
        } else {
            c.registry.is_builtin(name)
        }
    } else {
        false
    };

    if builtin_in_state || is_builtin_tool_name(name) {
        return Err(format!(
            "Cannot delete built-in tool '{}'. Only custom tools in .harness/tools/ can be removed.",
            name
        ));
    }

    delete_custom_tool(sandbox, name)?;

    let mut unregistered = false;
    if let Some(c) = ctx {
        if let Some(ref st) = c.state {
            let mut reg = st.registry.write().await;
            unregistered = reg.unregister(name);
        }
    }

    serde_json::to_string_pretty(&json!({
        "status": "removed",
        "name": name,
        "unregistered": unregistered
    }))
    .map_err(|e| format!("Failed to serialize remove result: {}", e))
}
