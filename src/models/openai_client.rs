use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::HarnessConfig;
use crate::engine::context::{format_attachments_as_text, ChatMessage, ToolCall, ToolFunctionCall};
use crate::engine::events::HarnessEvent;
use super::capabilities::{infer_model_modalities, resolve_effective_reasoning, EffectiveReasoningAction};
use super::{ModelAdapter, ModelTurnResult};

/// Serialize internal `ChatMessage` items to OpenAI Chat Completions JSON,
/// emitting multimodal `content` arrays when images/video/audio are attached on supported models.
pub fn build_openai_api_messages(model_id: &str, messages: &[ChatMessage]) -> Vec<Value> {
    let modalities = infer_model_modalities(model_id, None);
    let mut out = Vec::with_capacity(messages.len());

    for msg in messages {
        if msg.attachments.is_empty() {
            let mut obj = serde_json::Map::new();
            obj.insert("role".into(), json!(msg.role));
            if let Some(ref c) = msg.content {
                obj.insert("content".into(), json!(c));
            }
            if let Some(ref r) = msg.reasoning_content {
                obj.insert("reasoning_content".into(), json!(r));
            }
            if let Some(ref tc) = msg.tool_calls {
                obj.insert("tool_calls".into(), json!(tc));
            }
            if let Some(ref id) = msg.tool_call_id {
                obj.insert("tool_call_id".into(), json!(id));
            }
            out.push(Value::Object(obj));
            continue;
        }

        let mut text_parts = Vec::new();
        if let Some(ref c) = msg.content {
            if !c.trim().is_empty() {
                text_parts.push(c.clone());
            }
        }
        let attached_txt = format_attachments_as_text(&msg.attachments, false);
        if !attached_txt.is_empty() {
            text_parts.push(attached_txt);
        }

        let mut media_parts: Vec<Value> = Vec::new();
        for att in &msg.attachments {
            if att.text_content.is_some() {
                continue;
            }
            let kind = att.kind.to_ascii_lowercase();
            let has_data = att.data_url.as_deref().is_some_and(|u| !u.is_empty());
            if kind == "image" && modalities.vision && has_data {
                if let Some(ref url) = att.data_url {
                    media_parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }));
                }
            } else if kind == "video" && modalities.video && has_data {
                if let Some(ref url) = att.data_url {
                    media_parts.push(json!({
                        "type": "video_url",
                        "video_url": { "url": url }
                    }));
                }
            } else if kind == "audio" && modalities.audio && has_data {
                if let Some(ref url) = att.data_url {
                    if let Some((meta, b64)) = url.split_once(";base64,") {
                        let fmt = meta
                            .strip_prefix("data:audio/")
                            .unwrap_or("mp3")
                            .split(';')
                            .next()
                            .unwrap_or("mp3");
                        media_parts.push(json!({
                            "type": "input_audio",
                            "input_audio": { "data": b64, "format": fmt }
                        }));
                    }
                }
            } else if att.mime_type == "application/pdf" && modalities.pdf && has_data {
                if let Some(ref url) = att.data_url {
                    media_parts.push(json!({
                        "type": "file",
                        "file": { "filename": att.name, "file_data": url }
                    }));
                }
            } else {
                text_parts.push(format!(
                    "[Attachment omitted: {} ({}, {} bytes) — active model {} lacks native {} input]",
                    att.name, att.mime_type, att.size_bytes, model_id, kind
                ));
            }
        }

        let merged_text = text_parts.join("\n\n");
        let content_val = if media_parts.is_empty() {
            json!(merged_text)
        } else {
            let mut arr = Vec::with_capacity(media_parts.len() + 1);
            if !merged_text.trim().is_empty() {
                arr.push(json!({ "type": "text", "text": merged_text }));
            }
            arr.extend(media_parts);
            Value::Array(arr)
        };

        out.push(json!({
            "role": msg.role,
            "content": content_val,
        }));
    }

    out
}

pub struct OpenAiClient {
    client: reqwest::Client,
    config: HarnessConfig,
}

impl OpenAiClient {
    pub fn new(config: HarnessConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(4)
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self { client, config }
    }

    fn endpoint_url(&self) -> String {
        let base = self.config.api_base.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    }
}

#[async_trait]
impl ModelAdapter for OpenAiClient {
    async fn stream_completion(
        &self,
        messages: &[ChatMessage],
        tools: &Value,
        event_tx: &mpsc::Sender<HarnessEvent>,
    ) -> Result<ModelTurnResult, String> {
        // Fail with an actionable message instead of sending an unauthenticated
        // request that comes back as a bare upstream 401.
        let base = self.config.api_base.trim();
        let is_local = base.contains("localhost")
            || base.contains("127.0.0.1")
            || base.contains("[::1]");
        let key = self.config.api_key.trim();
        if base.is_empty() {
            return Err(
                "No model provider configured — add one in Settings → Providers.".to_string(),
            );
        }
        if key.is_empty() && !is_local {
            return Err(format!(
                "No API key for provider '{}'. Add one in Settings → Providers.",
                if self.config.provider.is_empty() {
                    base
                } else {
                    &self.config.provider
                }
            ));
        }
        if key == crate::secret_redaction::SECRET_REDACTED {
            return Err(
                "The stored API key was not sent with this request — re-enter it in \
                 Settings → Providers."
                    .to_string(),
            );
        }

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !self.config.api_key.is_empty() {
            let auth_val = format!("Bearer {}", self.config.api_key);
            match HeaderValue::from_str(&auth_val) {
                Ok(hv) => {
                    headers.insert(AUTHORIZATION, hv);
                }
                Err(_) => {
                    return Err(
                        "API key contains non-visible-ASCII characters; cannot build Authorization header."
                            .to_string(),
                    );
                }
            }
        }

        let api_messages = build_openai_api_messages(&self.config.model, messages);
        let mut payload = json!({
            "model": self.config.model,
            "messages": api_messages,
            "stream": true,
            "temperature": self.config.temperature,
            "stream_options": { "include_usage": true },
        });

        // Model-capability-aware thinking / reasoning effort control
        let model_lower = self.config.model.to_lowercase();
        let base_lower = self.config.api_base.to_lowercase();

        match resolve_effective_reasoning(&self.config.model, &self.config.reasoning_effort, None) {
            EffectiveReasoningAction::Omit => {}
            EffectiveReasoningAction::Disable => {
                if model_lower.contains("claude") || base_lower.contains("anthropic") {
                    payload["thinking"] = json!({ "type": "disabled" });
                } else if model_lower.contains("gemini") {
                    payload["thinking_config"] = json!({ "thinking_budget": 0 });
                } else {
                    payload["reasoning_effort"] = json!("none");
                }
            }
            EffectiveReasoningAction::Effort(eff) => {
                if model_lower.contains("claude") || base_lower.contains("anthropic") {
                    let budget = match eff.as_str() {
                        "high" => 16384,
                        "medium" => 8192,
                        "low" => 2048,
                        "minimal" => 1024,
                        val => val.parse::<u64>().unwrap_or(4096),
                    };
                    payload["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
                } else if model_lower.contains("gemini") {
                    let budget = match eff.as_str() {
                        "high" => 8192,
                        "medium" => 4096,
                        "low" => 1024,
                        "minimal" => 512,
                        val => val.parse::<u64>().unwrap_or(2048),
                    };
                    payload["thinking_config"] = json!({ "thinking_budget": budget });
                } else {
                    payload["reasoning_effort"] = json!(eff);
                }
            }
        }

        if let Some(tool_arr) = tools.as_array() {
            if !tool_arr.is_empty() {
                payload["tools"] = tools.clone();
            }
        }

        let url = self.endpoint_url();
        const PRE_STREAM_RETRY_BACKOFF: Duration = Duration::from_millis(1200);
        let mut retried = false;

        loop {
            if event_tx.is_closed() {
                return Err("Stream cancelled: event receiver dropped".to_string());
            }

            let mut res = match self.client.post(&url)
                .headers(headers.clone())
                .json(&payload)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if !retried {
                        retried = true;
                        sleep_with_cancel(PRE_STREAM_RETRY_BACKOFF, event_tx).await?;
                        continue;
                    }
                    return Err(format!("HTTP request error to {}: {}", url, e));
                }
            };

            // Fallback retry if the model endpoint rejected thinking or stream_options parameters
            if res.status().as_u16() == 400 && (payload.get("reasoning_effort").is_some() || payload.get("thinking").is_some() || payload.get("thinking_config").is_some() || payload.get("stream_options").is_some()) {
                let err_body = res.text().await.unwrap_or_default();
                if err_body.contains("reasoning_effort") || err_body.contains("thinking") || err_body.contains("stream_options") || err_body.contains("Extra inputs") || err_body.contains("unexpected") || err_body.contains("Additional properties") {
                    if let Some(obj) = payload.as_object_mut() {
                        obj.remove("reasoning_effort");
                        obj.remove("thinking");
                        obj.remove("thinking_config");
                        obj.remove("stream_options");
                    }
                    res = match self.client.post(&url)
                        .headers(headers.clone())
                        .json(&payload)
                        .send()
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            if !retried {
                                retried = true;
                                sleep_with_cancel(PRE_STREAM_RETRY_BACKOFF, event_tx).await?;
                                continue;
                            }
                            return Err(format!("HTTP request error to {}: {}", url, e));
                        }
                    };
                } else {
                    return Err(format!("Model API error (status 400): {}", err_body));
                }
            }

            if !res.status().is_success() {
                let status = res.status();
                let status_code = status.as_u16();
                let err_body = res.text().await.unwrap_or_default();
                if !retried && is_transient_status(status_code) {
                    retried = true;
                    sleep_with_cancel(PRE_STREAM_RETRY_BACKOFF, event_tx).await?;
                    continue;
                }
                return Err(format!("Model API error (status {}): {}", status, err_body));
            }

            let mut stream = res.bytes_stream();
            let mut full_content = String::new();
            let mut full_reasoning = String::new();
            let mut tool_calls_builder: HashMap<usize, (String, String, String)> = HashMap::new(); // index -> (id, name, args)
            let mut finish_reason: Option<String> = None;
            let mut line_buffer = String::new();
            let mut is_done = false;
            let mut in_think_tag = false;
            let mut prompt_tokens = 0usize;
            let mut completion_tokens = 0usize;
            let mut cached_tokens = 0usize;
            let mut emitted_any = false;
            let mut retry_pre_stream = false;

            while let Some(chunk_res) = stream.next().await {
                let chunk = match chunk_res {
                    Ok(c) => c,
                    Err(e) => {
                        if !emitted_any && !retried {
                            retried = true;
                            retry_pre_stream = true;
                            break;
                        }
                        return Err(format!("Stream read error: {}", e));
                    }
                };
                line_buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = line_buffer.find('\n') {
                    let line: String = line_buffer.drain(..=pos).collect();
                    let trimmed = line.trim();

                    if trimmed.is_empty() || trimmed.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = trimmed.strip_prefix("data:") {
                        let data = data.trim_start();
                        if data == "[DONE]" {
                            is_done = true;
                            break;
                        }

                        if let Ok(val) = serde_json::from_str::<Value>(data) {
                            // Parse usage details if returned in stream
                            if let Some(usage) = val.get("usage") {
                                if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                                    prompt_tokens = pt as usize;
                                }
                                if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                                    completion_tokens = ct as usize;
                                }
                                if let Some(cached) = usage.get("prompt_tokens_details")
                                    .and_then(|d| d.get("cached_tokens"))
                                    .and_then(|v| v.as_u64())
                                {
                                    cached_tokens = cached as usize;
                                }
                            }

                            if let Some(choices) = val.get("choices").and_then(|c| c.as_array()) {
                                if let Some(first) = choices.first() {
                                    if let Some(fr) = first.get("finish_reason").and_then(|r| r.as_str()) {
                                        finish_reason = Some(fr.to_string());
                                    }

                                    if let Some(delta) = first.get("delta") {
                                        // 1. Explicit reasoning fields (DeepSeek, NVIDIA, Groq, OpenRouter, Gemini)
                                        let explicit_reasoning = delta.get("reasoning_content")
                                            .or_else(|| delta.get("reasoning"))
                                            .or_else(|| delta.get("thought"))
                                            .and_then(|r| r.as_str());

                                        if let Some(reasoning) = explicit_reasoning {
                                            if !reasoning.is_empty() {
                                                emitted_any = true;
                                                full_reasoning.push_str(reasoning);
                                                let _ = event_tx.send(HarnessEvent::ThoughtDelta {
                                                    delta: reasoning.to_string(),
                                                }).await;
                                            }
                                        }

                                        // 2. Standard content stream with dynamic <think> tag extraction
                                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                            if !content.is_empty() {
                                                let mut remaining = content;
                                                while !remaining.is_empty() {
                                                    if in_think_tag {
                                                        if let Some(end_idx) = remaining.find("</think>") {
                                                            let think_part = &remaining[..end_idx];
                                                            if !think_part.is_empty() {
                                                                emitted_any = true;
                                                                full_reasoning.push_str(think_part);
                                                                let _ = event_tx.send(HarnessEvent::ThoughtDelta {
                                                                    delta: think_part.to_string(),
                                                                }).await;
                                                            }
                                                            in_think_tag = false;
                                                            remaining = &remaining[end_idx + 8..];
                                                        } else {
                                                            emitted_any = true;
                                                            full_reasoning.push_str(remaining);
                                                            let _ = event_tx.send(HarnessEvent::ThoughtDelta {
                                                                delta: remaining.to_string(),
                                                            }).await;
                                                            break;
                                                        }
                                                    } else if let Some(start_idx) = remaining.find("<think>") {
                                                        let normal_part = &remaining[..start_idx];
                                                        if !normal_part.is_empty() {
                                                            emitted_any = true;
                                                            full_content.push_str(normal_part);
                                                            let _ = event_tx.send(HarnessEvent::MessageDelta {
                                                                delta: normal_part.to_string(),
                                                            }).await;
                                                        }
                                                        in_think_tag = true;
                                                        remaining = &remaining[start_idx + 7..];
                                                    } else {
                                                        emitted_any = true;
                                                        full_content.push_str(remaining);
                                                        let _ = event_tx.send(HarnessEvent::MessageDelta {
                                                            delta: remaining.to_string(),
                                                        }).await;
                                                        break;
                                                    }
                                                }
                                            }
                                        }

                                        // 3. Tool calls stream
                                        if let Some(tc_array) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                            for tc in tc_array {
                                                emitted_any = true;
                                                let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                                let entry = tool_calls_builder.entry(idx).or_insert_with(|| (
                                                    String::new(),
                                                    String::new(),
                                                    String::new()
                                                ));

                                                if let Some(id) = tc.get("id").and_then(|s| s.as_str()) {
                                                    if entry.0.is_empty() {
                                                        entry.0.push_str(id);
                                                    }
                                                }

                                                if let Some(func) = tc.get("function") {
                                                    if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                                        if entry.1.is_empty() {
                                                            entry.1.push_str(name);
                                                        }
                                                    }
                                                    if let Some(args_val) = func.get("arguments") {
                                                        if let Some(args_str) = args_val.as_str() {
                                                            entry.2.push_str(args_str);
                                                        } else if !args_val.is_null() {
                                                            entry.2.push_str(&args_val.to_string());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if is_done {
                    break;
                }
            }

            if retry_pre_stream {
                sleep_with_cancel(PRE_STREAM_RETRY_BACKOFF, event_tx).await?;
                continue;
            }

            // Convert accumulated tool calls
            let mut sorted_indices: Vec<usize> = tool_calls_builder.keys().cloned().collect();
            sorted_indices.sort();

            let mut tool_calls = Vec::new();
            for idx in sorted_indices {
                if let Some((id, name, args)) = tool_calls_builder.remove(&idx) {
                    let call_id = if id.is_empty() { format!("call_{}", idx) } else { id };
                    tool_calls.push(ToolCall {
                        id: call_id,
                        r#type: "function".to_string(),
                        function: ToolFunctionCall {
                            name,
                            arguments: args,
                        },
                    });
                }
            }

            // Fallback parser: XML or markdown formatted tool calls (e.g., <tool_call>...</tool_call>)
            if tool_calls.is_empty() && full_content.contains("<tool_call>") {
                if let Some(extracted) = parse_xml_tool_call(&full_content) {
                    tool_calls.push(extracted);
                }
            }

            let estimated_tokens = if completion_tokens > 0 { completion_tokens } else { (full_content.len() + full_reasoning.len()) / 4 };

            return Ok(ModelTurnResult {
                content: if full_content.is_empty() { None } else { Some(full_content) },
                reasoning_content: if full_reasoning.is_empty() { None } else { Some(full_reasoning) },
                tool_calls,
                finish_reason,
                estimated_tokens,
                prompt_tokens,
                completion_tokens,
                cached_tokens,
            });
        }
    }
}

fn is_transient_status(status_code: u16) -> bool {
    matches!(status_code, 429 | 502 | 503 | 504)
}

async fn sleep_with_cancel(
    duration: Duration,
    event_tx: &mpsc::Sender<HarnessEvent>,
) -> Result<(), String> {
    if event_tx.is_closed() {
        return Err("Stream cancelled: event receiver dropped".to_string());
    }
    tokio::select! {
        _ = tokio::time::sleep(duration) => {
            if event_tx.is_closed() {
                Err("Stream cancelled: event receiver dropped".to_string())
            } else {
                Ok(())
            }
        }
        _ = event_tx.closed() => {
            Err("Stream cancelled: event receiver dropped".to_string())
        }
    }
}

fn parse_xml_tool_call(text: &str) -> Option<ToolCall> {
    let start_tag = "<tool_call>";
    let end_tag = "</tool_call>";
    let start_idx = text.find(start_tag)? + start_tag.len();
    let end_idx = text[start_idx..].find(end_tag)? + start_idx;
    let snippet = text[start_idx..end_idx].trim();

    if let Ok(val) = serde_json::from_str::<Value>(snippet) {
        let name = val.get("name").and_then(|n| n.as_str())?.to_string();
        let args = val.get("arguments").or_else(|| val.get("args")).map(|a| a.to_string()).unwrap_or_else(|| "{}".to_string());
        return Some(ToolCall {
            id: "xml_call_1".into(),
            r#type: "function".into(),
            function: ToolFunctionCall {
                name,
                arguments: args,
            },
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn read_http_request(socket: &mut tokio::net::TcpStream) {
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
    }

    fn test_config(base_url: String) -> HarnessConfig {
        HarnessConfig {
            api_base: base_url,
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            ..HarnessConfig::default()
        }
    }

    #[tokio::test]
    async fn test_pre_stream_retry_on_503_then_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_srv = calls.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break; };
                let n = calls_srv.fetch_add(1, Ordering::SeqCst);
                read_http_request(&mut socket).await;
                if n == 0 {
                    let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy";
                    let _ = socket.write_all(resp.as_bytes()).await;
                } else {
                    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                }
            }
        });

        let client = OpenAiClient::new(test_config(format!("http://{}", addr)));
        let (tx, mut rx) = mpsc::channel(16);
        let result = client.stream_completion(&[], &json!([]), &tx).await.unwrap();
        assert_eq!(result.content.as_deref(), Some("ok"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(rx.try_recv(), Ok(HarnessEvent::MessageDelta { delta }) if delta == "ok"));
    }

    #[tokio::test]
    async fn test_pre_stream_retry_on_stream_error_before_any_chunk() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_srv = calls.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break; };
                let n = calls_srv.fetch_add(1, Ordering::SeqCst);
                read_http_request(&mut socket).await;
                if n == 0 {
                    // Malformed chunked stream before any SSE data causes reqwest body decode error
                    let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nZZZ\r\n";
                    let _ = socket.write_all(resp.as_bytes()).await;
                } else {
                    let body = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\ndata: [DONE]\n\n";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(resp.as_bytes()).await;
                }
            }
        });

        let client = OpenAiClient::new(test_config(format!("http://{}", addr)));
        let (tx, _rx) = mpsc::channel(16);
        let result = client.stream_completion(&[], &json!([]), &tx).await.unwrap();
        assert_eq!(result.reasoning_content.as_deref(), Some("think"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_no_retry_after_emitted_chunk() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_srv = calls.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break; };
                calls_srv.fetch_add(1, Ordering::SeqCst);
                read_http_request(&mut socket).await;
                let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\nZZZ\r\n",
                    sse.len(),
                    sse
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            }
        });

        let client = OpenAiClient::new(test_config(format!("http://{}", addr)));
        let (tx, _rx) = mpsc::channel(16);
        let err = client.stream_completion(&[], &json!([]), &tx).await.unwrap_err();
        assert!(err.starts_with("Stream read error:"), "unexpected err: {}", err);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_pre_stream_retry_respects_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_srv = calls.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break; };
                calls_srv.fetch_add(1, Ordering::SeqCst);
                read_http_request(&mut socket).await;
                let resp = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy";
                let _ = socket.write_all(resp.as_bytes()).await;
            }
        });

        let client = OpenAiClient::new(test_config(format!("http://{}", addr)));
        let (tx, rx) = mpsc::channel(16);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(rx);
        });

        let start = std::time::Instant::now();
        let err = client.stream_completion(&[], &json!([]), &tx).await.unwrap_err();
        assert!(err.contains("cancelled"), "unexpected err: {}", err);
        assert!(start.elapsed() < Duration::from_millis(800));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
