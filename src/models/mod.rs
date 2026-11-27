use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::engine::context::{ChatMessage, ToolCall};
use crate::engine::events::HarnessEvent;

pub mod capabilities;
pub mod google_search_client;
pub mod mock_client;
pub mod openai_client;

#[derive(Debug, Clone, Default)]
pub struct ModelTurnResult {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
    pub estimated_tokens: usize,
    pub prompt_tokens: usize,
    #[allow(dead_code)]
    pub completion_tokens: usize,
    pub cached_tokens: usize,
}

#[async_trait]
pub trait ModelAdapter: Send + Sync {
    async fn stream_completion(
        &self,
        messages: &[ChatMessage],
        tools: &Value,
        event_tx: &mpsc::Sender<HarnessEvent>,
    ) -> Result<ModelTurnResult, String>;
}
