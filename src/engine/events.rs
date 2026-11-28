use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum HarnessEvent {
    SessionStarted {
        session_id: String,
        task: String,
    },
    IterationStep {
        step: usize,
        max_steps: usize,
    },
    ThoughtDelta {
        delta: String,
    },
    MessageDelta {
        delta: String,
    },
    PermissionRequested {
        perm_id: String,
        session_id: String,
        tool: String,
        description: String,
        args: serde_json::Value,
    },
    PermissionDecided {
        perm_id: String,
        tool: String,
        approved: bool,
        /// Why it was not approved: "timeout" (no answer within 120s), "stopped"
        /// (the user stopped the turn), or None for a plain denial. A timeout or a
        /// stop is NOT a denial, and telling the user they denied something they
        /// never saw is a lie that costs them trust in every other prompt.
        #[serde(default)]
        reason: Option<String>,
    },
    ToolCallStarted {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolCallFinished {
        call_id: String,
        tool: String,
        success: bool,
        output: String,
        duration_ms: u64,
        truncated: bool,
    },
    MetricsUpdate {
        rss_memory_mb: f64,
        cpu_usage_pct: f32,
        total_tokens_est: usize,
        elapsed_secs: f64,
        tool_invocations: usize,
    },
    TokenUsageUpdate {
        current_context_tokens: usize,
        max_context_tokens: usize,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
        cached_tokens: usize,
        cache_hit_rate_pct: f32,
        estimated_cost_usd: f64,
    },
    Warning {
        message: String,
    },
    Error {
        message: String,
        fatal: bool,
    },
    SessionCompleted {
        session_id: String,
        final_answer: String,
        total_steps: usize,
        duration_secs: f64,
    },
    SubagentSpawned {
        subagent_id: String,
        role: String,
        task: String,
        parent_id: Option<String>,
    },
    SubagentUpdate {
        subagent_id: String,
        role: String,
        status: String,
        delta_kind: String,
        content: String,
    },
    SubagentCompleted {
        subagent_id: String,
        role: String,
        success: bool,
        summary: String,
        duration_secs: f64,
    },
    SwarmBoardMessage {
        seq: usize,
        sender: String,
        target: Option<String>,
        content: String,
        timestamp: String,
    },
    ArtifactCreated {
        id: String,
        title: String,
        kind: String,
        path: String,
        preview: String,
    },
    BrowserSnapshot {
        url: String,
        title: String,
        screenshot_url: Option<String>,
        console_errors: Vec<String>,
        dom_summary: String,
    },
    UserMessage {
        turn_id: usize,
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<crate::engine::context::ChatAttachment>,
        timestamp: String,
    },
    SystemCommand {
        turn_id: usize,
        command: String,
        args: String,
        icon: String,
        label: String,
        timestamp: String,
    },
    PromptQueued {
        queue_id: String,
        content: String,
        position: usize,
    },
    PromptDequeued {
        queue_id: String,
        content: String,
        mode: String,
    },
    QueueUpdated {
        queue: Vec<serde_json::Value>,
    },
    SteeringApplied {
        content: String,
    },
    MemoryCompacted {
        before_tokens: usize,
        after_tokens: usize,
        messages_folded: usize,
        reason: String,
    },
    ToolRetryScheduled {
        call_id: String,
        tool: String,
        attempt: usize,
        max_attempts: usize,
        reason: String,
    },
}
