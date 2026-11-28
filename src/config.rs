use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    Allow,
    Ask,
    Deny,
}

impl Default for PermissionLevel {
    fn default() -> Self {
        PermissionLevel::Allow
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Safe,       // Ask before bash and workspace-write custom tools
    #[serde(alias = "auto")]
    Auto,       // Allow ReadOnly/WorkspaceWrite/Meta; Ask for ShellOrNetwork/HighRisk
    Autonomous, // Auto-approve safe and configured operations (hands-free)
    #[serde(alias = "yolo")]
    Yolo,       // Allow all tools unless overridden by rules
    ReadOnly,   // Disallow bash, write_file, edit_file, and mutating/network contracts
}

impl Default for PermissionMode {
    fn default() -> Self {
        PermissionMode::Safe
    }
}

impl PermissionMode {
    /// The stable snake_case name used by the API and the config file.
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionMode::Safe => "safe",
            PermissionMode::Auto => "auto",
            PermissionMode::Autonomous => "autonomous",
            PermissionMode::Yolo => "yolo",
            PermissionMode::ReadOnly => "read_only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub mode: PermissionMode,
    pub bash_policy: PermissionLevel,
    pub file_write_policy: PermissionLevel,
    pub file_read_policy: PermissionLevel,
    pub web_policy: PermissionLevel,
    #[serde(default)]
    pub rules: HashMap<String, PermissionLevel>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            mode: PermissionMode::Safe,
            bash_policy: PermissionLevel::Ask,
            file_write_policy: PermissionLevel::Allow,
            file_read_policy: PermissionLevel::Allow,
            web_policy: PermissionLevel::Allow,
            rules: HashMap::new(),
        }
    }
}

impl PermissionsConfig {
    pub fn for_tool(&self, tool: &str) -> PermissionLevel {
        if let Some(&override_level) = self.rules.get(tool) {
            return override_level;
        }
        match self.mode {
            PermissionMode::Yolo => PermissionLevel::Allow,
            PermissionMode::ReadOnly => {
                match tool {
                    "bash" | "write_file" | "edit_file" | "create_artifact" => PermissionLevel::Deny,
                    _ => PermissionLevel::Allow,
                }
            }
            PermissionMode::Auto => {
                match tool {
                    "bash" => {
                        if self.bash_policy == PermissionLevel::Deny {
                            PermissionLevel::Deny
                        } else {
                            PermissionLevel::Ask
                        }
                    }
                    "web_search" | "web_fetch" | "browser_ui" => {
                        if self.web_policy == PermissionLevel::Deny {
                            PermissionLevel::Deny
                        } else {
                            PermissionLevel::Ask
                        }
                    }
                    "write_file" | "edit_file" | "create_artifact" => {
                        if self.file_write_policy == PermissionLevel::Deny {
                            PermissionLevel::Deny
                        } else {
                            PermissionLevel::Allow
                        }
                    }
                    _ => PermissionLevel::Allow,
                }
            }
            PermissionMode::Autonomous => {
                match tool {
                    "bash" => if self.bash_policy == PermissionLevel::Deny { PermissionLevel::Deny } else { PermissionLevel::Allow },
                    "write_file" | "edit_file" | "create_artifact" => if self.file_write_policy == PermissionLevel::Deny { PermissionLevel::Deny } else { PermissionLevel::Allow },
                    _ => PermissionLevel::Allow,
                }
            }
            PermissionMode::Safe => {
                match tool {
                    "bash" => self.bash_policy,
                    "write_file" | "edit_file" | "create_artifact" => self.file_write_policy,
                    "read_file" | "list_dir" | "grep_search" | "sys_info" => self.file_read_policy,
                    "web_search" | "web_fetch" | "browser_ui" => self.web_policy,
                    _ => PermissionLevel::Allow,
                }
            }
        }
    }

    /// Resolve effective permission policy for a tool using its declared `ToolContract` permission tier.
    pub fn policy_for_contract(
        &self,
        tool_name: &str,
        default_perm: &crate::plugins::contract::ToolPermission,
    ) -> PermissionLevel {
        use crate::plugins::contract::ToolPermission;

        if let Some(&override_level) = self.rules.get(tool_name) {
            return override_level;
        }

        match self.mode {
            PermissionMode::Yolo => PermissionLevel::Allow,
            PermissionMode::Autonomous => match tool_name {
                "bash" if self.bash_policy == PermissionLevel::Deny => PermissionLevel::Deny,
                "write_file" | "edit_file" | "create_artifact"
                    if self.file_write_policy == PermissionLevel::Deny =>
                {
                    PermissionLevel::Deny
                }
                "read_file" | "list_dir" | "grep_search" | "sys_info"
                    if self.file_read_policy == PermissionLevel::Deny =>
                {
                    PermissionLevel::Deny
                }
                "web_search" | "web_fetch" | "browser_ui"
                    if self.web_policy == PermissionLevel::Deny =>
                {
                    PermissionLevel::Deny
                }
                _ => PermissionLevel::Allow,
            },
            PermissionMode::Auto => match default_perm {
                ToolPermission::ReadOnly
                | ToolPermission::WorkspaceWrite
                | ToolPermission::Meta => PermissionLevel::Allow,
                ToolPermission::ShellOrNetwork | ToolPermission::HighRisk => PermissionLevel::Ask,
            },
            PermissionMode::ReadOnly => match default_perm {
                ToolPermission::ReadOnly | ToolPermission::Meta => {
                    if matches!(
                        tool_name,
                        "read_file" | "list_dir" | "grep_search" | "sys_info"
                    ) && self.file_read_policy == PermissionLevel::Deny
                    {
                        PermissionLevel::Deny
                    } else {
                        PermissionLevel::Allow
                    }
                }
                ToolPermission::WorkspaceWrite
                | ToolPermission::ShellOrNetwork
                | ToolPermission::HighRisk => PermissionLevel::Deny,
            },
            PermissionMode::Safe => match tool_name {
                "bash" => self.bash_policy,
                "write_file" | "edit_file" | "create_artifact" => self.file_write_policy,
                "read_file" | "list_dir" | "grep_search" | "sys_info" => self.file_read_policy,
                "web_search" | "web_fetch" | "browser_ui" => self.web_policy,
                _ => match default_perm {
                    ToolPermission::ReadOnly | ToolPermission::Meta => PermissionLevel::Allow,
                    ToolPermission::WorkspaceWrite
                    | ToolPermission::ShellOrNetwork
                    | ToolPermission::HighRisk => PermissionLevel::Ask,
                },
            },
        }
    }
}

fn default_reasoning_effort() -> String {
    "high".to_string()
}

fn default_display_verbosity() -> String {
    "compact".to_string()
}

pub fn normalize_display_verbosity(val: &str) -> String {
    match val.trim().to_ascii_lowercase().as_str() {
        "compact" | "balanced" | "verbose" => val.trim().to_ascii_lowercase(),
        _ => default_display_verbosity(),
    }
}

fn default_send_mode() -> String {
    "queue".to_string()
}

pub fn normalize_send_mode(val: &str) -> String {
    match val.trim().to_ascii_lowercase().as_str() {
        "steer" => "steer".to_string(),
        _ => default_send_mode(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub default_model: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerNodeConfig {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_token: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
}

pub fn default_servers() -> Vec<ServerNodeConfig> {
    vec![ServerNodeConfig {
        id: "local".to_string(),
        name: "Local Built-in Server".to_string(),
        url: "local://in-process".to_string(),
        api_token: String::new(),
        enabled: true,
        is_default: true,
    }]
}

pub fn default_active_server_id() -> String {
    "local".to_string()
}

pub const CONVERSATIONS_PROJECT_ID: &str = "__conversations__";

pub fn is_knowledge_only_project(project_id: &str) -> bool {
    let trimmed = project_id.trim();
    trimmed.is_empty() || trimmed == CONVERSATIONS_PROJECT_ID
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SubagentProfileConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub provider_id: String,
    pub model: String,
    pub thinking_effort: String,
    pub server_id: String,
    pub read_only: bool,
}

impl Default for SubagentProfileConfig {
    fn default() -> Self {
        Self {
            id: "worker".to_string(),
            name: "General Worker".to_string(),
            description: "General subagent task execution".to_string(),
            provider_id: String::new(),
            model: String::new(),
            thinking_effort: "medium".to_string(),
            server_id: "local".to_string(),
            read_only: false,
        }
    }
}

pub fn default_subagent_mode() -> String {
    "single".to_string()
}

pub fn default_subagent_profiles() -> Vec<SubagentProfileConfig> {
    vec![
        SubagentProfileConfig {
            id: "research".to_string(),
            name: "Research".to_string(),
            description: "Web & knowledge research, reading/synthesizing facts and references".to_string(),
            provider_id: String::new(),
            model: String::new(),
            thinking_effort: "medium".to_string(),
            server_id: "local".to_string(),
            read_only: true,
        },
        SubagentProfileConfig {
            id: "coding".to_string(),
            name: "Coding".to_string(),
            description: "Code implementation, refactoring, and build/test verification".to_string(),
            provider_id: String::new(),
            model: String::new(),
            thinking_effort: "high".to_string(),
            server_id: "local".to_string(),
            read_only: false,
        },
        SubagentProfileConfig {
            id: "reviewer".to_string(),
            name: "Reviewer".to_string(),
            description: "Independent read-only verification, bug hunting, and diff review".to_string(),
            provider_id: String::new(),
            model: String::new(),
            thinking_effort: "high".to_string(),
            server_id: "local".to_string(),
            read_only: true,
        },
    ]
}

pub fn default_orchestrator_system_prompt() -> String {
    "You are operating in **Orchestrator Mode**. Your primary responsibility is to decompose the user's goal into clear, well-scoped subtasks, delegate execution to specialized subagents, coordinate their results, and verify correctness before answering.\n\
    Guidelines:\n\
    1. For multi-file, research+implementation, or parallelizable tasks, call `swarm_board` (`set_title`, `create_job`, `create_specialist`) and spawn workers via `invoke_subagent`.\n\
    2. When multiple execution servers are configured, route specialized or independent subagents across servers using `server_id` when helpful, and include clear file ownership and acceptance criteria in each subagent prompt.\n\
    3. Spawn an independent verifier/reviewer subagent before finalizing code changes.\n\
    4. Use `swarm_board{action:\"wait\", since:<seq>}` to sleep zero-CPU while background subagents run rather than busy-polling.\n\
    5. Synthesize verified outcomes concisely in your final answer without inventing unverified claims."
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchestratorConfig {
    pub enabled: bool,
    pub model: String,
    pub provider_id: String,
    pub thinking_effort: String,
    pub subagent_model: String,
    pub subagent_provider_id: String,
    pub subagent_thinking_effort: String,
    pub subagent_server_id: String,
    #[serde(default = "default_subagent_mode")]
    pub subagent_mode: String, // "single" | "multi"
    #[serde(default = "default_subagent_profiles")]
    pub subagent_profiles: Vec<SubagentProfileConfig>,
    pub system_prompt: String,
}

/// Canonical ordering of reasoning-effort levels, least to most intensive.
pub const REASONING_LEVELS: &[&str] = &["none", "minimal", "low", "medium", "high"];

/// Normalize an effort label to a canonical level, or `None` when unknown.
pub fn normalize_reasoning_level(raw: &str) -> Option<&'static str> {
    let v = raw.trim().to_ascii_lowercase().replace('-', "_");
    let v = match v.as_str() {
        "" => return None,
        "off" | "disabled" => "none",
        "min" | "minimum" => "minimal",
        "med" | "mid" | "middle" | "normal" | "default" => "medium",
        "max" | "maximum" | "extra_high" | "xhigh" | "ultra" => "high",
        other => other,
    };
    REASONING_LEVELS.iter().copied().find(|l| *l == v)
}

/// Step one level down (`high` -> `medium` -> `low` -> `minimal` -> `none`).
///
/// This is what a subagent inherits from its orchestrator: the parent reasons at
/// full effort while workers run a notch cheaper. An unknown level is returned
/// unchanged so a typo never silently disables reasoning.
pub fn step_down_effort(raw: &str) -> String {
    match normalize_reasoning_level(raw) {
        Some(level) => {
            let idx = REASONING_LEVELS
                .iter()
                .position(|l| *l == level)
                .unwrap_or(REASONING_LEVELS.len() - 1);
            REASONING_LEVELS[idx.saturating_sub(1)].to_string()
        }
        None => raw.trim().to_string(),
    }
}

/// Sentinel values that mean "follow the orchestrator" rather than a fixed value.
pub fn is_inherit_sentinel(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().replace('-', "_").as_str(),
        "inherit" | "follow" | "orchestrator" | "same" | "parent"
    )
}

/// Sentinel values that mean "one level below the orchestrator".
pub fn is_step_down_sentinel(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().replace('-', "_").as_str(),
        "step_down" | "one_lower" | "lower" | "downgrade" | "reduce"
    )
}

/// Resolve the effective reasoning effort for a subagent.
///
/// * `inherit`  -> the orchestrator's effort unchanged
/// * `step_down` -> one level below the orchestrator's effort
/// * anything else -> that fixed level (empty falls back to `default`)
pub fn resolve_subagent_effort(configured: &str, orchestrator_effort: &str, default: &str) -> String {
    let c = configured.trim();
    if is_step_down_sentinel(c) {
        return step_down_effort(orchestrator_effort);
    }
    if is_inherit_sentinel(c) {
        let o = orchestrator_effort.trim();
        return if o.is_empty() { default.to_string() } else { o.to_string() };
    }
    if c.is_empty() {
        return default.to_string();
    }
    c.to_string()
}
impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            provider_id: String::new(),
            thinking_effort: "high".to_string(),
            subagent_model: String::new(),
            subagent_provider_id: String::new(),
            // Default: workers reason one level below the orchestrator.
            subagent_thinking_effort: "step_down".to_string(),
            subagent_server_id: String::new(),
            subagent_mode: default_subagent_mode(),
            subagent_profiles: default_subagent_profiles(),
            system_prompt: default_orchestrator_system_prompt(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    #[serde(default = "default_search_provider")]
    pub active_provider: String, // "tinyfish", "duckduckgo", "tavily", "brave", "searxng"
    #[serde(default = "default_tinyfish_key")]
    pub tinyfish_api_key: String,
    #[serde(default)]
    pub tavily_api_key: String,
    #[serde(default)]
    pub brave_api_key: String,
    #[serde(default)]
    pub searxng_url: String,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_search_provider() -> String {
    "tinyfish".to_string()
}

fn default_tinyfish_key() -> String {
    String::new() // no bundled key: set TINYFISH_API_KEY or configure search
}

fn default_max_results() -> usize {
    5
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            active_provider: default_search_provider(),
            tinyfish_api_key: default_tinyfish_key(),
            tavily_api_key: String::new(),
            brave_api_key: String::new(),
            searxng_url: String::new(),
            max_results: 5,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_skill_router_min_confidence() -> f32 {
    0.35
}

fn default_skill_router_max_skills() -> u32 {
    2
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

fn default_max_tool_retries() -> usize {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub max_context_tokens: usize,
    pub auto_compact_threshold_pct: u8,
    pub preserve_recent_messages: usize,
    pub prune_tool_outputs_over_bytes: usize,
    pub summarize_compacted_turns: bool,
    pub enable_working_memory: bool,
    #[serde(default = "default_max_tool_retries")]
    pub max_tool_retries: usize,
    #[serde(default = "default_true")]
    pub auto_detect_context: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 128_000,
            auto_compact_threshold_pct: 78,
            preserve_recent_messages: 8,
            prune_tool_outputs_over_bytes: 2048,
            summarize_compacted_turns: true,
            enable_working_memory: true,
            max_tool_retries: default_max_tool_retries(),
            auto_detect_context: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_mcp_transport")]
    pub transport: String, // "stdio", "sse", or "builtin"
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_mcp_servers() -> Vec<McpServerConfig> {
    let mut google_env = std::collections::HashMap::new();
    google_env.insert("GEMINI_API_KEY".to_string(), "".to_string());
    google_env.insert("GOOGLE_AI_MODEL".to_string(), "gemini-2.5-flash".to_string());

    vec![
        McpServerConfig {
            id: "google-ai-search".to_string(),
            name: "Google AI Search (MCP)".to_string(),
            transport: "builtin".to_string(),
            command: "builtin://google-ai-search".to_string(),
            args: vec![],
            env: google_env,
            url: "https://www.google.com/search".to_string(),
            enabled: true,
        },
        McpServerConfig {
            id: "memory".to_string(),
            name: "Knowledge Graph Memory (MCP)".to_string(),
            transport: "stdio".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@modelcontextprotocol/server-memory".to_string()],
            env: std::collections::HashMap::new(),
            url: "".to_string(),
            enabled: false,
        },
        McpServerConfig {
            id: "filesystem".to_string(),
            name: "Filesystem Explorer (MCP)".to_string(),
            transport: "stdio".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@modelcontextprotocol/server-filesystem".to_string(), ".".to_string()],
            env: std::collections::HashMap::new(),
            url: "".to_string(),
            enabled: false,
        },
        McpServerConfig {
            id: "sqlite".to_string(),
            name: "SQLite Database (MCP)".to_string(),
            transport: "stdio".to_string(),
            command: "uvx".to_string(),
            args: vec!["mcp-server-sqlite".to_string(), "--db-path".to_string(), "./harness.db".to_string()],
            env: std::collections::HashMap::new(),
            url: "".to_string(),
            enabled: false,
        },
    ]
}

/// No providers are configured by default: the harness ships with an empty list
/// so the UI shows "no provider configured" until the user adds one. Presets
/// (NVIDIA NIM, DeepSeek, OpenRouter, Groq, Ollama, vLLM) are offered in the UI
/// "Add Provider" form.
/// Preset providers that older builds shipped by default.
const SHIPPED_DEFAULT_PROVIDERS: &[(&str, &str)] = &[
    ("nvidia", "https://integrate.api.nvidia.com/v1"),
    ("deepseek", "https://api.deepseek.com/v1"),
    ("openrouter", "https://openrouter.ai/api/v1"),
    ("groq", "https://api.groq.com/openai/v1"),
    ("ollama", "http://localhost:11434/v1"),
    ("vllm", "http://localhost:8000/v1"),
];

/// True when `p` still looks exactly like a shipped preset (same id, same base,
/// and no real credential). Used to purge leftovers from older configs once.
pub fn is_shipped_default_provider(p: &CustomProvider) -> bool {
    let base = p.api_base.trim().trim_end_matches('/').to_ascii_lowercase();
    let has_real_key = !p.api_key.trim().is_empty() && p.api_key.trim() != "ollama";
    if has_real_key {
        return false;
    }
    SHIPPED_DEFAULT_PROVIDERS
        .iter()
        .any(|(id, preset_base)| p.id == *id && base == *preset_base)
}
fn default_providers() -> Vec<CustomProvider> {
    Vec::new()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectDefaults {
    pub provider_id: String,
    pub model: String,
    pub reasoning_effort: String,
    pub server_id: String,
    pub orchestrator_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Primary workspace folder (kept for compatibility; mirrors `folders[0]`).
    pub path: PathBuf,
    pub icon: String,
    pub created_at: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub defaults: ProjectDefaults,
    /// Additional workspace folders belonging to this project. A project may span
    /// several directories (e.g. a service and its docs); the agent can read/write
    /// in any of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_paths: Vec<PathBuf>,
}

impl Project {
    /// Every folder this project grants access to, primary first.
    pub fn all_paths(&self) -> Vec<PathBuf> {
        let mut out = vec![self.path.clone()];
        for p in &self.extra_paths {
            if !out.iter().any(|existing| existing == p) {
                out.push(p.clone());
            }
        }
        out
    }

    /// Normalize `path` to the primary folder and keep `extra_paths` deduped.
    pub fn normalize_folders(&mut self) {
        let mut seen: Vec<PathBuf> = vec![self.path.clone()];
        self.extra_paths.retain(|p| {
            if p.as_os_str().is_empty() || seen.iter().any(|s| s == p) {
                false
            } else {
                seen.push(p.clone());
                true
            }
        });
    }
}

fn default_max_active_subagents() -> usize {
    32
}

fn default_projects() -> Vec<Project> {
    // The default project is a `Project` folder beside the harness executable, so a
    // fresh install never points at whatever directory the binary was launched from.
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let project_dir = base.join("Project");
    let _ = std::fs::create_dir_all(&project_dir);
    vec![
        Project {
            id: "project".to_string(),
            name: "Project".to_string(),
            path: project_dir,
            icon: "\u{1F4C1}".to_string(),
            created_at: chrono::Local::now().to_rfc3339(),
            is_active: true,
            defaults: ProjectDefaults::default(),
            extra_paths: Vec::new(),
        }
    ]
}

fn default_active_project_id() -> String {
    "project".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub content: String,
    pub path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    pub host: String,
    pub port: u16,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String, // "low", "medium", "high", "none"
    #[serde(default = "default_display_verbosity")]
    pub display_verbosity: String, // "compact", "balanced", "verbose"
    #[serde(default = "default_send_mode")]
    pub send_mode: String, // "queue", "steer"
    #[serde(default)]
    pub voice_activity_detection: bool,
    pub tinyfish_api_key: String,
    pub provider: String,
    pub temperature: f32,
    #[serde(default)]
    pub max_iterations: usize,
    pub max_tool_output_bytes: usize,
    pub tool_timeout_secs: u64,
    pub sandbox_root: PathBuf,
    pub mock_mode: bool,
    /// Opt in to the privileged `bash` tool. It is **not** contained by the
    /// sandbox (a shell can read any absolute path the harness process can), so
    /// it is unregistered by default. Enable only when the agent is trusted with
    /// the user's full machine rights. `HARNESS_ALLOW_PRIVILEGED_SHELL=1` also
    /// turns it on at startup.
    #[serde(default)]
    pub allow_privileged_shell: bool,
    pub permissions: PermissionsConfig,
    #[serde(default = "default_providers")]
    pub custom_providers: Vec<CustomProvider>,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default = "default_mcp_servers")]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default = "default_projects")]
    pub projects: Vec<Project>,
    #[serde(default = "default_active_project_id")]
    pub active_project_id: String,
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    /// Automatically route each user message to matching skills and inject the
    /// best one's instructions as a system directive (see `crate::skill_router`).
    /// When false, skills are only activated by an explicit `/skill` command.
    #[serde(default = "default_true")]
    pub auto_skill_router: bool,
    /// Minimum router confidence (0.0..=1.0) required before a skill is injected.
    #[serde(default = "default_skill_router_min_confidence")]
    pub skill_router_min_confidence: f32,
    /// Maximum number of skills injected for one message (multi-intent cap).
    #[serde(default = "default_skill_router_max_skills")]
    pub skill_router_max_skills: u32,
    #[serde(default)]
    pub skills_dirs: Vec<String>,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    #[serde(default = "default_servers")]
    pub servers: Vec<ServerNodeConfig>,
    #[serde(default = "default_active_server_id")]
    pub active_server_id: String,
    #[serde(default)]
    pub server_identity: String,
    #[serde(default)]
    pub db_path: Option<PathBuf>,
    #[serde(default)]
    pub knowledge_only: bool,
    /// Optional bearer token required on `/api/*` (or set `HARNESS_AUTH_TOKEN`).
    #[serde(default)]
    pub auth_token: String,
    /// GitHub repository (`owner/repo`) used for auto-update checks.
    #[serde(default = "default_update_repo")]
    pub update_repo: String,
    /// Check GitHub for a newer release at startup.
    #[serde(default = "default_true")]
    pub auto_update_check: bool,
    /// Automatically download *and install* a newer release in the background
    /// (progress is shown in the UI; the app restarts itself when done).
    #[serde(default = "default_true")]
    pub auto_update_apply: bool,
    /// Pin the auto-updater to one exact release (e.g. `1.1.3` or `v1.1.3`).
    /// When set, the updater installs *that* version even if it is older than the
    /// running build, and never jumps to `latest`. Empty means "follow latest".
    /// `HARNESS_UPDATE_PIN` seeds this at startup.
    #[serde(default)]
    pub update_pin_version: String,
    /// First-class automations: scheduled agent runs, standalone or project-bound.
    #[serde(default)]
    pub automations: Vec<crate::automation::Automation>,
    /// Global ceiling on concurrently RUNNING subagents, across every session.
    ///
    /// A batch cap (8) bounds one `invoke_subagent` call and `max_depth` bounds
    /// nesting, but the product of the two is a fan-out: with depth 2 a coordinator
    /// can spawn 8 workers, each of which can spawn 8 more, and nothing anywhere
    /// looked at the *global* count - so a single ambitious prompt could reach 584
    /// concurrent agent loops on one machine. This is the bound on that.
    #[serde(default = "default_max_active_subagents")]
    pub max_active_subagents: usize,
    /// Extra filesystem roots the user explicitly approved for tools to access
    /// outside the project workspace.
    #[serde(default)]
    pub extra_access_roots: Vec<PathBuf>,
    /// Server-owned allowlist of directory prefixes a *remote execution server*
    /// may grant to a client-dispatched run. A client project that lives under
    /// one of these prefixes (e.g. a shared projects directory) is accepted;
    /// anything else is refused with 409 instead of being silently dropped.
    /// Empty means "only this server's own sandbox and extra_access_roots".
    #[serde(default)]
    pub remote_root_allowlist: Vec<PathBuf>,
    /// False until the first-run onboarding has been completed.
    #[serde(default)]
    pub onboarded: bool,
    /// Version the auto-updater last attempted to install (loop guard: never
    /// auto-install the same release twice in a row).
    #[serde(default)]
    pub last_auto_update_target: String,
    /// Monotonic revision of the config row, bumped by every endpoint that
    /// mutates it (`activate_provider`, `save_mcp_server`, `save_server`,
    /// `set_active_server`, `save_automation`, `save_skill`, the path-access
    /// grant, ...). `POST /api/config` submits the *whole* config as a snapshot,
    /// so without this counter a stale snapshot silently reverted whatever those
    /// endpoints had just changed. A payload carrying an older revision than the
    /// stored one is rejected with `409 Conflict` instead.
    ///
    /// Server-owned: `merge_config_update` always keeps the stored value, so a
    /// client can never rewind it.
    #[serde(default)]
    pub config_revision: u64,
}


pub fn default_update_repo() -> String {
    crate::updater::DEFAULT_REPO.to_string()
}

/// Parse an opt-in environment flag. Only explicit truthy values enable a
/// privileged capability; an empty or unset variable never does.
fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// True when the harness can actually reach a model with the stored config.
///
/// The wizard (and the "No model provider configured" guard) must look at every
/// way a provider can be configured - not only saved custom providers. A harness
/// that was pointed at `https://integrate.api.nvidia.com/v1` with a top-level
/// `api_key` + `model` works fine, but the old check reported "no provider".
pub fn has_usable_provider(conf: &HarnessConfig) -> bool {
    use crate::models::google_search_client::is_google_search_model;
    if conf.mock_mode {
        return true;
    }
    let base = conf.api_base.trim();
    let local_base = base.contains("localhost") || base.contains("127.0.0.1");
    // A saved custom provider with a key or a local endpoint.
    let custom = conf
        .custom_providers
        .iter()
        .any(|p| !p.api_key.trim().is_empty() || p.api_base.contains("localhost"));
    if custom {
        return true;
    }
    // The top-level (single) provider: either an explicit key, a local endpoint
    // that needs no key, or a model that runs through a built-in integration.
    if !conf.api_key.trim().is_empty() {
        return true;
    }
    if local_base && !conf.model.trim().is_empty() {
        return true;
    }
    if is_google_search_model(&conf.model) {
        return true;
    }
    false
}

impl Default for HarnessConfig {
    fn default() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let default_tinyfish_key = ""; // no bundled key

        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            api_base: std::env::var("MODEL_API_BASE").unwrap_or_default(),
            api_key: std::env::var("MODEL_API_KEY").unwrap_or_default(),
            model: std::env::var("MODEL_NAME").unwrap_or_default(),
            reasoning_effort: "high".to_string(),
            display_verbosity: default_display_verbosity(),
            send_mode: default_send_mode(),
            voice_activity_detection: false,
            tinyfish_api_key: std::env::var("TINYFISH_API_KEY")
                .unwrap_or_else(|_| default_tinyfish_key.to_string()),
            provider: String::new(),
            temperature: 0.6,
            max_iterations: 0,
            max_tool_output_bytes: 16 * 1024,
            tool_timeout_secs: 30,
            sandbox_root: current_dir.clone(),
            mock_mode: false,
            allow_privileged_shell: env_truthy("HARNESS_ALLOW_PRIVILEGED_SHELL"),
            permissions: PermissionsConfig::default(),
            custom_providers: default_providers(),
            search: SearchConfig::default(),
            mcp_servers: default_mcp_servers(),
            projects: default_projects(),
            active_project_id: default_active_project_id(),
            disabled_skills: Vec::new(),
            auto_skill_router: true,
            skill_router_min_confidence: default_skill_router_min_confidence(),
            skill_router_max_skills: default_skill_router_max_skills(),
            skills_dirs: Vec::new(),
            memory: MemoryConfig::default(),
            orchestrator: OrchestratorConfig::default(),
            servers: default_servers(),
            active_server_id: default_active_server_id(),
            server_identity: String::new(),
            db_path: None,
            knowledge_only: false,
            auth_token: std::env::var("HARNESS_AUTH_TOKEN").unwrap_or_default(),
            update_repo: std::env::var("HARNESS_UPDATE_REPO")
                .unwrap_or_else(|_| default_update_repo()),
            auto_update_check: true,
            auto_update_apply: true,
            update_pin_version: std::env::var("HARNESS_UPDATE_PIN").unwrap_or_default(),
            automations: Vec::new(),
            max_active_subagents: default_max_active_subagents(),
            extra_access_roots: Vec::new(),
            remote_root_allowlist: Vec::new(),
            last_auto_update_target: String::new(),
            onboarded: false,
            config_revision: 0,
        }
    }
}





impl HarnessConfig {
    /// Fields carrying operator-provided CLI/env values that must take precedence over a
    /// DB-persisted config when the operator explicitly set them at startup. A field is
    /// considered operator-provided when it differs from `HarnessConfig::default()`
    /// (defaults read the same env vars in-process, so unset env values compare equal).
    pub const OPERATOR_OVERRIDE_FIELDS: &'static [&'static str] =
        &["sandbox_root", "model", "api_key", "api_base", "provider"];

    /// Merge operator-provided (CLI/env) values from `incoming` over the DB-persisted `base`.
    /// `defaults` is a `HarnessConfig::default()` snapshot used to detect which `incoming`
    /// fields were explicitly set (they differ from the default) versus left at their default.
    pub fn apply_operator_overrides(&mut self, incoming: &HarnessConfig, defaults: &HarnessConfig) {
        if incoming.sandbox_root != defaults.sandbox_root {
            self.sandbox_root = incoming.sandbox_root.clone();
        }
        if incoming.model != defaults.model {
            self.model = incoming.model.clone();
        }
        if incoming.api_key != defaults.api_key {
            self.api_key = incoming.api_key.clone();
        }
        if incoming.api_base != defaults.api_base {
            self.api_base = incoming.api_base.clone();
        }
        if incoming.provider != defaults.provider {
            self.provider = incoming.provider.clone();
        }
    }

    /// Resolve `(provider_id, api_base, api_key)` for a model and optional provider hint so multiple
    /// providers can be used simultaneously across main turns, orchestrator, and subagents.
    pub fn resolve_provider_credentials(
        &self,
        model_id: &str,
        provider_hint: Option<&str>,
    ) -> (String, String, String) {
        if let Some(hint) = provider_hint.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(p) = self.custom_providers.iter().find(|p| {
                p.enabled
                    && (p.id.eq_ignore_ascii_case(hint) || p.name.eq_ignore_ascii_case(hint))
            }) {
                return (p.id.clone(), p.api_base.clone(), p.api_key.clone());
            }
        }

        let trimmed_model = model_id.trim();
        if !trimmed_model.is_empty() {
            if let Some(p) = self.custom_providers.iter().find(|p| {
                p.enabled
                    && !p.api_base.trim().is_empty()
                    && (!p.api_key.trim().is_empty() || p.api_base.contains("localhost") || p.api_base.contains("127.0.0.1"))
                    && p.default_model.eq_ignore_ascii_case(trimmed_model)
            }) {
                return (p.id.clone(), p.api_base.clone(), p.api_key.clone());
            }
        }

        (
            self.provider.clone(),
            self.api_base.clone(),
            self.api_key.clone(),
        )
    }

    /// Return a cloned config configured for a specific `(model_id, provider_hint, effort)`.
    pub fn with_model_override(
        &self,
        model_id: &str,
        provider_hint: Option<&str>,
        reasoning_effort: Option<&str>,
    ) -> Self {
        let mut next = self.clone();
        let trimmed_model = model_id.trim();
        if !trimmed_model.is_empty() {
            next.model = trimmed_model.to_string();
        }
        let (pid, base, key) = self.resolve_provider_credentials(&next.model, provider_hint);
        next.provider = pid;
        next.api_base = base;
        next.api_key = key;
        if let Some(eff) = reasoning_effort.map(str::trim).filter(|s| !s.is_empty()) {
            next.reasoning_effort = eff.to_string();
        }
        next
    }
    /// Discover skill markdown definitions under `.harness/skills`, `skills/`,
    /// and any custom `skills_dirs` relative to the given workspace root.
    /// Built-in default skills are always available; see [`BUILTIN_SKILLS`].
    pub fn discover_skills_in(&self, workspace_root: &Path) -> Vec<SkillDefinition> {
        self.discover_skills_in_inner(workspace_root)
    }

    #[doc(hidden)]
    pub fn __builtin_skill_count() -> usize {
        crate::builtin_skills::BUILTIN_SKILLS.len()
    }

    fn discover_skills_in_inner(&self, workspace_root: &Path) -> Vec<SkillDefinition> {
        let mut dirs: Vec<PathBuf> = vec![
            workspace_root.join(".harness").join("skills"),
            workspace_root.join("skills"),
        ];
        for extra in &self.skills_dirs {
            let p = Path::new(extra);
            if p.is_absolute() {
                dirs.push(p.to_path_buf());
            } else {
                dirs.push(workspace_root.join(p));
            }
        }

        let disabled: HashSet<String> = self
            .disabled_skills
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        let mut seen_names: HashSet<String> = HashSet::new();
        let mut skills = Vec::new();

        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();

            for path in paths {
                let candidate = if path.is_dir() {
                    let skill_md = path.join("SKILL.md");
                    if skill_md.is_file() {
                        Some((
                            path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("skill")
                                .to_string(),
                            skill_md,
                        ))
                    } else {
                        None
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    Some((
                        path.file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or("skill")
                            .to_string(),
                        path.clone(),
                    ))
                } else {
                    None
                };

                let Some((fallback_name, file_path)) = candidate else {
                    continue;
                };
                let Ok(raw) = std::fs::read_to_string(&file_path) else {
                    continue;
                };

                let (name, description, body) = parse_skill_markdown(&raw, &fallback_name);
                let norm = name.to_lowercase();
                if !seen_names.insert(norm.clone()) {
                    continue;
                }

                let rel_path = file_path
                    .strip_prefix(workspace_root)
                    .unwrap_or(&file_path)
                    .to_string_lossy()
                    .replace('\\', "/");

                skills.push(SkillDefinition {
                    name,
                    description,
                    content: body,
                    path: rel_path,
                    enabled: !disabled.contains(&norm),
                });
            }
        }

        // Append built-in default skills only for names not provided by the
        // workspace (scan-order wins); a workspace skill of the same name
        // overrides the built-in. Built-ins can be disabled by name.
        for builtin in crate::builtin_skills::BUILTIN_SKILLS {
            let norm = builtin.name.to_lowercase();
            if seen_names.contains(&norm) {
                continue;
            }
            skills.push(SkillDefinition {
                name: builtin.name.to_string(),
                description: builtin.description.to_string(),
                content: builtin.content.to_string(),
                path: format!("builtin://{}", builtin.name),
                enabled: !disabled.contains(&norm),
            });
        }

        skills
    }

    #[allow(dead_code)]
    pub fn discover_skills(&self) -> Vec<SkillDefinition> {
        self.discover_skills_in(&self.sandbox_root)
    }

    pub fn format_skills_prompt_snippet(&self, workspace_root: &Path) -> String {
        let enabled: Vec<SkillDefinition> = self
            .discover_skills_in(workspace_root)
            .into_iter()
            .filter(|s| s.enabled)
            .collect();
        if enabled.is_empty() {
            return String::new();
        }
        let mut out = String::from("\nAvailable Workspace Skills:\n");
        for s in enabled.iter().take(16) {
            out.push_str(&format!("- {} ({}): {}\n", s.name, s.path, s.description));
        }
        out
    }

    /// Effective knobs for the automatic skill router. Values are clamped so a
    /// hand-edited config can never disable injection unintentionally or flood
    /// the context with skills.
    pub fn skill_router_settings(&self) -> crate::skill_router::RouterSettings {
        let settings = crate::skill_router::RouterSettings {
            min_confidence: self.skill_router_min_confidence,
            max_skills: self.skill_router_max_skills as usize,
            min_margin: crate::skill_router::DEFAULT_MIN_MARGIN,
        };
        let clamped = settings.clamped();
        crate::skill_router::RouterSettings {
            min_confidence: clamped.min_confidence,
            max_skills: clamped.max_skills,
            min_margin: clamped.min_margin,
        }
    }
}

fn parse_skill_markdown(raw: &str, fallback_name: &str) -> (String, String, String) {
    let normalized = raw.replace("\r\n", "\n");
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut body = normalized.clone();

    if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some(end_idx) = rest.find("\n---") {
            let frontmatter = &rest[..end_idx];
            body = rest[end_idx + 4..].trim_start_matches('\n').to_string();
            for line in frontmatter.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim().to_lowercase();
                    let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                    if key == "name" && !val.is_empty() {
                        name = val;
                    } else if key == "description" && !val.is_empty() {
                        description = val;
                    }
                }
            }
        }
    }

    if description.is_empty() {
        for line in body.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            description = t.chars().take(160).collect();
            break;
        }
    }
    if description.is_empty() {
        description = format!("Workspace skill '{}'", name);
    }

    (name, description, body)
}

#[cfg(test)]
mod operator_override_tests {
    use super::*;

    #[test]
    fn operator_fields_win_over_persisted_config() {
        let defaults = HarnessConfig::default();
        let mut db_config = defaults.clone();
        db_config.sandbox_root = PathBuf::from("/db/workspace");
        db_config.model = "db-model".to_string();
        db_config.api_key = "db-key".to_string();
        db_config.api_base = "https://db.example/v1".to_string();
        db_config.provider = "db-provider".to_string();
        db_config.port = 9999;

        let mut incoming = defaults.clone();
        incoming.sandbox_root = PathBuf::from("/cli/workspace");
        incoming.model = "cli-model".to_string();
        incoming.api_key = "cli-key".to_string();
        incoming.api_base = "https://cli.example/v1".to_string();
        incoming.provider = "cli-provider".to_string();

        db_config.apply_operator_overrides(&incoming, &defaults);

        assert_eq!(db_config.sandbox_root, PathBuf::from("/cli/workspace"));
        assert_eq!(db_config.model, "cli-model");
        assert_eq!(db_config.api_key, "cli-key");
        assert_eq!(db_config.api_base, "https://cli.example/v1");
        assert_eq!(db_config.provider, "cli-provider");
        // Untouched fields must keep the DB value.
        assert_eq!(db_config.port, 9999);
    }

    #[test]
    fn default_incoming_values_do_not_clobber_persisted_config() {
        let defaults = HarnessConfig::default();
        let mut db_config = defaults.clone();
        db_config.sandbox_root = PathBuf::from("/db/workspace");
        db_config.model = "db-model".to_string();
        db_config.api_key = "db-key".to_string();
        db_config.api_base = "https://db.example/v1".to_string();
        db_config.provider = "db-provider".to_string();

        db_config.apply_operator_overrides(&defaults, &defaults);

        assert_eq!(db_config.sandbox_root, PathBuf::from("/db/workspace"));
        assert_eq!(db_config.model, "db-model");
        assert_eq!(db_config.api_key, "db-key");
        assert_eq!(db_config.api_base, "https://db.example/v1");
        assert_eq!(db_config.provider, "db-provider");
    }
}


#[cfg(test)]
mod skill_router_config_tests {
    use super::*;

    #[test]
    fn skill_router_defaults_are_enabled_and_conservative() {
        let cfg = HarnessConfig::default();
        assert!(cfg.auto_skill_router, "auto routing should ship enabled");
        assert_eq!(cfg.skill_router_min_confidence, 0.35);
        assert_eq!(cfg.skill_router_max_skills, 2);

        let settings = cfg.skill_router_settings();
        assert_eq!(
            settings.min_confidence,
            crate::skill_router::DEFAULT_MIN_CONFIDENCE
        );
        assert_eq!(settings.max_skills, crate::skill_router::DEFAULT_MAX_SKILLS);
    }

    #[test]
    fn skill_router_fields_deserialize_from_partial_json() {
        // Configs saved before these fields existed must keep the defaults.
        let json = r#"{
            "host": "127.0.0.1",
            "port": 8080,
            "api_base": "",
            "api_key": "",
            "model": "",
            "tinyfish_api_key": "",
            "provider": "",
            "temperature": 0.6,
            "max_tool_output_bytes": 16384,
            "tool_timeout_secs": 30,
            "sandbox_root": ".",
            "mock_mode": false,
            "permissions": {
                "mode": "safe",
                "bash_policy": "ask",
                "file_write_policy": "allow",
                "file_read_policy": "allow",
                "web_policy": "allow"
            }
        }"#;
        let cfg: HarnessConfig = serde_json::from_str(json).expect("legacy config parses");
        assert!(cfg.auto_skill_router);
        assert_eq!(cfg.skill_router_min_confidence, 0.35);
        assert_eq!(cfg.skill_router_max_skills, 2);
    }

    #[test]
    fn skill_router_disabling_flag_round_trips() {
        // Round-trip a full config snapshot, the way `POST /api/config` does.
        let mut value = serde_json::to_value(HarnessConfig::default()).expect("serializes");
        let obj = value.as_object_mut().expect("object");
        obj.insert("auto_skill_router".to_string(), serde_json::json!(false));
        obj.insert(
            "skill_router_min_confidence".to_string(),
            serde_json::json!(0.5),
        );
        obj.insert("skill_router_max_skills".to_string(), serde_json::json!(3));
        let cfg: HarnessConfig = serde_json::from_value(value).expect("config parses");
        assert!(!cfg.auto_skill_router);
        let settings = cfg.skill_router_settings();
        assert_eq!(settings.min_confidence, 0.5);
        assert_eq!(settings.max_skills, 3);
        let serialized = serde_json::to_value(&cfg).expect("serializes");
        assert_eq!(serialized["auto_skill_router"], serde_json::json!(false));
        assert_eq!(
            serialized["skill_router_min_confidence"],
            serde_json::json!(0.5)
        );
        assert_eq!(
            serialized["skill_router_max_skills"],
            serde_json::json!(3)
        );
    }

    #[test]
    fn router_settings_clamp_hostile_values() {
        let mut cfg = HarnessConfig::default();
        cfg.skill_router_min_confidence = f32::NAN;
        cfg.skill_router_max_skills = 99;
        let settings = cfg.skill_router_settings();
        assert_eq!(
            settings.min_confidence,
            crate::skill_router::DEFAULT_MIN_CONFIDENCE
        );
        assert_eq!(
            settings.max_skills,
            crate::skill_router::MAX_INJECTABLE_SKILLS
        );
    }
}

#[cfg(test)]
mod builtin_skills_discovery_tests {
    use super::*;

    #[test]
    fn builtins_discovered_and_can_be_disabled() {
        let cfg = HarnessConfig::default();
        let root = std::env::temp_dir().join(format!("harness-skills-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let all = cfg.discover_skills_in(&root);
        assert!(all.iter().any(|s| s.name == "code" && s.path == "builtin://code"));
        assert!(all.iter().any(|s| s.name == "reviewer"));
        assert!(all.iter().filter(|s| s.name == "web-research").count() == 1);

        let mut disabled = cfg.clone();
        disabled.disabled_skills = vec!["code".to_string()];
        let after = disabled.discover_skills_in(&root);
        let code = after.iter().find(|s| s.name == "code").expect("code present");
        assert!(!code.enabled, "disabled built-in should remain listed but disabled");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_skill_overrides_builtin() {
        let cfg = HarnessConfig::default();
        let root = std::env::temp_dir().join(format!("harness-skills-override-{}", std::process::id()));
        let skills_dir = root.join(".harness").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("code.md"), "---\nname: code\ndescription: custom\n---\n\ncustom body").unwrap();
        let all = cfg.discover_skills_in(&root);
        let code: Vec<_> = all.iter().filter(|s| s.name == "code").collect();
        assert_eq!(code.len(), 1, "override should not duplicate");
        assert_eq!(code[0].description, "custom");
        assert!(code[0].content.contains("custom body"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod subagent_effort_tests {
    use super::*;

    #[test]
    fn default_is_step_down() {
        let cfg = OrchestratorConfig::default();
        assert_eq!(cfg.subagent_thinking_effort, "step_down");
    }

    #[test]
    fn step_down_lowers_one_level() {
        assert_eq!(step_down_effort("high"), "medium");
        assert_eq!(step_down_effort("medium"), "low");
        assert_eq!(step_down_effort("low"), "minimal");
        assert_eq!(step_down_effort("minimal"), "none");
        // Never goes below the floor.
        assert_eq!(step_down_effort("none"), "none");
    }

    #[test]
    fn step_down_normalizes_aliases() {
        assert_eq!(step_down_effort("HIGH"), "medium");
        assert_eq!(step_down_effort("xhigh"), "medium");
        assert_eq!(step_down_effort("med"), "low");
        // Unknown labels are passed through untouched.
        assert_eq!(step_down_effort("turbo"), "turbo");
    }

    #[test]
    fn resolve_follows_orchestrator_and_steps_down() {
        // The headline behaviour: orchestrator high -> subagent medium.
        assert_eq!(resolve_subagent_effort("step_down", "high", "medium"), "medium");
        assert_eq!(resolve_subagent_effort("step_down", "medium", "medium"), "low");
        // Explicit inherit copies the orchestrator level.
        assert_eq!(resolve_subagent_effort("inherit", "high", "medium"), "high");
        // A fixed level always wins.
        assert_eq!(resolve_subagent_effort("low", "high", "medium"), "low");
        // Empty configuration falls back to the default.
        assert_eq!(resolve_subagent_effort("", "high", "medium"), "medium");
        // Inherit with no orchestrator level uses the default.
        assert_eq!(resolve_subagent_effort("inherit", "", "medium"), "medium");
    }

    #[test]
    fn sentinels_are_recognized() {
        for s in ["step_down", "STEP-DOWN", "one_lower", "lower", "downgrade", "reduce"] {
            assert!(is_step_down_sentinel(s), "{s} should be a step-down sentinel");
        }
        for s in ["inherit", "follow", "orchestrator", "same", "parent"] {
            assert!(is_inherit_sentinel(s), "{s} should be an inherit sentinel");
        }
        assert!(!is_step_down_sentinel("high"));
        assert!(!is_inherit_sentinel("high"));
    }

    #[test]
    fn missing_field_deserializes_to_step_down() {
        // Configs written before this field existed must default to step_down.
        let cfg: OrchestratorConfig = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(cfg.subagent_thinking_effort, "step_down");
    }
}
