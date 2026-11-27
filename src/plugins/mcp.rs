use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::config::McpServerConfig;
use super::contract::{
    FieldType, OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::ToolPlugin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTestResult {
    pub success: bool,
    pub server_id: String,
    pub latency_ms: u64,
    pub tools: Vec<McpToolDefinition>,
    pub error: Option<String>,
}

/// Sanitize an MCP server id or tool name into a clean function-calling identifier segment.
pub fn sanitize_mcp_identifier(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Returns true when `cfg` refers to the built-in Google AI Search MCP server.
pub fn is_google_ai_search_server(cfg: &McpServerConfig) -> bool {
    (cfg.transport == "builtin" && cfg.id == "google-ai-search")
        || cfg.command == "builtin://google-ai-search"
}

/// Tool definition exposed by the built-in Google AI Search MCP server.
pub fn google_ai_search_tool_definition() -> McpToolDefinition {
    McpToolDefinition {
        name: "search".to_string(),
        description: "Search the live web using Google AI grounded search (Gemini Google Search grounding when GEMINI_API_KEY/GOOGLE_API_KEY is configured, with automatic pure-Rust Google/Wikipedia/DuckDuckGo fallback). Returns a synthesized AI overview and numbered cited sources.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Search query or research question."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 5,
                    "description": "Maximum number of search results or citations to return (1-10, default 5)."
                },
                "mode": {
                    "type": "string",
                    "enum": ["ai_overview", "web"],
                    "default": "ai_overview",
                    "description": "Search mode: 'ai_overview' for Google AI synthesized answer + citations, 'web' for fast search links."
                }
            },
            "required": ["query"]
        }),
    }
}

/// Synchronously enumerate tools exposed by a built-in MCP server (zero process/network overhead).
pub fn builtin_mcp_tools(cfg: &McpServerConfig) -> Vec<McpToolDefinition> {
    if is_google_ai_search_server(cfg) {
        vec![google_ai_search_tool_definition()]
    } else {
        Vec::new()
    }
}

#[allow(dead_code)]
pub struct DynamicMcpTool {
    pub tool_name: String,
    pub tool_description: String,
    pub schema: Value,
    pub server_config: McpServerConfig,
    pub mcp_internal_name: String,
}

impl DynamicMcpTool {
    #[allow(dead_code)]
    pub fn new(
        server_config: McpServerConfig,
        internal_name: String,
        description: String,
        schema: Value,
    ) -> Self {
        let sanitized_server = sanitize_mcp_identifier(&server_config.id);
        let sanitized_tool = sanitize_mcp_identifier(&internal_name);
        let tool_name = format!("mcp_{}_{}", sanitized_server, sanitized_tool);
        Self {
            tool_name,
            tool_description: description,
            schema,
            server_config,
            mcp_internal_name: internal_name,
        }
    }
}

#[async_trait]
impl ToolPlugin for DynamicMcpTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn contract(&self) -> ToolContract {
        let description = if self.tool_description.trim().is_empty() {
            format!(
                "Dynamic MCP tool '{}' exposed by MCP server '{}'.",
                self.mcp_internal_name, self.server_config.id
            )
        } else {
            self.tool_description.clone()
        };

        let is_builtin_google_search = self.server_config.transport == "builtin"
            && self.server_config.id == "google-ai-search";
        let permission_level = if is_builtin_google_search {
            ToolPermission::ReadOnly
        } else {
            ToolPermission::ShellOrNetwork
        };

        let (expected_output, failure_modes) = if is_builtin_google_search {
            (
                OutputContract::new(
                    "markdown",
                    "Google AI Search markdown response with synthesized AI Overview, numbered cited sources, and grounding source footer.",
                    "### Google AI Search for: 'rust async'\n\n**AI Overview**\n...\n\n**Cited Sources**\n1. [Async Book](https://rust-lang.github.io/async-book/)\n\n---\n*Grounding source: Gemini Grounded Search*",
                ),
                vec![
                    ToolFailureMode::new(
                        "invalid_query",
                        "Required 'query' field is missing or empty",
                        "Missing or empty required parameter 'query'",
                        "Provide a non-empty search query string.",
                    ),
                    ToolFailureMode::new(
                        "network_or_timeout",
                        "Upstream search endpoints fail or time out",
                        "Google AI Search fallback failed: <network error>",
                        "Check network connectivity and retry with a concise search query.",
                    ),
                ],
            )
        } else {
            (
                OutputContract::new(
                    "text",
                    format!(
                        "Text or JSON response returned by MCP tool '{}' on server '{}'.",
                        self.mcp_internal_name, self.server_config.id
                    ),
                    "MCP tool execution result",
                ),
                vec![
                    ToolFailureMode::new(
                        "mcp_server_unreachable",
                        "Stdio MCP command fails to spawn or SSE endpoint cannot be reached",
                        "Failed to spawn MCP server '<command>' / HTTP request to MCP server failed",
                        "Check the MCP server command, arguments, environment variables, or SSE URL.",
                    ),
                    ToolFailureMode::new(
                        "mcp_timeout",
                        "MCP server does not respond within the 30-second RPC timeout",
                        "Timed out waiting for MCP server response",
                        "Verify the MCP server is healthy and retry with smaller or simpler arguments.",
                    ),
                    ToolFailureMode::new(
                        "mcp_rpc_error",
                        "MCP server returns a JSON-RPC error or malformed response",
                        "MCP Server returned error: <error>",
                        "Inspect the error payload and verify parameter names/types against the MCP tool schema.",
                    ),
                ],
            )
        };

        ToolContract {
            name: self.tool_name.clone(),
            description,
            category: "mcp".to_string(),
            permission_level,
            fields: fields_from_json_schema(&self.schema),
            expected_output,
            failure_modes,
            timeout_secs: 30,
            is_custom: false,
        }
    }

    fn parameter_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value, _sandbox: &Path) -> Result<String, String> {
        execute_mcp_tool(&self.server_config, &self.mcp_internal_name, args).await
    }
}

#[allow(dead_code)]
fn fields_from_json_schema(schema: &Value) -> Vec<ToolFieldSpec> {
    let required_names: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|item| item.as_str()).collect())
        .unwrap_or_default();

    let Some(props) = schema.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut fields = Vec::with_capacity(props.len());
    for (name, prop) in props {
        let Some(field_type) = extract_field_type(prop.get("type")) else {
            continue;
        };
        let description = prop
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("MCP tool parameter.")
            .to_string();
        let required = required_names.contains(&name.as_str());
        let enum_values = prop
            .get("enum")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<String>>()
            })
            .filter(|v| !v.is_empty());
        let default = prop.get("default").cloned();
        let min = prop.get("minimum").and_then(|v| v.as_f64());
        let max = prop.get("maximum").and_then(|v| v.as_f64());
        let non_empty = prop
            .get("minLength")
            .and_then(|v| v.as_u64())
            .map(|m| m >= 1)
            .unwrap_or(false)
            || prop
                .get("minItems")
                .and_then(|v| v.as_u64())
                .map(|m| m >= 1)
                .unwrap_or(false);

        fields.push(ToolFieldSpec {
            name: name.clone(),
            field_type,
            description,
            required,
            default,
            enum_values,
            non_empty,
            min,
            max,
            error_hint: format!(
                "Provide a valid {} value for MCP parameter '{}'.",
                field_type, name
            ),
        });
    }
    fields
}

#[allow(dead_code)]
fn extract_field_type(type_val: Option<&Value>) -> Option<FieldType> {
    let type_str = match type_val {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Array(arr)) => {
            let non_null: Vec<&str> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| *s != "null")
                .collect();
            if non_null.len() == 1 {
                non_null[0]
            } else {
                return None;
            }
        }
        _ => return None,
    };

    match type_str {
        "string" => Some(FieldType::String),
        "integer" => Some(FieldType::Integer),
        "number" => Some(FieldType::Number),
        "boolean" => Some(FieldType::Boolean),
        "array" => Some(FieldType::Array),
        "object" => Some(FieldType::Object),
        _ => None,
    }
}

/// Test connection and query discovered tools from an MCP server
pub async fn test_mcp_server(cfg: &McpServerConfig) -> McpTestResult {
    let start = Instant::now();

    if is_google_ai_search_server(cfg) {
        return McpTestResult {
            success: true,
            server_id: cfg.id.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            tools: vec![google_ai_search_tool_definition()],
            error: None,
        };
    }

    if cfg.transport == "builtin" {
        return McpTestResult {
            success: false,
            server_id: cfg.id.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            tools: Vec::new(),
            error: Some(format!(
                "Unsupported built-in MCP server '{}' (command: '{}')",
                cfg.id, cfg.command
            )),
        };
    }

    if cfg.transport == "sse" || !cfg.url.is_empty() {
        return test_mcp_sse(cfg, start).await;
    }

    // stdio transport
    let mut cmd = build_stdio_command(cfg);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return McpTestResult {
                success: false,
                server_id: cfg.id.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                tools: Vec::new(),
                error: Some(format!("Failed to spawn command '{}': {}", cfg.command, e)),
            };
        }
    };

    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            let _ = child.kill().await;
            return McpTestResult {
                success: false,
                server_id: cfg.id.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                tools: Vec::new(),
                error: Some("Failed to open child stdin".to_string()),
            };
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill().await;
            return McpTestResult {
                success: false,
                server_id: cfg.id.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                tools: Vec::new(),
                error: Some("Failed to open child stdout".to_string()),
            };
        }
    };

    let mut reader = BufReader::new(stdout).lines();

    // 1. Send initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "deepseek-harness-rs",
                "version": "0.1.0"
            }
        }
    });

    let timeout_dur = Duration::from_secs(12);

    let res: Result<Vec<McpToolDefinition>, String> = async {
        let mut msg_str = serde_json::to_string(&init_req).map_err(|e| e.to_string())?;
        msg_str.push('\n');
        stdin.write_all(msg_str.as_bytes()).await.map_err(|e| format!("Write init error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Flush error: {}", e))?;

        // Read response to initialize
        let init_line = read_next_json_line(&mut reader, timeout_dur).await?;
        let _init_val: Value = serde_json::from_str(&init_line).map_err(|e| format!("Bad JSON response: {}", e))?;

        // 2. Send notifications/initialized
        let notify = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let mut notif_str = serde_json::to_string(&notify).map_err(|e| e.to_string())?;
        notif_str.push('\n');
        stdin.write_all(notif_str.as_bytes()).await.map_err(|e| format!("Write notify error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Flush error: {}", e))?;

        // 3. Send tools/list
        let tools_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        let mut tools_str = serde_json::to_string(&tools_req).map_err(|e| e.to_string())?;
        tools_str.push('\n');
        stdin.write_all(tools_str.as_bytes()).await.map_err(|e| format!("Write tools/list error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Flush error: {}", e))?;

        // Read response to tools/list
        let tools_line = read_next_json_line(&mut reader, timeout_dur).await?;
        let tools_val: Value = serde_json::from_str(&tools_line).map_err(|e| format!("Bad JSON in tools/list: {}", e))?;

        let mut defs = Vec::new();
        if let Some(tools_arr) = tools_val.get("result").and_then(|r| r.get("tools")).and_then(|t| t.as_array()) {
            for t in tools_arr {
                let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("unnamed").to_string();
                let desc = t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
                let schema = t.get("inputSchema").cloned().unwrap_or_else(|| serde_json::json!({ "type": "object" }));
                defs.push(McpToolDefinition {
                    name,
                    description: desc,
                    input_schema: schema,
                });
            }
        }
        Ok(defs)
    }.await;

    let _ = child.kill().await;

    match res {
        Ok(tools) => McpTestResult {
            success: true,
            server_id: cfg.id.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            tools,
            error: None,
        },
        Err(err) => McpTestResult {
            success: false,
            server_id: cfg.id.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            tools: Vec::new(),
            error: Some(err),
        },
    }
}

/// Execute a specific tool on an MCP server (built-in, SSE, or stdio JSON-RPC)
#[allow(dead_code)]
pub async fn execute_mcp_tool(cfg: &McpServerConfig, tool_name: &str, args: Value) -> Result<String, String> {
    if is_google_ai_search_server(cfg) {
        if tool_name != "search" {
            return Err(format!(
                "Unknown tool '{}' on built-in MCP server '{}'. Available tools: [search]",
                tool_name, cfg.id
            ));
        }
        return execute_google_ai_search_mcp(cfg, args).await;
    }

    if cfg.transport == "builtin" {
        return Err(format!(
            "Unsupported built-in MCP server '{}' (command: '{}')",
            cfg.id, cfg.command
        ));
    }

    if cfg.transport == "sse" || !cfg.url.is_empty() {
        return execute_mcp_sse(cfg, tool_name, args).await;
    }

    let mut cmd = build_stdio_command(cfg);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn MCP server '{}': {}", cfg.command, e))?;
    let mut stdin = child.stdin.take().ok_or_else(|| "Failed to open child stdin".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "Failed to open child stdout".to_string())?;
    let mut reader = BufReader::new(stdout).lines();

    let timeout_dur = Duration::from_secs(30);

    let exec_future = async {
        // 1. Initialize
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "deepseek-harness-rs",
                    "version": "0.1.0"
                }
            }
        });
        let mut msg_str = serde_json::to_string(&init_req).unwrap();
        msg_str.push('\n');
        stdin.write_all(msg_str.as_bytes()).await.map_err(|e| format!("Init write error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Init flush error: {}", e))?;

        let _ = read_next_json_line(&mut reader, timeout_dur).await?;

        // 2. Notification
        let notify = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let mut notif_str = serde_json::to_string(&notify).unwrap();
        notif_str.push('\n');
        stdin.write_all(notif_str.as_bytes()).await.map_err(|e| format!("Notify write error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Notify flush error: {}", e))?;

        // 3. Call tool
        let call_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args
            }
        });
        let mut call_str = serde_json::to_string(&call_req).unwrap();
        call_str.push('\n');
        stdin.write_all(call_str.as_bytes()).await.map_err(|e| format!("Tool call write error: {}", e))?;
        stdin.flush().await.map_err(|e| format!("Tool call flush error: {}", e))?;

        let call_resp_line = read_next_json_line(&mut reader, timeout_dur).await?;
        let resp_val: Value = serde_json::from_str(&call_resp_line).map_err(|e| format!("Invalid JSON response: {}", e))?;

        if let Some(err) = resp_val.get("error") {
            return Err(format!("MCP Server returned error: {}", err));
        }

        let result = resp_val.get("result").ok_or_else(|| "Missing 'result' in response".to_string())?;

        let mut out = String::new();
        if let Some(content_arr) = result.get("content").and_then(|c| c.as_array()) {
            for item in content_arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    out.push_str(text);
                    out.push('\n');
                } else {
                    out.push_str(&item.to_string());
                    out.push('\n');
                }
            }
        } else {
            out = result.to_string();
        }

        Ok(out.trim().to_string())
    };

    let result = exec_future.await;
    let _ = child.kill().await;
    result
}

/// Built-in Google AI Search MCP execution:
/// Uses Gemini Google Search grounding when a key is configured, falling back seamlessly
/// in pure Rust to Google Search web / Wikipedia / DuckDuckGo Instant Answer + HTML results.
pub async fn execute_google_ai_search_mcp(
    cfg: &McpServerConfig,
    args: Value,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing or empty required parameter 'query'".to_string())?;

    let max_results = args
        .get("max_results")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
                .or_else(|| v.as_f64().filter(|n| *n >= 1.0).map(|n| n as u64))
        })
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let mode = match args.get("mode").and_then(|v| v.as_str()) {
        Some("web") => "web",
        _ => "ai_overview",
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    if let Some(api_key) = resolve_gemini_api_key(cfg) {
        let model = resolve_gemini_model(cfg);
        if let Some(gemini_md) =
            try_gemini_grounded_search(&client, &api_key, &model, query, max_results, mode).await
        {
            return Ok(gemini_md);
        }
    }

    execute_web_grounded_fallback(&client, query, max_results, mode).await
}

fn resolve_gemini_api_key(cfg: &McpServerConfig) -> Option<String> {
    for key in ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_AI_API_KEY"] {
        if let Some(val) = cfg.env.get(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    for key in ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_AI_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn resolve_gemini_model(cfg: &McpServerConfig) -> String {
    if let Some(m) = cfg.env.get("GOOGLE_AI_MODEL") {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(m) = std::env::var("GOOGLE_AI_MODEL") {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "gemini-2.5-flash".to_string()
}

async fn try_gemini_grounded_search(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    query: &str,
    max_results: usize,
    mode: &str,
) -> Option<String> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        url_encode(model),
        url_encode(api_key)
    );

    let prompt = if mode == "web" {
        format!(
            "Search the web for: {query}\nProvide a brief 2-3 sentence factual summary and prioritize up to {max_results} authoritative web citations."
        )
    } else {
        format!(
            "Provide a clear, accurate, well-structured Google AI overview answering this query using live Google Search grounding: {query}"
        )
    };

    let payload = serde_json::json!({
        "contents": [
            {
                "parts": [
                    { "text": prompt }
                ]
            }
        ],
        "tools": [
            { "google_search": {} }
        ]
    });

    let resp = client.post(&url).json(&payload).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let body: Value = resp.json().await.ok()?;
    let candidate = body.get("candidates")?.as_array()?.first()?;

    let mut summary_parts = Vec::new();
    if let Some(parts) = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    summary_parts.push(trimmed.to_string());
                }
            }
        }
    }
    let summary = summary_parts.join("\n\n");

    let mut citations: Vec<(String, String)> = Vec::new();
    let mut seen_urls = HashSet::new();
    if let Some(chunks) = candidate
        .get("groundingMetadata")
        .and_then(|m| m.get("groundingChunks"))
        .and_then(|c| c.as_array())
    {
        for chunk in chunks {
            if citations.len() >= max_results {
                break;
            }
            if let Some(web) = chunk.get("web") {
                let uri = web.get("uri").and_then(|u| u.as_str()).unwrap_or("").trim();
                if uri.is_empty() {
                    continue;
                }
                let norm = normalize_url_key(uri);
                if !seen_urls.insert(norm) {
                    continue;
                }
                let title = web
                    .get("title")
                    .and_then(|t| t.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(uri);
                citations.push((title.to_string(), uri.to_string()));
            }
        }
    }

    if summary.is_empty() && citations.is_empty() {
        return None;
    }

    let mut md = format!("### Google AI Search for: '{}'\n\n", query);
    if !summary.is_empty() {
        let heading = if mode == "web" {
            "**Summary**"
        } else {
            "**AI Overview**"
        };
        md.push_str(&format!("{}\n\n{}\n\n", heading, summary));
    }

    if !citations.is_empty() {
        md.push_str("**Cited Sources**\n");
        for (idx, (title, uri)) in citations.iter().enumerate() {
            md.push_str(&format!("{}. [{}]({})\n", idx + 1, title, uri));
        }
        md.push('\n');
    }

    md.push_str(&format!(
        "---\n*Grounding source: Gemini Grounded Search (model: {})*",
        model
    ));
    Some(md)
}

#[derive(Debug, Clone)]
struct WebSearchHit {
    title: String,
    url: String,
    snippet: String,
}

async fn execute_web_grounded_fallback(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
    mode: &str,
) -> Result<String, String> {
    let (google_res, ddg_html_res, ddg_ia_res, wiki_res) = tokio::join!(
        fetch_google_web_results(client, query, max_results),
        fetch_duckduckgo_html(client, query, max_results),
        fetch_duckduckgo_instant(client, query, max_results),
        fetch_wikipedia_search(client, query, max_results.min(3)),
    );

    let mut errors = Vec::new();
    let mut all_hits = Vec::new();
    let mut ddg_abstract: Option<String> = None;

    match google_res {
        Ok(hits) => all_hits.extend(hits),
        Err(e) => errors.push(format!("Google web: {}", e)),
    }
    match ddg_html_res {
        Ok(hits) => all_hits.extend(hits),
        Err(e) => errors.push(format!("DuckDuckGo HTML: {}", e)),
    }
    match ddg_ia_res {
        Ok((abs, hits)) => {
            ddg_abstract = abs;
            all_hits.extend(hits);
        }
        Err(e) => errors.push(format!("DuckDuckGo IA: {}", e)),
    }
    match wiki_res {
        Ok(hits) => all_hits.extend(hits),
        Err(e) => errors.push(format!("Wikipedia: {}", e)),
    }

    let mut deduped: Vec<WebSearchHit> = Vec::with_capacity(max_results);
    let mut seen_urls = HashSet::new();
    for hit in all_hits {
        if deduped.len() >= max_results {
            break;
        }
        if hit.url.is_empty() || hit.url == "#" {
            continue;
        }
        let key = normalize_url_key(&hit.url);
        if seen_urls.insert(key) {
            deduped.push(hit);
        }
    }

    // If every single upstream failed at the network/HTTP layer and we have zero hits, surface the error.
    if deduped.is_empty() && ddg_abstract.is_none() && errors.len() >= 4 {
        return Err(format!(
            "Google/Web Grounded Fallback failed (upstream sources unreachable): {}",
            errors.join("; ")
        ));
    }

    let mut md = format!("### Google AI Search for: '{}'\n\n", query);

    if mode == "ai_overview" {
        md.push_str("**AI Overview**\n\n");
        if let Some(abs) = ddg_abstract.as_ref().filter(|s| !s.trim().is_empty()) {
            md.push_str(abs.trim());
            md.push_str("\n\n");
        } else if !deduped.is_empty() {
            let mut overview_lines = Vec::new();
            for hit in deduped.iter().take(3) {
                if !hit.snippet.is_empty() {
                    overview_lines.push(format!("- **{}**: {}", hit.title, hit.snippet));
                } else {
                    overview_lines.push(format!("- **{}** ({})", hit.title, hit.url));
                }
            }
            md.push_str(&overview_lines.join("\n"));
            md.push_str("\n\n");
        } else {
            md.push_str("No direct live results were returned for this query. Try refining or broadening your search terms.\n\n");
        }
    }

    if !deduped.is_empty() {
        md.push_str("**Cited Sources**\n");
        for (idx, hit) in deduped.iter().enumerate() {
            md.push_str(&format!("{}. [{}]({})\n", idx + 1, hit.title, hit.url));
            if !hit.snippet.is_empty() {
                md.push_str(&format!("   {}\n", hit.snippet));
            }
        }
        md.push('\n');
    } else if mode == "web" {
        md.push_str("No web links found for this query.\n\n");
    }

    md.push_str("---\n*Grounding source: Google/Web Grounded Fallback*");
    Ok(md)
}

async fn fetch_google_web_results(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<WebSearchHit>, String> {
    let url = format!(
        "https://www.google.com/search?q={}&num={}&hl=en",
        url_encode(query),
        max_results.clamp(1, 10)
    );
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(4))
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let html = resp.text().await.map_err(|e| e.to_string())?;
    let mut hits = Vec::new();
    let marker = "href=\"/url?q=";
    let mut rest = html.as_str();

    while hits.len() < max_results {
        let Some(pos) = rest.find(marker) else {
            break;
        };
        let after = &rest[pos + marker.len()..];
        let Some(end_quote) = after.find('"') else {
            break;
        };
        let raw_href = &after[..end_quote];
        let target_raw = raw_href.split('&').next().unwrap_or(raw_href);
        let decoded_url = url_decode(target_raw);

        let after_tag_start = &after[end_quote..];
        let title = if let Some(gt) = after_tag_start.find('>') {
            let inside = &after_tag_start[gt + 1..];
            if let Some(close_a) = inside.find("</a>") {
                clean_text(&inside[..close_a])
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if (decoded_url.starts_with("http://") || decoded_url.starts_with("https://"))
            && !decoded_url.contains("google.com/")
            && !decoded_url.contains("accounts.google.")
            && !decoded_url.contains("support.google.")
            && !title.is_empty()
        {
            hits.push(WebSearchHit {
                title,
                url: decoded_url,
                snippet: String::new(),
            });
        }

        rest = &after[end_quote + 1..];
    }

    Ok(hits)
}

async fn fetch_duckduckgo_instant(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<(Option<String>, Vec<WebSearchHit>), String> {
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        url_encode(query)
    );
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let val: Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut hits = Vec::new();
    let mut abstract_text = None;

    if let Some(abs) = val.get("AbstractText").and_then(|v| v.as_str()) {
        let trimmed = abs.trim();
        if !trimmed.is_empty() {
            abstract_text = Some(trimmed.to_string());
            let abs_url = val
                .get("AbstractURL")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let heading = val
                .get("Heading")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Overview")
                .to_string();
            if !abs_url.is_empty() {
                hits.push(WebSearchHit {
                    title: heading,
                    url: abs_url,
                    snippet: trimmed.to_string(),
                });
            }
        }
    }

    if let Some(topics) = val.get("RelatedTopics").and_then(|v| v.as_array()) {
        collect_ddg_topics(topics, max_results, &mut hits);
    }

    Ok((abstract_text, hits))
}

fn collect_ddg_topics(topics: &[Value], max_results: usize, hits: &mut Vec<WebSearchHit>) {
    for item in topics {
        if hits.len() >= max_results {
            break;
        }
        if let Some(nested) = item.get("Topics").and_then(|t| t.as_array()) {
            collect_ddg_topics(nested, max_results, hits);
            continue;
        }
        let text = item.get("Text").and_then(|t| t.as_str()).unwrap_or("").trim();
        let link = item.get("FirstURL").and_then(|u| u.as_str()).unwrap_or("").trim();
        if !text.is_empty() && !link.is_empty() {
            let title = text.split(" - ").next().unwrap_or(text).trim().to_string();
            hits.push(WebSearchHit {
                title,
                url: link.to_string(),
                snippet: text.to_string(),
            });
        }
    }
}

async fn fetch_duckduckgo_html(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<WebSearchHit>, String> {
    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode(query));
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(7))
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let html = resp.text().await.map_err(|e| e.to_string())?;
    let mut hits = Vec::new();
    let mut rest = html.as_str();

    while hits.len() < max_results {
        let Some(pos) = rest.find("class=\"result__a\"") else {
            break;
        };
        // Find enclosing <a ...> start backwards within a small window
        let prefix = &rest[..pos];
        let tag_start = prefix.rfind("<a ").unwrap_or(pos);
        let tag_slice = &rest[tag_start..];
        let Some(tag_end) = tag_slice.find('>') else {
            break;
        };
        let attrs = &tag_slice[..tag_end];
        let href = extract_attr(attrs, "href").unwrap_or_default();
        let resolved_url = resolve_ddg_href(&href);

        let after_open = &tag_slice[tag_end + 1..];
        let Some(close_a) = after_open.find("</a>") else {
            break;
        };
        let title = clean_text(&after_open[..close_a]);

        // Look ahead briefly for result__snippet before the next result__a
        let tail = &after_open[close_a + 4..];
        let next_result_idx = tail.find("class=\"result__a\"").unwrap_or(tail.len().min(4096));
        let block = &tail[..next_result_idx];
        let snippet = if let Some(sp) = block.find("class=\"result__snippet\"") {
            let s_slice = &block[sp..];
            if let Some(gt) = s_slice.find('>') {
                let inner = &s_slice[gt + 1..];
                let end_idx = inner
                    .find("</a>")
                    .or_else(|| inner.find("</span>"))
                    .or_else(|| inner.find("</div>"))
                    .unwrap_or(inner.len().min(600));
                clean_text(&inner[..end_idx])
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if (resolved_url.starts_with("http://") || resolved_url.starts_with("https://"))
            && !title.is_empty()
        {
            hits.push(WebSearchHit {
                title,
                url: resolved_url,
                snippet,
            });
        }

        rest = &after_open[close_a + 4..];
    }

    Ok(hits)
}

async fn fetch_wikipedia_search(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchHit>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit={}&utf8=1&format=json",
        url_encode(query),
        limit
    );
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let val: Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut hits = Vec::new();
    if let Some(items) = val
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.as_array())
    {
        for item in items.iter().take(limit) {
            let title = item
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if title.is_empty() {
                continue;
            }
            let raw_snippet = item.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
            let snippet = clean_text(raw_snippet);
            let page_slug = url_encode(&title.replace(' ', "_"));
            hits.push(WebSearchHit {
                title: format!("{} - Wikipedia", title),
                url: format!("https://en.wikipedia.org/wiki/{}", page_slug),
                snippet,
            });
        }
    }
    Ok(hits)
}

fn extract_attr(attrs: &str, attr_name: &str) -> Option<String> {
    let double_pat = format!("{}=\"", attr_name);
    if let Some(idx) = attrs.find(&double_pat) {
        let after = &attrs[idx + double_pat.len()..];
        if let Some(end) = after.find('"') {
            return Some(decode_html_entities(&after[..end]));
        }
    }
    let single_pat = format!("{}='", attr_name);
    if let Some(idx) = attrs.find(&single_pat) {
        let after = &attrs[idx + single_pat.len()..];
        if let Some(end) = after.find('\'') {
            return Some(decode_html_entities(&after[..end]));
        }
    }
    None
}

fn resolve_ddg_href(href: &str) -> String {
    let trimmed = href.trim();
    if let Some(idx) = trimmed.find("uddg=") {
        let after = &trimmed[idx + 5..];
        let encoded = after.split('&').next().unwrap_or(after);
        return url_decode(encoded);
    }
    if trimmed.starts_with("//") {
        return format!("https:{}", trimmed);
    }
    trimmed.to_string()
}

fn normalize_url_key(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn clean_text(raw_html: &str) -> String {
    let no_tags = strip_html_tags(raw_html);
    let decoded = decode_html_entities(&no_tags);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        if let Some(semi) = after.find(';') {
            if semi <= 10 {
                let entity = &after[1..semi];
                let replacement = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                        u32::from_str_radix(&entity[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    _ if entity.starts_with('#') => {
                        entity[1..].parse::<u32>().ok().and_then(char::from_u32)
                    }
                    _ => None,
                };
                if let Some(ch) = replacement {
                    out.push(ch);
                    rest = &after[semi + 1..];
                    continue;
                }
            }
        }
        out.push('&');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

fn url_encode(data: &str) -> String {
    let mut escaped = String::with_capacity(data.len() * 3);
    for b in data.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                escaped.push(b as char);
            }
            b' ' => escaped.push('+'),
            _ => escaped.push_str(&format!("%{:02X}", b)),
        }
    }
    escaped
}

fn url_decode(data: &str) -> String {
    let bytes = data.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn build_stdio_command(cfg: &McpServerConfig) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd.exe");
        c.arg("/C").arg(&cfg.command);
        for a in &cfg.args {
            c.arg(a);
        }
        c
    } else {
        let mut c = Command::new(&cfg.command);
        c.args(&cfg.args);
        c
    };

    for (k, v) in &cfg.env {
        cmd.env(k, v);
    }

    cmd
}

async fn read_next_json_line(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    timeout_dur: Duration,
) -> Result<String, String> {
    tokio::time::timeout(timeout_dur, async {
        while let Some(line) = lines.next_line().await.map_err(|e| format!("Stdout read error: {}", e))? {
            let trimmed = line.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                return Ok(trimmed.to_string());
            }
        }
        Err("Process closed stdout before responding with valid JSON-RPC".to_string())
    })
    .await
    .map_err(|_| "Timed out waiting for MCP server response".to_string())?
}

async fn test_mcp_sse(cfg: &McpServerConfig, start: Instant) -> McpTestResult {
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(e) => {
            return McpTestResult {
                success: false,
                server_id: cfg.id.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                tools: Vec::new(),
                error: Some(e.to_string()),
            };
        }
    };

    let test_url = format!("{}/tools/list", cfg.url.trim_end_matches('/'));
    match client.get(&test_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let val = resp.json::<Value>().await.unwrap_or_default();
            let mut tools = Vec::new();
            if let Some(arr) = val.get("tools").and_then(|t| t.as_array()) {
                for t in arr {
                    tools.push(McpToolDefinition {
                        name: t.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                        description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
                        input_schema: t.get("inputSchema").cloned().unwrap_or_else(|| serde_json::json!({ "type": "object" })),
                    });
                }
            }
            McpTestResult {
                success: true,
                server_id: cfg.id.clone(),
                latency_ms: start.elapsed().as_millis() as u64,
                tools,
                error: None,
            }
        }
        Ok(resp) => McpTestResult {
            success: false,
            server_id: cfg.id.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            tools: Vec::new(),
            error: Some(format!("HTTP error {}", resp.status())),
        },
        Err(e) => McpTestResult {
            success: false,
            server_id: cfg.id.clone(),
            latency_ms: start.elapsed().as_millis() as u64,
            tools: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

#[allow(dead_code)]
async fn execute_mcp_sse(cfg: &McpServerConfig, tool_name: &str, args: Value) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("{}/tools/call", cfg.url.trim_end_matches('/'));
    let payload = serde_json::json!({
        "name": tool_name,
        "arguments": args
    });

    let resp = client.post(&url).json(&payload).send().await
        .map_err(|e| format!("HTTP request to MCP server failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("MCP Server HTTP error {}", resp.status()));
    }

    let text = resp.text().await.map_err(|e| e.to_string())?;
    Ok(text)
}
