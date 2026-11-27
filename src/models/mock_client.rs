use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::engine::context::{ChatMessage, ToolCall, ToolFunctionCall};
use crate::engine::events::HarnessEvent;
use super::{ModelAdapter, ModelTurnResult};

pub struct MockClient;

#[async_trait]
impl ModelAdapter for MockClient {
    async fn stream_completion(
        &self,
        messages: &[ChatMessage],
        _tools: &Value,
        event_tx: &mpsc::Sender<HarnessEvent>,
    ) -> Result<ModelTurnResult, String> {
        let has_tool_results = messages.iter().any(|m| m.role == "tool");

        if !has_tool_results {
            // Step 1: Simulate thinking and calling a tool
            let thoughts = [
                "Analyzing user request and available workspace tools...\n",
                "DeepSeek Harness initialized in high-efficiency Rust runtime.\n",
                "Invoking 'sys_info' to verify hardware specs and harness RSS memory footprint.\n",
            ];

            for t in thoughts {
                let _ = event_tx.send(HarnessEvent::ThoughtDelta { delta: t.to_string() }).await;
                sleep(Duration::from_millis(40)).await;
            }

            let tool_call = ToolCall {
                id: "call_mock_1".into(),
                r#type: "function".into(),
                function: ToolFunctionCall {
                    name: "sys_info".into(),
                    arguments: "{}".into(),
                },
            };

            Ok(ModelTurnResult {
                content: None,
                reasoning_content: Some(thoughts.concat()),
                tool_calls: vec![tool_call],
                finish_reason: Some("tool_calls".into()),
                estimated_tokens: 120,
                ..Default::default()
            })
        } else {
            // Step 2: Tool result received -> stream final analysis
            let thoughts = [
                "Received system diagnostic information.\n",
                "Synthesizing final comprehensive report for user.\n",
            ];

            for t in thoughts {
                let _ = event_tx.send(HarnessEvent::ThoughtDelta { delta: t.to_string() }).await;
                sleep(Duration::from_millis(30)).await;
            }

            let response_chunks = [
                "### DeepSeek Harness (Rust Edition) - Execution Report\n\n",
                "The task executed successfully with the following highlights:\n\n",
                "- **Language & Architecture:** 100% Rust with zero-cost abstractions\n",
                "- **Resource Efficiency:** Zero runtime garbage collection, bounded memory buffers, HTTP compression\n",
                "- **Plugin System:** 'Everything is a Plugin' modular tool architecture matching DeepSeek Cordis\n",
                "- **Agent Loop:** Autonomous ReAct loop with self-correction on tool failures\n\n",
                "System diagnostics verified successfully.",
            ];

            for chunk in response_chunks {
                let _ = event_tx.send(HarnessEvent::MessageDelta { delta: chunk.to_string() }).await;
                sleep(Duration::from_millis(25)).await;
            }

            Ok(ModelTurnResult {
                content: Some(response_chunks.concat()),
                reasoning_content: Some(thoughts.concat()),
                tool_calls: vec![],
                finish_reason: Some("stop".into()),
                estimated_tokens: 280,
                ..Default::default()
            })
        }
    }
}
