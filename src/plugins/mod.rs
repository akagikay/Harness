use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

use crate::config::HarnessConfig;
use crate::engine::events::HarnessEvent;
use crate::engine::swarm::SwarmBoard;
use crate::models::ModelAdapter;
use crate::state::AppState;

pub mod automation_tool;
pub mod bash;
pub mod browser_ui;
pub mod contract;
pub mod custom_tool;
pub mod file_ops;
pub mod mcp;
pub mod meta_tools;
pub mod path_access_tool;
pub mod search;
pub mod skill_tool;
pub mod swarm_tools;
pub mod sys_info;
pub mod web_fetch;
pub mod web_search;

const BUILTIN_TOOL_NAMES: &[&str] = &[
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
    "create_skill",
    "manage_automations",
    "request_path_access",
];

pub fn is_builtin_tool_name(name: &str) -> bool {
    BUILTIN_TOOL_NAMES.contains(&name) || name.starts_with("mcp_")
}

/// Returns whether a tool may be advertised and executed inside a standalone Knowledge-Work
/// conversation (no project workspace attached: read/write/shell tools are disabled).
pub fn is_tool_allowed_in_knowledge_mode(
    name: &str,
    contract: Option<&contract::ToolContract>,
) -> bool {
    if matches!(
        name,
        "bash"
            | "read_file"
            | "write_file"
            | "edit_file"
            | "list_dir"
            | "grep_search"
            | "manage_tools"
            | "create_artifact"
            | "browser_ui"
    ) || name.starts_with("mcp_filesystem")
    {
        return false;
    }
    if let Some(c) = contract {
        if matches!(c.category.as_str(), "filesystem" | "shell") {
            return false;
        }
        if c.permission_level == contract::ToolPermission::WorkspaceWrite {
            return false;
        }
    }
    true
}

#[derive(Clone)]
pub struct SessionToolContext {
    pub session_id: String,
    pub agent_id: String,
    #[allow(dead_code)]
    pub parent_id: Option<String>,
    pub depth: usize,
    pub max_depth: usize,
    pub state: Option<AppState>,
    pub swarm: Option<SwarmBoard>,
    pub event_tx: Option<mpsc::Sender<HarnessEvent>>,
    pub config: HarnessConfig,
    pub registry: PluginRegistry,
    pub model: Arc<dyn ModelAdapter>,
    pub cancel_notify: Arc<Notify>,
    /// Every workspace folder the session may access (project folders, primary
    /// first). Falls back to the single `sandbox` argument when empty.
    pub sandbox_roots: Vec<std::path::PathBuf>,
}

#[async_trait]
pub trait ToolPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn contract(&self) -> contract::ToolContract;

    fn parameter_schema(&self) -> Value {
        self.contract().to_json_schema()
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String>;

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        _ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        self.execute(args, sandbox).await
    }
}

#[derive(Clone)]
pub struct PluginRegistry {
    tools: Arc<HashMap<String, Arc<dyn ToolPlugin>>>,
}

impl PluginRegistry {
    pub fn new(config: &HarnessConfig) -> Self {
        let mut registry = Self {
            tools: Arc::new(HashMap::new()),
        };
        registry.register_defaults(config);
        registry
    }

    pub fn register(&mut self, tool: Arc<dyn ToolPlugin>) {
        let name = tool.name().to_string();
        if let Some(map) = Arc::get_mut(&mut self.tools) {
            map.insert(name, tool);
        } else {
            let mut map = (*self.tools).clone();
            map.insert(name, tool);
            self.tools = Arc::new(map);
        }
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        if let Some(map) = Arc::get_mut(&mut self.tools) {
            map.remove(name).is_some()
        } else {
            let mut map = (*self.tools).clone();
            let removed = map.remove(name).is_some();
            if removed {
                self.tools = Arc::new(map);
            }
            removed
        }
    }

    pub fn register_defaults(&mut self, config: &HarnessConfig) {
        self.register(Arc::new(bash::BashPlugin));
        self.register(Arc::new(file_ops::ReadFilePlugin));
        self.register(Arc::new(file_ops::WriteFilePlugin));
        self.register(Arc::new(file_ops::EditFilePlugin));
        self.register(Arc::new(file_ops::ListDirPlugin));
        self.register(Arc::new(search::GrepSearchPlugin));
        self.register(Arc::new(sys_info::SysInfoPlugin));
        self.register(Arc::new(web_search::WebSearchPlugin::new(config.search.clone())));
        self.register(Arc::new(web_fetch::WebFetchPlugin::new(
            config.tinyfish_api_key.clone(),
        )));
        self.register(Arc::new(browser_ui::BrowserUiPlugin::new()));
        self.register(Arc::new(swarm_tools::InvokeSubagentPlugin));
        self.register(Arc::new(swarm_tools::SwarmBoardPlugin));
        self.register(Arc::new(swarm_tools::CreateArtifactPlugin));
        self.register(Arc::new(meta_tools::ManageToolsPlugin));
        self.register(Arc::new(skill_tool::CreateSkillPlugin));
        self.register(Arc::new(automation_tool::ManageAutomationsPlugin));
        self.register(Arc::new(path_access_tool::RequestPathAccessPlugin));

        self.load_custom_tools_from(&config.sandbox_root);

        for srv in &config.mcp_servers {
            if srv.enabled && srv.transport == "builtin" {
                for def in mcp::builtin_mcp_tools(srv) {
                    self.register_mcp_tool(
                        srv.clone(),
                        def.name,
                        def.description,
                        def.input_schema,
                    );
                }
            }
        }
    }

    pub fn load_custom_tools_from(&mut self, sandbox: &Path) {
        for spec in custom_tool::load_custom_tools_in(sandbox) {
            if spec.enabled && !self.is_builtin(&spec.contract.name) {
                self.register(Arc::new(custom_tool::DynamicCustomTool::new(spec)));
            }
        }
    }

    pub fn register_custom_tool(&mut self, mut spec: custom_tool::CustomToolSpec) -> Result<(), String> {
        spec.normalize();
        spec.validate_spec()?;
        if self.is_builtin(&spec.contract.name) {
            return Err(format!(
                "Cannot overwrite built-in tool '{}'.",
                spec.contract.name
            ));
        }
        let name = spec.contract.name.clone();
        if spec.enabled {
            self.register(Arc::new(custom_tool::DynamicCustomTool::new(spec)));
        } else {
            self.unregister(&name);
        }
        Ok(())
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        if is_builtin_tool_name(name) {
            return true;
        }
        self.tools
            .get(name)
            .map(|t| !t.contract().is_custom)
            .unwrap_or(false)
    }

    pub fn get_contracts(&self) -> Vec<contract::ToolContract> {
        let mut contracts: Vec<contract::ToolContract> =
            self.tools.values().map(|t| t.contract()).collect();
        contracts.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
        contracts
    }

    pub fn get_contract(&self, name: &str) -> Option<contract::ToolContract> {
        self.tools.get(name).map(|t| t.contract())
    }

    pub fn get_tools_schema(&self) -> Value {
        self.get_tools_schema_filtered(true)
    }

    pub fn get_tools_schema_filtered(&self, allow_subagents: bool) -> Value {
        self.get_tools_schema_for_mode(allow_subagents, false)
    }

    pub fn get_tools_schema_for_mode(&self, allow_subagents: bool, knowledge_only: bool) -> Value {
        let mut list: Vec<Value> = self
            .tools
            .values()
            .filter(|t| {
                if !allow_subagents && t.name() == "invoke_subagent" {
                    return false;
                }
                if knowledge_only && !is_tool_allowed_in_knowledge_mode(t.name(), Some(&t.contract())) {
                    return false;
                }
                true
            })
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameter_schema()
                    }
                })
            })
            .collect();

        // Sort for deterministic prompt caching
        list.sort_by(|a, b| {
            let na = a
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let nb = b
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            na.cmp(nb)
        });

        Value::Array(list)
    }

    #[allow(dead_code)]
    pub async fn execute(&self, name: &str, args: Value, sandbox: &Path) -> Result<String, String> {
        self.execute_with_context(name, args, sandbox, None).await
    }

    pub async fn execute_with_context(
        &self,
        name: &str,
        args: Value,
        sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let tool = self.tools.get(name).ok_or_else(|| {
            format!(
                "Unknown tool '{}'. Available tools: {:?}",
                name,
                self.tools.keys()
            )
        })?;
        let contract = tool.contract();
        let args = coerce_args_for_contract(&contract, args);
        contract.validate_args(&args)?;
        tool.execute_with_context(args, sandbox, ctx).await
    }

    #[allow(dead_code)]
    pub fn register_mcp_tool(
        &mut self,
        server: crate::config::McpServerConfig,
        internal_name: String,
        description: String,
        schema: Value,
    ) {
        let tool = Arc::new(mcp::DynamicMcpTool::new(
            server,
            internal_name,
            description,
            schema,
        ));
        self.register(tool);
    }

    /// Unregister any dynamically registered MCP tools belonging to `server_id`.
    pub fn unregister_mcp_server_tools(&mut self, server_id: &str) {
        let prefix = format!("mcp_{}_", mcp::sanitize_mcp_identifier(server_id));
        let matching: Vec<String> = self
            .tools
            .keys()
            .filter(|name| name.starts_with(&prefix))
            .cloned()
            .collect();
        for name in matching {
            self.unregister(&name);
        }
    }

    /// Register all discovered tools from an `McpTestResult` when `res.success` and `server.enabled`.
    pub fn register_mcp_test_result(
        &mut self,
        server: &crate::config::McpServerConfig,
        res: &mcp::McpTestResult,
    ) -> usize {
        self.unregister_mcp_server_tools(&server.id);
        if !server.enabled || !res.success {
            return 0;
        }
        let mut count = 0;
        for def in &res.tools {
            self.register_mcp_tool(
                server.clone(),
                def.name.clone(),
                def.description.clone(),
                def.input_schema.clone(),
            );
            count += 1;
        }
        count
    }

    /// Probe enabled external (stdio/SSE) MCP servers concurrently with bounded timeout and
    /// register their discovered tools without blocking or hanging startup.
    #[allow(dead_code)]
    pub async fn sync_external_mcp_servers(
        &mut self,
        servers: &[crate::config::McpServerConfig],
    ) -> Vec<mcp::McpTestResult> {
        let external: Vec<crate::config::McpServerConfig> = servers
            .iter()
            .filter(|s| s.enabled && s.transport != "builtin")
            .cloned()
            .collect();

        for s in servers {
            if !s.enabled && s.transport != "builtin" {
                self.unregister_mcp_server_tools(&s.id);
            }
        }

        if external.is_empty() {
            return Vec::new();
        }

        let probe_futures = external.into_iter().map(|srv| async move {
            let res = match tokio::time::timeout(
                std::time::Duration::from_secs(8),
                mcp::test_mcp_server(&srv),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => mcp::McpTestResult {
                    success: false,
                    server_id: srv.id.clone(),
                    latency_ms: 8000,
                    tools: Vec::new(),
                    error: Some("Timed out probing external MCP server".to_string()),
                },
            };
            (srv, res)
        });

        let outcomes = futures_util::future::join_all(probe_futures).await;
        let mut results = Vec::with_capacity(outcomes.len());
        for (srv, res) in outcomes {
            if res.success {
                self.register_mcp_test_result(&srv, &res);
            }
            results.push(res);
        }
        results
    }

    #[allow(dead_code)]
    pub fn list_tools(&self) -> Vec<(String, String)> {
        self.tools
            .iter()
            .map(|(k, v)| (k.clone(), v.description().to_string()))
            .collect()
    }
}

/// Trim accidental whitespace in argument keys and safely coerce stringified JSON scalars
/// (integers, numbers, booleans, arrays, objects) to match declared contract `FieldType`s.
pub fn coerce_args_for_contract(contract: &contract::ToolContract, args: Value) -> Value {
    let Value::Object(raw_map) = args else {
        return args;
    };

    let field_types: HashMap<&str, contract::FieldType> = contract
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.field_type))
        .collect();

    // Trim whitespace in object keys without clobbering exact-match keys.
    let mut trimmed_map = serde_json::Map::with_capacity(raw_map.len());
    let mut deferred = Vec::new();
    for (k, v) in raw_map {
        let trimmed = k.trim();
        if trimmed == k {
            trimmed_map.insert(k, v);
        } else {
            deferred.push((trimmed.to_string(), v));
        }
    }
    for (k, v) in deferred {
        trimmed_map.entry(k).or_insert(v);
    }

    for (k, v) in trimmed_map.iter_mut() {
        let Some(&expected) = field_types.get(k.as_str()) else {
            continue;
        };
        coerce_value_for_field_type(expected, v);
    }

    Value::Object(trimmed_map)
}

fn coerce_value_for_field_type(expected: contract::FieldType, val: &mut Value) {
    use contract::FieldType;

    match expected {
        FieldType::Integer => {
            if let Value::String(s) = val {
                let t = s.trim();
                if let Ok(n) = t.parse::<i64>() {
                    *val = Value::Number(n.into());
                } else if let Ok(n) = t.parse::<u64>() {
                    *val = Value::Number(n.into());
                }
            } else if let Value::Number(n) = val {
                if !(n.is_i64() || n.is_u64()) {
                    if let Some(f) = n.as_f64() {
                        if f.is_finite() && f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                            *val = Value::Number((f as i64).into());
                        }
                    }
                }
            }
        }
        FieldType::Number => {
            if let Value::String(s) = val {
                let t = s.trim();
                if let Ok(f) = t.parse::<f64>() {
                    if f.is_finite() {
                        if let Some(num) = serde_json::Number::from_f64(f) {
                            *val = Value::Number(num);
                        }
                    }
                }
            }
        }
        FieldType::Boolean => {
            if let Value::String(s) = val {
                match s.trim().to_ascii_lowercase().as_str() {
                    "true" => *val = Value::Bool(true),
                    "false" => *val = Value::Bool(false),
                    _ => {}
                }
            }
        }
        FieldType::Array => {
            if let Value::String(s) = val {
                let t = s.trim();
                if t.starts_with('[') {
                    if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(t) {
                        *val = Value::Array(arr);
                    }
                }
            }
        }
        FieldType::Object => {
            if let Value::String(s) = val {
                let t = s.trim();
                if t.starts_with('{') {
                    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(t) {
                        *val = Value::Object(obj);
                    }
                }
            }
        }
        FieldType::String => {}
    }
}
