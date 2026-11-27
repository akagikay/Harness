use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::engine::context::{estimate_message_tokens, ChatMessage, ToolCall, ToolFunctionCall};
use crate::engine::events::HarnessEvent;
use crate::plugins::browser_ui::{discover_chrome_browser, CdpBrowserSession};

use super::{ModelAdapter, ModelTurnResult};

pub const EXPERIMENTAL_GOOGLE_SEARCH_MODEL_ID: &str = "Google-Search-Model";

const GOOGLE_AI_MODE_URL: &str = "https://www.google.com/search?udm=50&hl=en";
const RESPONSE_POLL_INTERVAL_MS: u64 = 350;
const RESPONSE_TIMEOUT_SECS: u64 = 50;
const REQUIRED_STABLE_POLLS: u8 = 3;
const MAX_COMPACT_TOOL_ARGS: usize = 6;
const MAX_COMPACT_TOOL_DESC_CHARS: usize = 160;
const MAX_AI_MODE_PROMPT_CHARS: usize = 7800;

pub fn is_google_search_model(model_id: &str) -> bool {
    let lower = model_id.trim().to_ascii_lowercase();
    lower == "google-search-model" || lower == "experimental/google-search-model"
}

#[derive(Debug, Clone, Default)]
pub struct GoogleSearchClient;

impl GoogleSearchClient {
    pub fn new() -> Self {
        Self
    }
}

fn truncate_compact_description(desc: &str, max_chars: usize) -> String {
    let collapsed: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let cutoff = max_chars.saturating_sub(3);
    let byte_limit = collapsed
        .char_indices()
        .nth(cutoff)
        .map(|(idx, _)| idx)
        .unwrap_or(collapsed.len());
    let prefix = &collapsed[..byte_limit];
    let trimmed = if let Some(space_idx) = prefix.rfind(' ') {
        if space_idx >= byte_limit / 2 {
            &prefix[..space_idx]
        } else {
            prefix
        }
    } else {
        prefix
    };
    format!("{}...", trimmed.trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == ';'))
}

fn summarize_param_type(prop: &Value) -> String {
    if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
        let short_enums: Vec<&str> = enum_vals
            .iter()
            .filter_map(|v| v.as_str())
            .take(5)
            .collect();
        if !short_enums.is_empty() && enum_vals.len() <= 4 {
            let joined = short_enums.join("|");
            if joined.len() <= 48 {
                return joined;
            }
        }
    }

    match prop.get("type").and_then(|v| v.as_str()) {
        Some("string") => "string".to_string(),
        Some("integer") => "int".to_string(),
        Some("number") => "number".to_string(),
        Some("boolean") => "bool".to_string(),
        Some("array") => "array".to_string(),
        Some("object") => "object".to_string(),
        Some(other) => other.to_string(),
        None => "any".to_string(),
    }
}

fn format_tool_args(parameters: Option<&Value>) -> String {
    let Some(props) = parameters
        .and_then(|p| p.get("properties"))
        .and_then(|p| p.as_object())
    else {
        return String::new();
    };
    if props.is_empty() {
        return String::new();
    }

    let required_list: Vec<&str> = parameters
        .and_then(|p| p.get("required"))
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut ordered_keys: Vec<(&str, bool)> = Vec::new();
    for req in &required_list {
        if props.contains_key(*req) && !ordered_keys.iter().any(|(k, _)| k == req) {
            ordered_keys.push((*req, true));
        }
    }

    let mut optional_keys: Vec<&str> = props
        .keys()
        .map(String::as_str)
        .filter(|k| !ordered_keys.iter().any(|(ok, _)| ok == k))
        .collect();
    optional_keys.sort_unstable();
    for opt in optional_keys {
        ordered_keys.push((opt, false));
    }

    let limit = MAX_COMPACT_TOOL_ARGS.max(
        ordered_keys
            .iter()
            .filter(|(_, req)| *req)
            .count()
            .min(10),
    );
    let omitted = ordered_keys.len().saturating_sub(limit);

    let mut parts: Vec<String> = ordered_keys
        .into_iter()
        .take(limit)
        .map(|(key, is_required)| {
            let ty = props
                .get(key)
                .map(summarize_param_type)
                .unwrap_or_else(|| "any".to_string());
            if is_required {
                format!("{}: {}", key, ty)
            } else {
                format!("{}?: {}", key, ty)
            }
        })
        .collect();

    if omitted > 0 {
        parts.push(format!("+{} more", omitted));
    }
    parts.join(", ")
}

fn format_tool_definitions(tools: &Value) -> Vec<String> {
    let Some(arr) = tools.as_array() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for item in arr {
        let fn_obj = item.get("function").unwrap_or(item);
        let Some(name) = fn_obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let desc = fn_obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(|d| truncate_compact_description(d, MAX_COMPACT_TOOL_DESC_CHARS))
            .unwrap_or_default();
        let args = format_tool_args(fn_obj.get("parameters"));
        if desc.is_empty() {
            lines.push(format!("- {}({})", name, args));
        } else {
            lines.push(format!("- {}({}): {}", name, args, desc));
        }
    }
    lines
}

fn format_history_message(msg: &ChatMessage) -> Option<String> {
    let role = msg.role.trim().to_ascii_lowercase();
    let base_content = msg.content.as_deref().map(str::trim).unwrap_or("");
    let att_text = crate::engine::context::format_attachments_as_text(&msg.attachments, true);
    let combined = match (base_content.is_empty(), att_text.is_empty()) {
        (true, true) => String::new(),
        (false, true) => base_content.to_string(),
        (true, false) => att_text,
        (false, false) => format!("{}\n\n{}", base_content, att_text),
    };
    let content = combined.as_str();

    match role.as_str() {
        "user" => (!content.is_empty()).then(|| format!("User: {}", content)),
        "assistant" => {
            let mut parts = Vec::new();
            if !content.is_empty() {
                parts.push(content.to_string());
            }
            if let Some(ref calls) = msg.tool_calls {
                for call in calls {
                    let args_val = serde_json::from_str::<Value>(&call.function.arguments)
                        .unwrap_or_else(|_| Value::String(call.function.arguments.clone()));
                    let payload = json!({
                        "name": call.function.name,
                        "arguments": args_val
                    });
                    parts.push(format!(
                        "<harness_tool_call>{}</harness_tool_call>",
                        payload
                    ));
                }
            }
            (!parts.is_empty()).then(|| format!("Assistant: {}", parts.join("\n")))
        }
        "tool" => {
            let call_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
            Some(format!("Tool Result ({}): {}", call_id, content))
        }
        other => (!content.is_empty()).then(|| format!("{}: {}", other, content)),
    }
}

fn truncate_middle_for_budget(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    if max_chars <= 40 {
        return text.chars().take(max_chars).collect();
    }
    let marker = "\n...[truncated for Google AI Mode input limit]...\n";
    let avail = max_chars.saturating_sub(marker.chars().count());
    let head_len = avail * 6 / 10;
    let tail_len = avail.saturating_sub(head_len);
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text.chars().skip(char_count.saturating_sub(tail_len)).collect();
    format!("{}{}{}", head.trim_end(), marker, tail.trim_start())
}

/// Build a single cohesive user prompt for Google Search AI Mode (`udm=50`).
///
/// Since Google Search AI Mode has no separate `system` field, all `system`
/// messages and available tool definitions are wrapped inside a clear System Prompt
/// section with an explicit instruction telling AI Mode to obey them as its
/// authoritative System Prompt before answering the conversation / user request.
pub fn build_google_search_prompt(messages: &[ChatMessage], tools: &Value) -> String {
    let mut system_sections: Vec<String> = messages
        .iter()
        .filter(|m| m.role.eq_ignore_ascii_case("system"))
        .filter_map(|m| m.content.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let tool_lines = format_tool_definitions(tools);
    if !tool_lines.is_empty() {
        let mut tool_block = String::from(
            "AVAILABLE TOOLS:\nYou may call any tool listed below when needed to complete the task.\n",
        );
        tool_block.push_str(&tool_lines.join("\n"));
        tool_block.push_str(
            "\n\nTOOL CALL FORMAT:\nTo invoke a tool, emit one or more blocks using this exact XML tag and valid JSON:\n<harness_tool_call>{\"name\": \"tool_name\", \"arguments\": {\"key\": \"value\"}}</harness_tool_call>\nDo not invent or simulate tool results; wait for the harness to execute the tool and return the result.",
        );
        system_sections.push(tool_block);
    }

    let non_system: Vec<&ChatMessage> = messages
        .iter()
        .filter(|m| !m.role.eq_ignore_ascii_case("system"))
        .collect();

    let (history_msgs, current_request) = match non_system.last() {
        Some(last) if last.role.eq_ignore_ascii_case("user") => (
            &non_system[..non_system.len() - 1],
            last.content
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(empty user request)")
                .to_string(),
        ),
        _ if !non_system.is_empty() => (
            &non_system[..],
            "Continue from the conversation history and tool results above to complete the user's request (or emit another <harness_tool_call> if needed).".to_string(),
        ),
        _ => (&[][..], "Respond according to the system prompt.".to_string()),
    };

    let req_section = format!("User Request:\n{}", current_request);

    let history_lines: Vec<String> = history_msgs
        .iter()
        .filter_map(|m| format_history_message(m))
        .collect();

    let assemble = |sys_body: Option<&str>, hist_body: Option<&str>, req_body: &str| -> String {
        let mut sections = Vec::new();
        // Place the user's request first so Google AI Mode's query router recognizes it as a
        // direct question rather than falling back to web-link results on prompt boilerplate.
        sections.push(req_body.to_string());
        if let Some(hb) = hist_body {
            sections.push(format!("[CONVERSATION HISTORY]\n{}", hb));
        }
        if let Some(sb) = sys_body {
            sections.push(format!(
                "Listen to and strictly obey the instructions below as your authoritative System Prompt for this entire response before answering the request above:\n\n--- BEGIN SYSTEM PROMPT ---\n{}\n--- END SYSTEM PROMPT ---",
                sb
            ));
        }
        sections.join("\n\n")
    };

    let merged_system = (!system_sections.is_empty()).then(|| system_sections.join("\n\n"));
    let merged_history = (!history_lines.is_empty()).then(|| history_lines.join("\n\n"));

    let initial = assemble(
        merged_system.as_deref(),
        merged_history.as_deref(),
        &req_section,
    );
    if initial.chars().count() <= MAX_AI_MODE_PROMPT_CHARS {
        return initial;
    }

    // Fit within Google AI Mode's 8192-char textarea maxlength while preserving User Request.
    let req_chars = req_section.chars().count();
    let wrapper_overhead = 260usize;
    let remaining_for_ctx = MAX_AI_MODE_PROMPT_CHARS
        .saturating_sub(req_chars)
        .saturating_sub(wrapper_overhead);

    let trimmed_history = merged_history.as_ref().map(|hb| {
        let hist_budget = if merged_system.is_some() {
            (remaining_for_ctx / 3).max(400).min(remaining_for_ctx)
        } else {
            remaining_for_ctx
        };
        truncate_middle_for_budget(hb, hist_budget)
    });
    let hist_used = trimmed_history
        .as_ref()
        .map(|h| h.chars().count())
        .unwrap_or(0);
    let trimmed_system = merged_system.as_ref().map(|sb| {
        let sys_budget = remaining_for_ctx.saturating_sub(hist_used).max(600);
        truncate_middle_for_budget(sb, sys_budget)
    });

    assemble(
        trimmed_system.as_deref(),
        trimmed_history.as_deref(),
        &req_section,
    )
}

fn strip_code_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let after_lang = rest
            .find('\n')
            .map(|idx| &rest[idx + 1..])
            .unwrap_or(rest);
        if let Some(inner) = after_lang.strip_suffix("```") {
            return inner.trim();
        }
    }
    trimmed
}

fn parse_tool_call_json(raw_json: &str, call_index: usize) -> Option<ToolCall> {
    let cleaned = strip_code_fences(raw_json);
    let parsed: Value = serde_json::from_str(cleaned).ok()?;
    let obj = parsed.as_object()?;

    let name = obj
        .get("name")
        .or_else(|| obj.get("tool"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();

    let args_val = obj.get("arguments").or_else(|| obj.get("args"));
    let arguments = match args_val {
        Some(Value::String(s)) => {
            let s_trim = s.trim();
            if s_trim.is_empty() {
                "{}".to_string()
            } else {
                s_trim.to_string()
            }
        }
        Some(Value::Null) | None => "{}".to_string(),
        Some(other) => other.to_string(),
    };

    Some(ToolCall {
        id: format!("call_gs_{}", call_index),
        r#type: "function".to_string(),
        function: ToolFunctionCall { name, arguments },
    })
}

/// Parse `<harness_tool_call>...</harness_tool_call>` blocks from AI Mode output.
pub fn parse_google_search_response(
    raw_text: &str,
    allow_tool_calls: bool,
) -> (Option<String>, Vec<ToolCall>) {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return (None, Vec::new());
    }
    if !allow_tool_calls {
        return (Some(trimmed.to_string()), Vec::new());
    }

    const OPEN_TAG: &str = "<harness_tool_call>";
    const CLOSE_TAG: &str = "</harness_tool_call>";

    let lower = raw_text.to_ascii_lowercase();
    let mut cursor = 0usize;
    let mut prose = String::new();
    let mut tool_calls = Vec::new();

    while cursor < raw_text.len() {
        let Some(rel_open) = lower[cursor..].find(OPEN_TAG) else {
            prose.push_str(&raw_text[cursor..]);
            break;
        };
        let open_start = cursor + rel_open;
        let inner_start = open_start + OPEN_TAG.len();

        let Some(rel_close) = lower[inner_start..].find(CLOSE_TAG) else {
            prose.push_str(&raw_text[cursor..]);
            break;
        };
        let inner_end = inner_start + rel_close;
        let close_end = inner_end + CLOSE_TAG.len();

        prose.push_str(&raw_text[cursor..open_start]);
        let inner_payload = &raw_text[inner_start..inner_end];
        if let Some(call) = parse_tool_call_json(inner_payload, tool_calls.len() + 1) {
            tool_calls.push(call);
        } else {
            // Preserve unparseable block verbatim rather than dropping text silently.
            prose.push_str(&raw_text[open_start..close_end]);
        }

        cursor = close_end;
    }

    let cleaned_prose = prose.trim();
    let content = (!cleaned_prose.is_empty()).then(|| cleaned_prose.to_string());
    (content, tool_calls)
}

/// Detect whether Google returned a CAPTCHA, `/sorry/` block, or anti-bot gate.
///
/// Per security/policy rules, detected gates MUST abort immediately without
/// attempting to solve, click through, or bypass them.
pub fn detect_google_captcha_or_block(url: &str, page_text: &str) -> Option<String> {
    let url_lower = url.trim().to_ascii_lowercase();
    let text_lower = page_text.to_ascii_lowercase();

    if url_lower.contains("/sorry/")
        || url_lower.contains("sorry/index")
        || url_lower.contains("google.com/sorry")
        || url_lower.contains("/recaptcha/")
    {
        return Some(format!("blocked URL detected ({})", url.trim()));
    }

    let gate_phrases = [
        ("unusual traffic", "Google reported unusual traffic from this network"),
        (
            "automated queries",
            "Google blocked automated queries on this session",
        ),
        (
            "automated requests",
            "Google blocked automated requests on this session",
        ),
        (
            "i'm not a robot",
            "Google presented an 'I'm not a robot' verification gate",
        ),
        (
            "i am not a robot",
            "Google presented a robot verification gate",
        ),
        (
            "please show you're not a robot",
            "Google requested human verification",
        ),
        (
            "verify you are human",
            "Google requested human verification",
        ),
        (
            "verify you're human",
            "Google requested human verification",
        ),
        (
            "this page checks to see if it's really you",
            "Google presented a verification challenge page",
        ),
        (
            "to continue, please type the characters",
            "Google presented a character CAPTCHA challenge",
        ),
    ];

    for (needle, reason) in gate_phrases {
        if text_lower.contains(needle) {
            return Some(reason.to_string());
        }
    }

    if text_lower.contains("recaptcha")
        && (text_lower.contains("about this page")
            || text_lower.contains("not a robot")
            || text_lower.contains("our systems have detected")
            || url_lower.contains("google.com/sorry"))
    {
        return Some("Google presented a reCAPTCHA verification gate".to_string());
    }

    None
}

/// Inspect CDP failed network requests so subresource `/sorry/` or HTTP 429
/// verification redirects on Google AI Mode async endpoints fail fast rather
/// than waiting out the full response timeout.
pub fn detect_google_failed_request_block(failed_requests: &[Value]) -> Option<String> {
    for req in failed_requests {
        let url = req.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
        let url_lower = url.to_ascii_lowercase();
        let status = req.get("status").and_then(|v| v.as_u64());

        if url_lower.contains("/sorry/")
            || url_lower.contains("sorry/index")
            || url_lower.contains("google.com/sorry")
            || url_lower.contains("/recaptcha/")
        {
            return Some(format!(
                "subresource redirected to Google anti-bot gate ({})",
                url
            ));
        }

        if status == Some(429)
            && (url_lower.contains("google.com/async/")
                || url_lower.contains("google.com/search")
                || url_lower.contains("sgasync"))
        {
            return Some(format!(
                "Google AI Mode rate-limited request with HTTP 429 ({})",
                url
            ));
        }
    }
    None
}

async fn read_url_and_text(session: &mut CdpBrowserSession) -> Result<(String, String), String> {
    let val = session
        .eval_js(
            "(() => ({ url: location.href || '', text: (document.body && document.body.innerText) ? document.body.innerText : '' }))()",
        )
        .await?;
    let url = val
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let text = val
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((url, text))
}

async fn check_captcha_gate(session: &mut CdpBrowserSession) -> Result<(), String> {
    if let Some(reason) = detect_google_failed_request_block(session.failed_requests()) {
        return Err(format!(
            "Google Search AI Mode presented a CAPTCHA/verification gate and cannot be bypassed: {}",
            reason
        ));
    }
    let (url, text) = read_url_and_text(session).await?;
    if let Some(reason) = detect_google_captcha_or_block(&url, &text) {
        return Err(format!(
            "Google Search AI Mode presented a CAPTCHA/verification gate and cannot be bypassed: {}",
            reason
        ));
    }
    if text
        .to_ascii_lowercase()
        .contains("ai mode is not currently available")
    {
        return Err(
            "Google Search AI Mode reported: 'AI Mode is not currently available on your device or account'."
                .to_string(),
        );
    }
    Ok(())
}

async fn dismiss_cookie_consent_if_present(session: &mut CdpBrowserSession) -> Result<(), String> {
    let js = r#"(() => {
        const isVisible = (el) => {
            if (!el) return false;
            const st = window.getComputedStyle(el);
            if (!st || st.display === 'none' || st.visibility === 'hidden' || st.opacity === '0') return false;
            const r = el.getBoundingClientRect();
            return r.width > 0 && r.height > 0;
        };
        const directIds = ['W0wltc', 'L2AGLb'];
        for (const id of directIds) {
            const el = document.getElementById(id);
            if (isVisible(el)) {
                el.click();
                return 'clicked:' + id;
            }
        }
        const preferred = ['reject all', 'accept all', 'i agree'];
        const buttons = Array.from(document.querySelectorAll('button, [role="button"], input[type="submit"], input[type="button"]'));
        for (const target of preferred) {
            for (const btn of buttons) {
                if (!isVisible(btn)) continue;
                const label = (btn.innerText || btn.value || btn.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim().toLowerCase();
                if (label === target) {
                    btn.click();
                    return 'clicked:' + target;
                }
            }
        }
        return 'none';
    })()"#;

    if let Ok(res) = session.eval_js(js).await {
        if res
            .as_str()
            .map(|s| s.starts_with("clicked:"))
            .unwrap_or(false)
        {
            session.pump_events(600).await;
        }
    }
    Ok(())
}

async fn ensure_ai_mode_input_ready(session: &mut CdpBrowserSession) -> Result<(), String> {
    let find_input_js = r#"(() => {
        const isVisible = (el) => {
            if (!el || el.disabled) return false;
            const st = window.getComputedStyle(el);
            if (!st || st.display === 'none' || st.visibility === 'hidden') return false;
            const r = el.getBoundingClientRect();
            return r.width > 0 && r.height > 0;
        };
        const selectors = [
            'textarea.ITIRGe',
            'textarea.FGQVEf',
            'textarea[aria-label*="Ask" i]',
            'textarea[placeholder*="Ask" i]',
            '[contenteditable="true"][role="textbox"]',
            'textarea[name="q"]',
            'textarea'
        ];
        for (const sel of selectors) {
            const els = Array.from(document.querySelectorAll(sel));
            for (const el of els) {
                if (isVisible(el)) {
                    return { found: true, selector: sel, udm50: location.href.includes('udm=50') };
                }
            }
        }
        return { found: false, udm50: location.href.includes('udm=50') };
    })()"#;

    let initial = session.eval_js(find_input_js).await?;
    let found = initial
        .get("found")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let udm50 = initial
        .get("udm50")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !found || !udm50 {
        let click_ai_mode_js = r#"(() => {
            const isVisible = (el) => {
                if (!el) return false;
                const st = window.getComputedStyle(el);
                if (!st || st.display === 'none' || st.visibility === 'hidden') return false;
                const r = el.getBoundingClientRect();
                return r.width > 0 && r.height > 0;
            };
            const udmLink = Array.from(document.querySelectorAll('a[href*="udm=50"]')).find(isVisible);
            if (udmLink) {
                udmLink.click();
                return 'clicked_udm50_link';
            }
            const candidates = Array.from(document.querySelectorAll('a, button, [role="tab"], [role="button"]'));
            for (const el of candidates) {
                if (!isVisible(el)) continue;
                const txt = (el.innerText || el.getAttribute('aria-label') || '').replace(/\s+/g, ' ').trim().toLowerCase();
                if (txt === 'ai mode' || txt.includes('ai mode')) {
                    el.click();
                    return 'clicked_ai_mode_text';
                }
            }
            return 'not_found';
        })()"#;

        let click_res = session.eval_js(click_ai_mode_js).await?;
        if click_res
            .as_str()
            .map(|s| s.starts_with("clicked_"))
            .unwrap_or(false)
        {
            session.pump_events(1200).await;
            check_captcha_gate(session).await?;
        }
    }

    for _ in 0..10 {
        let state = session.eval_js(find_input_js).await?;
        if state.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(());
        }
        session.pump_events(300).await;
        check_captcha_gate(session).await?;
    }

    Err("Could not locate visible Google Search AI Mode input textbox.".to_string())
}

async fn fill_and_submit_prompt(session: &mut CdpBrowserSession, prompt: &str) -> Result<(), String> {
    let focus_js = r#"(() => {
        const isVisible = (el) => {
            if (!el || el.disabled) return false;
            const st = window.getComputedStyle(el);
            if (!st || st.display === 'none' || st.visibility === 'hidden') return false;
            const r = el.getBoundingClientRect();
            return r.width > 0 && r.height > 0;
        };
        const selectors = [
            'textarea.ITIRGe',
            'textarea.FGQVEf',
            'textarea[aria-label*="Ask" i]',
            'textarea[placeholder*="Ask" i]',
            '[contenteditable="true"][role="textbox"]',
            'textarea[name="q"]',
            'textarea'
        ];
        for (const sel of selectors) {
            for (const el of document.querySelectorAll(sel)) {
                if (!isVisible(el)) continue;
                el.scrollIntoView({ block: 'center' });
                el.click();
                el.focus();
                if (el.isContentEditable) {
                    el.innerText = '';
                } else {
                    el.value = '';
                    el.dispatchEvent(new Event('input', { bubbles: true }));
                }
                return { ok: true, selector: sel, isContentEditable: Boolean(el.isContentEditable) };
            }
        }
        return { ok: false };
    })()"#;

    let focus_res = session.eval_js(focus_js).await?;
    if !focus_res
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("Failed to focus Google Search AI Mode prompt input.".to_string());
    }

    // Wake Google AI Mode's Wiz key/input listeners with a real CDP keystroke before value injection.
    let _ = session
        .call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyDown",
                "key": " ",
                "text": " ",
                "code": "Space",
                "windowsVirtualKeyCode": 32,
                "nativeVirtualKeyCode": 32
            }),
        )
        .await;
    let _ = session
        .call("Input.dispatchKeyEvent", json!({ "type": "char", "text": " " }))
        .await;
    let _ = session
        .call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": " ",
                "code": "Space",
                "windowsVirtualKeyCode": 32,
                "nativeVirtualKeyCode": 32
            }),
        )
        .await;
    session.pump_events(80).await;

    let prompt_json = serde_json::to_string(prompt)
        .map_err(|e| format!("Failed to serialize prompt for DOM injection: {}", e))?;
    let set_value_js = format!(
        r#"(() => {{
        const promptText = {prompt_json};
        const isVisible = (el) => {{
            if (!el || el.disabled) return false;
            const st = window.getComputedStyle(el);
            if (!st || st.display === 'none' || st.visibility === 'hidden') return false;
            const r = el.getBoundingClientRect();
            return r.width > 0 && r.height > 0;
        }};
        const selectors = [
            'textarea.ITIRGe',
            'textarea.FGQVEf',
            'textarea[aria-label*="Ask" i]',
            'textarea[placeholder*="Ask" i]',
            '[contenteditable="true"][role="textbox"]',
            'textarea[name="q"]',
            'textarea'
        ];
        let target = null;
        for (const sel of selectors) {{
            for (const el of document.querySelectorAll(sel)) {{
                if (isVisible(el)) {{ target = el; break; }}
            }}
            if (target) break;
        }}
        if (!target) return {{ ok: false, len: 0 }};
        target.focus();
        if (target.isContentEditable) {{
            target.innerText = promptText;
        }} else {{
            const desc = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value');
            if (desc && desc.set) {{
                desc.set.call(target, promptText);
            }} else {{
                target.value = promptText;
            }}
        }}
        target.dispatchEvent(new Event('input', {{ bubbles: true }}));
        target.dispatchEvent(new Event('change', {{ bubbles: true }}));
        const finalVal = target.isContentEditable ? (target.innerText || '') : (target.value || '');
        return {{ ok: finalVal.trim().length > 0, len: finalVal.length }};
    }})()"#
    );

    let fill_res = session.eval_js(&set_value_js).await?;
    if !fill_res
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("Failed to populate Google Search AI Mode prompt textbox.".to_string());
    }

    // Trailing CDP Input.insertText guarantees the active input plate transitions to Send-enabled state.
    let _ = session
        .call("Input.insertText", json!({ "text": " " }))
        .await;
    session.pump_events(250).await;

    // Click ONLY an explicit visible & enabled Send/Submit button (never generic [type="submit"] buttons).
    let click_send_js = r#"(() => {
        const isVisibleEnabled = (el) => {
            if (!el || el.disabled || el.getAttribute('aria-disabled') === 'true') return false;
            const st = window.getComputedStyle(el);
            if (!st || st.display === 'none' || st.visibility === 'hidden') return false;
            const r = el.getBoundingClientRect();
            return r.width > 0 && r.height > 0;
        };
        const selectors = [
            'button[data-xid="input-plate-send-button"]',
            'button[aria-label="Send"]',
            'button[aria-label*="Send" i]',
            'button[aria-label*="Submit" i]'
        ];
        for (const sel of selectors) {
            for (const btn of document.querySelectorAll(sel)) {
                if (isVisibleEnabled(btn)) {
                    btn.click();
                    return 'clicked:' + sel;
                }
            }
        }
        return 'no_button';
    })()"#;

    let send_res = session.eval_js(click_send_js).await?;
    if send_res.as_str() == Some("no_button") {
        session
            .call(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyDown",
                    "key": "Enter",
                    "code": "Enter",
                    "windowsVirtualKeyCode": 13,
                    "nativeVirtualKeyCode": 13
                }),
            )
            .await?;
        let _ = session
            .call(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "char",
                    "text": "\r",
                    "unmodifiedText": "\r",
                    "windowsVirtualKeyCode": 13,
                    "nativeVirtualKeyCode": 13
                }),
            )
            .await;
        session
            .call(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyUp",
                    "key": "Enter",
                    "code": "Enter",
                    "windowsVirtualKeyCode": 13,
                    "nativeVirtualKeyCode": 13
                }),
            )
            .await?;
    }

    session.pump_events(400).await;
    Ok(())
}

fn is_placeholder_status_text(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "" | "thinking..."
            | "searching..."
            | "generating..."
            | "loading..."
            | "transcribing..."
            | "creating a response..."
            | "analyzing..."
            | "just a sec..."
            | "working on it..."
            | "ai mode"
            | "ask anything"
            | "ask about"
    )
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn rfind_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack
        .to_ascii_lowercase()
        .rfind(&needle.to_ascii_lowercase())
}

fn find_after_ignoring_ws(
    haystack: &str,
    needle: &str,
    search_from: usize,
    require_boundary: bool,
) -> Option<(usize, usize)> {
    let needle_chars: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if needle_chars.is_empty() || search_from > haystack.len() {
        return None;
    }
    let sub = &haystack[search_from..];
    for (rel_start, ch) in sub.char_indices() {
        if ch != needle_chars[0] {
            continue;
        }
        let mut matched = 0usize;
        for (offset, hc) in sub[rel_start..].char_indices() {
            if hc.is_whitespace() {
                continue;
            }
            if hc != needle_chars[matched] {
                break;
            }
            matched += 1;
            let rel_end = rel_start + offset + hc.len_utf8();
            if matched == needle_chars.len() {
                let abs_start = search_from + rel_start;
                let abs_end = search_from + rel_end;
                if require_boundary {
                    let prev_ok = haystack[..abs_start]
                        .chars()
                        .next_back()
                        .map(|c| !c.is_alphanumeric())
                        .unwrap_or(true);
                    let next_ok = haystack[abs_end..]
                        .chars()
                        .next()
                        .map(|c| !c.is_alphanumeric())
                        .unwrap_or(true);
                    if !(prev_ok && next_ok) {
                        break;
                    }
                }
                return Some((abs_start, abs_end));
            }
        }
    }
    None
}

fn strip_leading_ai_mode_chrome(mut text: &str) -> &str {
    text = text.trim_start();

    // If the extracted block begins with sidebar/header chrome ending at "AI Mode conversation:",
    // strip the entire leading chrome prefix regardless of intermediate sidebar items.
    if let Some(conv_idx) = find_ascii_ci(text, "ai mode conversation:") {
        if conv_idx <= 600 {
            let prefix_lower = text[..conv_idx].to_ascii_lowercase();
            let looks_like_chrome = prefix_lower.is_empty()
                || prefix_lower.contains("open sidebar")
                || prefix_lower.contains("new thread")
                || prefix_lower.contains("search history")
                || prefix_lower.contains("personalization")
                || prefix_lower.contains("connected content apps")
                || prefix_lower.contains("settings")
                || prefix_lower.contains("skip to main content");
            if looks_like_chrome {
                text = &text[conv_idx + "ai mode conversation:".len()..];
            }
        }
    }

    const LEADING_CHROME_PREFIXES: &[&str] = &[
        "open sidebar",
        "new thread",
        "see your search history",
        "personalization",
        "connected content apps",
        "settings",
        "ai mode conversation:",
        "ai mode conversation",
        "skip to main content",
        "accessibility help",
        "accessibility feedback",
        "filters and topics",
    ];

    loop {
        let trimmed = text.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        let mut stripped = false;

        for prefix in LEADING_CHROME_PREFIXES {
            if lower.starts_with(prefix) {
                text = &trimmed[prefix.len()..];
                stripped = true;
                break;
            }
        }
        if stripped {
            continue;
        }

        if lower.starts_with("ai mode") {
            let rest = &trimmed["ai mode".len()..];
            if rest.is_empty() || rest.starts_with('\n') || rest.starts_with(':') {
                text = rest.strip_prefix(':').unwrap_or(rest);
                continue;
            }
        }

        text = trimmed;
        break;
    }

    text.trim_start()
}

fn isolate_after_echoed_prompt<'a>(text: &'a str, submitted_prompt: &str) -> &'a str {
    let trimmed_prompt = submitted_prompt.trim();

    if !trimmed_prompt.is_empty() {
        // 1. Exact prompt match (take text after the last full match).
        if let Some(idx) = text.rfind(trimmed_prompt) {
            return &text[idx + trimmed_prompt.len()..];
        }

        // Anchor search past </system_prompt> or [END SYSTEM PROMPT OVERRIDE] when present in raw text
        // 1. Try matching the full submitted prompt first when available.
        let is_short_single_token =
            trimmed_prompt.chars().count() < 12 && !trimmed_prompt.chars().any(char::is_whitespace);
        if let Some((_, end)) =
            find_after_ignoring_ws(text, trimmed_prompt, 0, is_short_single_token)
        {
            return &text[end..];
        }

        // 2. Match closing system-prompt delimiter tail when system prompt appears last.
        for end_tag in ["--- END SYSTEM PROMPT ---", "[END SYSTEM PROMPT OVERRIDE]", "</system_prompt>"] {
            if let Some(end_override_pos) = trimmed_prompt.rfind(end_tag) {
                let tail = &trimmed_prompt[end_override_pos..];
                if let Some((_, end)) = find_after_ignoring_ws(text, tail, 0, false) {
                    return &text[end..];
                }
            }
        }

        // Skip past any echoed system-prompt boundary when searching for User Request / [CURRENT USER REQUEST],
        // so we never falsely match inside an echoed system prompt.
        let search_from = rfind_ascii_ci(text, "--- end system prompt ---")
            .map(|i| i + "--- end system prompt ---".len())
            .or_else(|| {
                rfind_ascii_ci(text, "[end system prompt override]")
                    .map(|i| i + "[end system prompt override]".len())
            })
            .or_else(|| rfind_ascii_ci(text, "</system_prompt>").map(|i| i + "</system_prompt>".len()))
            .unwrap_or(0);

        // 3. Match the tail "User Request:..." or "[CURRENT USER REQUEST]..." section ignoring DOM whitespace differences.
        for req_tag in ["User Request:", "[CURRENT USER REQUEST]"] {
            if let Some(req_pos) = trimmed_prompt.rfind(req_tag) {
                let tail_section = &trimmed_prompt[req_pos..];
                let query_body = tail_section[req_tag.len()..].trim();

                if let Some((_, end)) = find_after_ignoring_ws(text, tail_section, search_from, false) {
                    return &text[end..];
                }
                if search_from > 0 {
                    if let Some((_, end)) = find_after_ignoring_ws(text, tail_section, 0, false) {
                        return &text[end..];
                    }
                }

                // Also match alternative DOM request header labels (e.g., legacy "[CURRENT USER REQUEST]")
                for dom_tag in ["User Request:", "[CURRENT USER REQUEST]"] {
                    if !query_body.is_empty() {
                        let alt_section = format!("{}\n{}", dom_tag, query_body);
                        if let Some((_, end)) =
                            find_after_ignoring_ws(text, &alt_section, search_from, false)
                        {
                            return &text[end..];
                        }
                    }
                }

                // If AI Mode truncated a very long user request, try matching the end or beginning of tail_section.
                let tail_chars: Vec<char> = tail_section.chars().collect();
                if tail_chars.len() > 100 {
                    let tail_end: String = tail_chars[tail_chars.len() - 90..].iter().collect();
                    if let Some((_, end)) = find_after_ignoring_ws(text, &tail_end, search_from, false) {
                        return &text[end..];
                    }
                    let tail_head: String = tail_chars[..100].iter().collect();
                    if let Some((_, end)) = find_after_ignoring_ws(text, &tail_head, search_from, false) {
                        return &text[end..];
                    }
                }

                // Match just the request marker and advance past the user query line if present.
                for dom_tag in ["User Request:", "[CURRENT USER REQUEST]"] {
                    if let Some((_, req_end)) = find_after_ignoring_ws(text, dom_tag, search_from, false) {
                        let after_marker = text[req_end..].trim_start();
                        let first_query_line = query_body.lines().next().map(str::trim).unwrap_or("");
                        if !first_query_line.is_empty() {
                            if let Some((_, q_end)) =
                                find_after_ignoring_ws(after_marker, first_query_line, 0, false)
                            {
                                return &after_marker[q_end..];
                            }
                        }
                        return after_marker;
                    }
                }
            }
        }
    }

    // Fallback: if raw text still contains echoed prompt scaffolding, strip past known delimiters.
    let lower = text.to_ascii_lowercase();
    let has_echoed_system = lower.contains("critical system instruction:")
        || lower.contains("<system_prompt>")
        || lower.contains("[authoritative system prompt")
        || lower.contains("--- begin system prompt ---")
        || lower.contains("listen to and strictly obey the following instructions");

    if has_echoed_system {
        for end_marker in [
            "--- end system prompt ---",
            "[end system prompt override]",
            "</system_prompt>",
        ] {
            if let Some(idx) = rfind_ascii_ci(text, end_marker) {
                let after = text[idx + end_marker.len()..].trim_start();
                if let Some(after_req) = strip_optional_current_request_header(after) {
                    return after_req;
                }
                return after;
            }
        }
        // Prompt echo is present without a closing delimiter — still mid-echo or truncated,
        // so return empty rather than leaking system prompt instructions.
        return "";
    }

    text
}

fn strip_optional_current_request_header(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    for marker in ["User Request:", "[CURRENT USER REQUEST]"] {
        if trimmed.len() >= marker.len()
            && trimmed[..marker.len()].eq_ignore_ascii_case(marker)
        {
            let after = &trimmed[marker.len()..];
            return Some(after.trim_start());
        }
    }
    None
}

fn strip_trailing_ai_mode_chrome(mut text: &str) -> &str {
    text = text.trim_end();

    // Unambiguous Google AI Mode bottom toolbar / disclaimer markers: truncate at earliest match.
    const FOOTER_BOUNDARY_MARKERS: &[&str] = &[
        "skip to previous prompt",
        "add files, tools, and select a model",
        "transcribing...add files",
        "microphonestopretry",
        "stopretrysend",
        "stop retry send",
        "sendask about",
        "ai responses may include mistakes",
        "ai can make mistakes, so double-check responses",
        "ai can make mistakes",
    ];

    let lower = text.to_ascii_lowercase();
    let mut cut_at: Option<usize> = None;
    for marker in FOOTER_BOUNDARY_MARKERS {
        if let Some(idx) = lower.find(marker) {
            cut_at = Some(cut_at.map_or(idx, |prev| prev.min(idx)));
        }
    }

    // "Transcribing..." followed by toolbar items or within the tail of the text is also UI chrome.
    if let Some(idx) = lower.rfind("transcribing...") {
        let tail = &lower[idx..];
        if tail.len() <= 240
            || tail.contains("add files")
            || tail.contains("microphone")
            || tail.contains("ask about")
            || tail.contains("retry")
            || tail.contains("send")
        {
            cut_at = Some(cut_at.map_or(idx, |prev| prev.min(idx)));
        }
    }

    if let Some(idx) = cut_at {
        text = text[..idx].trim_end();
    }

    // Iteratively strip trailing standalone boilerplate lines and exact concatenated UI suffixes.
    const TRAILING_BOILERPLATE_LINES: &[&str] = &[
        "ai mode",
        "ai responses may include mistakes. learn more",
        "ai responses may include mistakes.",
        "ai responses may include mistakes",
        "ai can make mistakes, so double-check responses",
        "ai can make mistakes",
        "learn more",
        "ask anything",
        "ask about",
        "create a follow-up",
        "ask a follow-up",
        "sources",
        "show more",
        "copy",
        "share",
        "thank you for your feedback",
        "skip to previous prompt",
        "transcribing...",
        "transcribing",
        "add files, tools, and select a model",
        "add files, tools",
        "microphone",
        "stop",
        "retry",
        "send",
    ];

    const TRAILING_CONCAT_SUFFIXES: &[&str] = &[
        "Skip to previous prompt",
        "Add files, tools, and select a model",
        "Add files, tools",
        "Transcribing...",
        "Create a follow-up",
        "Ask a follow-up",
        "Ask anything",
        "Ask about",
        "Thank you for your feedback",
        "StopRetrySend",
        "RetrySend",
        "SendSend",
    ];

    loop {
        let trimmed = text.trim_end();
        let mut changed = false;

        for suffix in TRAILING_CONCAT_SUFFIXES {
            if let Some(before) = trimmed.strip_suffix(suffix) {
                text = before;
                changed = true;
                break;
            }
        }
        if changed {
            continue;
        }

        if let Some(last_nl) = trimmed.rfind('\n') {
            let last_line = trimmed[last_nl + 1..].trim().to_ascii_lowercase();
            if TRAILING_BOILERPLATE_LINES.contains(&last_line.as_str()) {
                text = &trimmed[..last_nl];
                continue;
            }
        } else {
            let only_line = trimmed.trim().to_ascii_lowercase();
            if TRAILING_BOILERPLATE_LINES.contains(&only_line.as_str()) {
                text = "";
                continue;
            }
        }

        text = trimmed;
        break;
    }

    text.trim()
}

/// Clean extracted Google Search AI Mode DOM text by isolating the assistant reply
/// after any echoed user/system prompt and stripping leading/trailing UI chrome.
pub fn clean_ai_mode_extracted_text(raw: &str, submitted_prompt: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let without_leading_chrome = strip_leading_ai_mode_chrome(&normalized);
    let after_prompt = isolate_after_echoed_prompt(without_leading_chrome, submitted_prompt);
    let without_leading_after_prompt = strip_leading_ai_mode_chrome(after_prompt);
    let without_trailing_chrome = strip_trailing_ai_mode_chrome(without_leading_after_prompt);

    let mut out = String::with_capacity(without_trailing_chrome.len());
    let mut consecutive_empty = 0usize;
    for line in without_trailing_chrome.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            consecutive_empty = 0;
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(trimmed_end);
        }
    }

    out.trim().to_string()
}

struct AiModeSnapshot {
    url: String,
    page_text: String,
    raw_response_text: String,
    is_generating: bool,
}

async fn extract_ai_mode_response(
    session: &mut CdpBrowserSession,
) -> Result<AiModeSnapshot, String> {
    let extract_js = r#"(() => {
        const url = location.href || '';
        const bodyText = (document.body && document.body.innerText) ? document.body.innerText : '';

        const isVisible = (el) => {
            if (!el) return false;
            const st = window.getComputedStyle(el);
            if (!st || st.display === 'none' || st.visibility === 'hidden' || st.opacity === '0') return false;
            if (st.display === 'contents') return true;
            const r = el.getBoundingClientRect();
            if (r.width <= 0 || r.height <= 0) return false;
            if (r.width <= 1 && r.height <= 1 && st.overflow === 'hidden') return false;
            return true;
        };

        const hasVisibleActiveMatch = (selector) => {
            for (const el of document.querySelectorAll(selector)) {
                if (isVisible(el) && !el.disabled && el.getAttribute('aria-disabled') !== 'true') {
                    return true;
                }
            }
            return false;
        };

        const hasVisibleStopButton = () => {
            for (const el of document.querySelectorAll('button, [role="button"]')) {
                if (!isVisible(el) || el.disabled || el.getAttribute('aria-disabled') === 'true') continue;
                const label = (el.getAttribute('aria-label') || '').trim().toLowerCase();
                const txt = (el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim().toLowerCase();
                if (
                    label.includes('stop generating') ||
                    label.includes('stop response') ||
                    label === 'stop' ||
                    txt === 'stop' ||
                    txt === 'stop generating' ||
                    txt === 'stop response'
                ) {
                    return true;
                }
            }
            return false;
        };

        const isGenerating =
            hasVisibleActiveMatch('[aria-busy="true"]') ||
            hasVisibleActiveMatch('button[aria-label*="Stop" i]') ||
            hasVisibleActiveMatch('[aria-label*="Stop generating" i]') ||
            hasVisibleActiveMatch('[role="button"][aria-label*="Stop" i]') ||
            hasVisibleActiveMatch('main [role="progressbar"], [role="main"] [role="progressbar"], [data-container-id="main-col"] [role="progressbar"]') ||
            hasVisibleStopButton();

        const excludedTags = new Set([
            'HEADER', 'NAV', 'FOOTER', 'FORM', 'ASIDE', 'TEXTAREA', 'INPUT',
            'SELECT', 'BUTTON', 'SCRIPT', 'STYLE', 'NOSCRIPT', 'SVG', 'TEMPLATE'
        ]);
        const excludedRoles = new Set([
            'navigation', 'banner', 'contentinfo', 'complementary',
            'search', 'tablist', 'toolbar', 'menu', 'menubar', 'dialog', 'button'
        ]);
        const blockTags = new Set([
            'ADDRESS', 'ARTICLE', 'ASIDE', 'BLOCKQUOTE', 'DD', 'DETAILS', 'DIV',
            'DL', 'DT', 'FIELDSET', 'FIGCAPTION', 'FIGURE', 'FOOTER', 'FORM',
            'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'HEADER', 'HR', 'LI', 'MAIN',
            'NAV', 'OL', 'P', 'PRE', 'SECTION', 'TABLE', 'TR', 'UL'
        ]);

        const isChromeElement = (el) => {
            if (excludedTags.has(el.tagName)) return true;
            const role = (el.getAttribute('role') || '').toLowerCase();
            if (excludedRoles.has(role)) return true;
            if (el.getAttribute('aria-hidden') === 'true') return true;
            const ce = el.getAttribute('contenteditable');
            if (el.isContentEditable || ce === '' || ce === 'true') return true;
            if (el.matches('#searchform, #hdtb, #top_nav, #footcnt, #fbar, [aria-label*="feedback" i], [aria-label*="sidebar" i], [aria-label*="New thread" i], [aria-label*="Search history" i], [data-ved][role="button"]')) {
                return true;
            }
            return false;
        };

        const extractCleanNodeText = (node) => {
            if (!node) return '';
            if (node.nodeType === Node.TEXT_NODE) {
                return node.nodeValue || '';
            }
            if (node.nodeType !== Node.ELEMENT_NODE) {
                return '';
            }
            const el = node;
            if (isChromeElement(el) || !isVisible(el)) return '';
            if (el.tagName === 'BR') return '\n';

            let out = '';
            for (const child of el.childNodes) {
                out += extractCleanNodeText(child);
            }
            if (!out) return '';

            if (el.tagName === 'TD' || el.tagName === 'TH') {
                if (!out.endsWith(' ') && !out.endsWith('\n')) out += ' ';
                return out;
            }

            const st = window.getComputedStyle(el);
            const isBlock = blockTags.has(el.tagName) || (st && (
                st.display === 'block' || st.display === 'flex' ||
                st.display === 'grid' || st.display === 'list-item' ||
                st.display === 'table' || st.display === 'table-row'
            ));
            if (isBlock) {
                if (!out.startsWith('\n')) out = '\n' + out;
                if (!out.endsWith('\n')) out += '\n';
            }
            return out;
        };

        const candidateSelectors = [
            '[data-container-id="main-col"]',
            'main',
            '[role="main"]',
            '#center_col',
            '#rcnt'
        ];

        let rawResponseText = '';
        for (const sel of candidateSelectors) {
            const nodes = Array.from(document.querySelectorAll(sel)).filter(isVisible);
            if (nodes.length === 0) continue;
            const combined = nodes
                .map((n) => extractCleanNodeText(n))
                .map((t) => t.replace(/\r\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim())
                .filter((t) => t.length > 0)
                .join('\n\n')
                .trim();
            if (combined.length > 0) {
                rawResponseText = combined;
                break;
            }
        }

        if (!rawResponseText && document.body) {
            rawResponseText = extractCleanNodeText(document.body)
                .replace(/\r\n/g, '\n')
                .replace(/\n{3,}/g, '\n\n')
                .trim();
        }

        return {
            url,
            pageText: bodyText,
            rawResponseText,
            isGenerating
        };
    })()"#;

    let snapshot = session.eval_js(extract_js).await?;
    Ok(AiModeSnapshot {
        url: snapshot
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        page_text: snapshot
            .get("pageText")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        raw_response_text: snapshot
            .get("rawResponseText")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        is_generating: snapshot
            .get("isGenerating")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

fn safe_streamable_prefix(clean_reply: &str, allow_tool_calls: bool) -> &str {
    if !allow_tool_calls {
        return clean_reply;
    }
    let lower = clean_reply.to_ascii_lowercase();
    if let Some(tag_idx) = lower.find("<harness_tool_call") {
        return clean_reply[..tag_idx].trim_end();
    }
    // Hold back a trailing partial "<harness_tool_call..." prefix while streaming.
    if let Some(lt_idx) = lower.rfind('<') {
        let suffix = &lower[lt_idx..];
        if "<harness_tool_call>".starts_with(suffix) {
            return clean_reply[..lt_idx].trim_end();
        }
    }
    clean_reply
}

async fn poll_ai_mode_response(
    session: &mut CdpBrowserSession,
    prompt: &str,
    allow_tool_calls: bool,
    event_tx: &mpsc::Sender<HarnessEvent>,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(RESPONSE_TIMEOUT_SECS);
    let mut last_text = String::new();
    let mut emitted_text = String::new();
    let mut stable_polls: u8 = 0;

    while Instant::now() < deadline {
        session.pump_events(RESPONSE_POLL_INTERVAL_MS).await;

        let snapshot = extract_ai_mode_response(session).await?;

        if let Some(reason) = detect_google_failed_request_block(session.failed_requests())
            .or_else(|| detect_google_captcha_or_block(&snapshot.url, &snapshot.page_text))
        {
            return Err(format!(
                "Google Search AI Mode presented a CAPTCHA/verification gate and cannot be bypassed: {}",
                reason
            ));
        }

        let clean_reply = clean_ai_mode_extracted_text(&snapshot.raw_response_text, prompt);

        if clean_reply.is_empty() || is_placeholder_status_text(&clean_reply) {
            stable_polls = 0;
            continue;
        }

        // Stream safe incremental deltas only for the clean assistant reply.
        let streamable_slice = safe_streamable_prefix(&clean_reply, allow_tool_calls);

        if streamable_slice.len() > emitted_text.len()
            && streamable_slice.starts_with(&emitted_text)
        {
            let delta = streamable_slice[emitted_text.len()..].to_string();
            if !delta.is_empty() {
                let _ = event_tx.send(HarnessEvent::MessageDelta { delta }).await;
                emitted_text = streamable_slice.to_string();
            }
        }

        if clean_reply == last_text && !snapshot.is_generating {
            stable_polls = stable_polls.saturating_add(1);
            if stable_polls >= REQUIRED_STABLE_POLLS {
                if emitted_text.is_empty() && !streamable_slice.is_empty() {
                    let _ = event_tx
                        .send(HarnessEvent::MessageDelta {
                            delta: streamable_slice.to_string(),
                        })
                        .await;
                }
                return Ok(clean_reply);
            }
        } else {
            last_text = clean_reply;
            stable_polls = 0;
        }
    }

    if !last_text.is_empty() && !is_placeholder_status_text(&last_text) {
        if emitted_text.is_empty() {
            let (parsed_content, _) = parse_google_search_response(&last_text, allow_tool_calls);
            if let Some(text) = parsed_content {
                let _ = event_tx
                    .send(HarnessEvent::MessageDelta { delta: text })
                    .await;
            }
        }
        return Ok(last_text);
    }

    Err(format!(
        "Google Search AI Mode timed out after {}s without returning a response.",
        RESPONSE_TIMEOUT_SECS
    ))
}

async fn run_google_search_session(
    session: &mut CdpBrowserSession,
    prompt: &str,
    allow_tool_calls: bool,
    event_tx: &mpsc::Sender<HarnessEvent>,
) -> Result<String, String> {
    session.enable_default_domains(1280, 900).await?;
    let _ = session
        .call("Emulation.setFocusEmulationEnabled", json!({ "enabled": true }))
        .await;
    session.navigate(GOOGLE_AI_MODE_URL, 1200).await?;
    check_captcha_gate(session).await?;

    dismiss_cookie_consent_if_present(session).await?;
    check_captcha_gate(session).await?;

    ensure_ai_mode_input_ready(session).await?;
    fill_and_submit_prompt(session, prompt).await?;
    poll_ai_mode_response(session, prompt, allow_tool_calls, event_tx).await
}

#[async_trait]
impl ModelAdapter for GoogleSearchClient {
    async fn stream_completion(
        &self,
        messages: &[ChatMessage],
        tools: &Value,
        event_tx: &mpsc::Sender<HarnessEvent>,
    ) -> Result<ModelTurnResult, String> {
        let browser_path = discover_chrome_browser().ok_or_else(|| {
            "Google-Search-Model requires Chrome or Edge installed locally, but no supported browser binary was found.".to_string()
        })?;

        let prompt = build_google_search_prompt(messages, tools);
        let allow_tool_calls = tools
            .as_array()
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);

        let mut session = CdpBrowserSession::launch_incognito(&browser_path, 1280, 900).await?;
        let session_result =
            run_google_search_session(&mut session, &prompt, allow_tool_calls, event_tx).await;
        session.close().await;

        let raw_response = session_result?;
        let (content, tool_calls) = parse_google_search_response(&raw_response, allow_tool_calls);

        if content.is_none() && tool_calls.is_empty() {
            return Err("Google Search AI Mode returned an empty response.".to_string());
        }

        let prompt_tokens: usize = messages.iter().map(estimate_message_tokens).sum();
        let completion_msg = ChatMessage::assistant(
            content.clone(),
            None,
            (!tool_calls.is_empty()).then(|| tool_calls.clone()),
        );
        let completion_tokens = estimate_message_tokens(&completion_msg);

        let finish_reason = if tool_calls.is_empty() {
            Some("stop".to_string())
        } else {
            Some("tool_calls".to_string())
        };

        Ok(ModelTurnResult {
            content,
            reasoning_content: None,
            tool_calls,
            finish_reason,
            estimated_tokens: prompt_tokens + completion_tokens,
            prompt_tokens,
            completion_tokens,
            cached_tokens: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_google_search_model() {
        assert!(is_google_search_model("Google-Search-Model"));
        assert!(is_google_search_model("google-search-model"));
        assert!(is_google_search_model("  experimental/google-search-model "));
        assert!(is_google_search_model("EXPERIMENTAL/GOOGLE-SEARCH-MODEL"));
        assert!(!is_google_search_model("gpt-4o"));
        assert!(!is_google_search_model("gemini-2.5-flash"));
    }

    #[test]
    fn test_build_prompt_wraps_system_prompts_and_tools() {
        let messages = vec![
            ChatMessage::system("You are a strict Rust engineer."),
            ChatMessage::system("Always respond with concise facts."),
            ChatMessage::user("What is Cargo.toml?"),
            ChatMessage::assistant(Some("It is the Rust package manifest.".into()), None, None),
            ChatMessage::user("How do I add serde?"),
        ];
        let tools = json!([
            {
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a file from disk",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }
                }
            }
        ]);

        let prompt = build_google_search_prompt(&messages, &tools);

        assert!(prompt.contains("--- BEGIN SYSTEM PROMPT ---"));
        assert!(prompt.contains("--- END SYSTEM PROMPT ---"));
        assert!(prompt.contains("authoritative System Prompt"));
        assert!(prompt.contains("You are a strict Rust engineer."));
        assert!(prompt.contains("Always respond with concise facts."));
        assert!(prompt.contains("- read_file(path: string): Read a file from disk"));
        assert!(prompt.contains("<harness_tool_call>"));
        assert!(prompt.contains("[CONVERSATION HISTORY]"));
        assert!(prompt.contains("User: What is Cargo.toml?"));
        assert!(prompt.contains("Assistant: It is the Rust package manifest."));
        assert!(prompt.contains("User Request:\nHow do I add serde?"));
    }

    #[test]
    fn test_build_prompt_compacts_verbose_tool_schemas() {
        let huge_steps_schema = json!({
            "type": "array",
            "description": "Ordered browser automation steps ".repeat(40),
            "items": {
                "type": "object",
                "properties": {
                    "interaction": {
                        "type": "string",
                        "enum": [
                            "click", "dblclick", "fill", "type", "press_key",
                            "select_option", "hover", "wait_for_selector",
                            "wait_ms", "scroll_by", "scroll_to", "eval",
                            "screenshot", "assert_text", "assert_selector"
                        ]
                    },
                    "selector": { "type": "string" },
                    "ref_id": { "type": "integer" },
                    "text": { "type": "string" },
                    "value": { "type": "string" },
                    "extra_payload": { "type": "string", "description": "verbose ".repeat(100) }
                }
            }
        });
        let tools = json!([
            {
                "type": "function",
                "function": {
                    "name": "browser_ui",
                    "description": format!("Automate browser actions and inspect live DOM/console state. {}", "Detail ".repeat(80)),
                    "parameters": {
                        "type": "object",
                        "required": ["action"],
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": ["navigate", "screenshot", "eval", "interact"]
                            },
                            "url": { "type": "string" },
                            "selector": { "type": "string" },
                            "steps": huge_steps_schema,
                            "timeout_ms": { "type": "integer" },
                            "full_page": { "type": "boolean" },
                            "session_id": { "type": "string" },
                            "wait_until": { "type": "string" }
                        }
                    }
                }
            }
        ]);

        let raw_schema_len = tools.to_string().len();
        assert!(raw_schema_len > 2000);

        let messages = vec![
            ChatMessage::system("System rule one."),
            ChatMessage::system("System rule two."),
            ChatMessage::user("Open index.html and check console errors."),
        ];
        let prompt = build_google_search_prompt(&messages, &tools);

        assert!(prompt.contains("System rule one."));
        assert!(prompt.contains("System rule two."));
        assert!(prompt.contains("- browser_ui(action: navigate|screenshot|eval|interact,"));
        assert!(!prompt.contains("\"extra_payload\""));
        assert!(!prompt.contains("wait_for_selector"));
        assert!(prompt.len() < raw_schema_len);
    }

    #[test]
    fn test_build_prompt_handles_tool_continuation_turn() {
        let messages = vec![
            ChatMessage::system("Be helpful."),
            ChatMessage::user("Check Cargo.toml"),
            ChatMessage::assistant(
                None,
                None,
                Some(vec![ToolCall {
                    id: "call_gs_1".into(),
                    r#type: "function".into(),
                    function: ToolFunctionCall {
                        name: "read_file".into(),
                        arguments: "{\"path\":\"Cargo.toml\"}".into(),
                    },
                }]),
            ),
            ChatMessage::tool("call_gs_1", "[package]\nname = \"demo\""),
        ];

        let prompt = build_google_search_prompt(&messages, &json!([]));
        assert!(prompt.contains("[CONVERSATION HISTORY]"));
        assert!(prompt.contains("Tool Result (call_gs_1): [package]\nname = \"demo\""));
        assert!(prompt.contains("User Request:"));
    }

    #[test]
    fn test_build_prompt_respects_ai_mode_maxlength_budget() {
        let huge_sys = "Rule line. ".repeat(1200);
        let messages = vec![
            ChatMessage::system(&huge_sys),
            ChatMessage::user("Short user question?"),
        ];
        let prompt = build_google_search_prompt(&messages, &json!([]));
        assert!(prompt.chars().count() <= MAX_AI_MODE_PROMPT_CHARS);
        assert!(prompt.contains("User Request:\nShort user question?"));
    }

    #[test]
    fn test_parse_google_search_response_with_tools() {
        let raw = "Let me check that file first.\n<harness_tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"src/main.rs\"}}</harness_tool_call>\nThen I will explain it.";
        let (content, calls) = parse_google_search_response(raw, true);

        assert_eq!(
            content.as_deref(),
            Some("Let me check that file first.\n\nThen I will explain it.")
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_gs_1");
        assert_eq!(calls[0].r#type, "function");
        assert_eq!(calls[0].function.name, "read_file");
        let parsed_args: Value = serde_json::from_str(&calls[0].function.arguments).unwrap();
        assert_eq!(parsed_args, json!({"path": "src/main.rs"}));
    }

    #[test]
    fn test_parse_google_search_response_fenced_and_multiple_calls() {
        let raw = r#"<harness_tool_call>```json
{"name": "sys_info", "arguments": {}}
```</harness_tool_call>
<harness_tool_call>{"name": "list_dir", "arguments": "{\"path\": \".\"}"}</harness_tool_call>"#;
        let (content, calls) = parse_google_search_response(raw, true);
        assert_eq!(content, None);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "sys_info");
        assert_eq!(calls[0].function.arguments, "{}");
        assert_eq!(calls[1].function.name, "list_dir");
        assert_eq!(calls[1].function.arguments, "{\"path\": \".\"}");
    }

    #[test]
    fn test_parse_google_search_response_disallows_tools_when_disabled() {
        let raw = "Answer: <harness_tool_call>{\"name\": \"read_file\", \"arguments\": {}}</harness_tool_call>";
        let (content, calls) = parse_google_search_response(raw, false);
        assert!(calls.is_empty());
        assert_eq!(content.as_deref(), Some(raw));
    }

    #[test]
    fn test_detect_google_captcha_or_block() {
        assert!(detect_google_captcha_or_block(
            "https://www.google.com/sorry/index?continue=https://www.google.com/search",
            "About this page"
        )
        .is_some());

        assert!(detect_google_captcha_or_block(
            "https://www.google.com/search?udm=50",
            "Our systems have detected unusual traffic from your computer network."
        )
        .is_some());

        assert!(detect_google_captcha_or_block(
            "https://www.google.com/search?udm=50",
            "Please show you're not a robot to continue."
        )
        .is_some());

        // Normal AI Mode response mentioning the word "captcha" conceptually should not false-positive
        assert!(detect_google_captcha_or_block(
            "https://www.google.com/search?udm=50&hl=en",
            "A CAPTCHA is a challenge-response test used in computing."
        )
        .is_none());

        assert!(detect_google_failed_request_block(&[json!({
            "url": "https://www.google.com/sorry/index?continue=https://www.google.com/async/folif",
            "status": 429
        })])
        .is_some());

        assert!(detect_google_failed_request_block(&[json!({
            "url": "https://www.google.com/async/folif?ei=abc",
            "status": 429
        })])
        .is_some());

        assert!(detect_google_failed_request_block(&[json!({
            "url": "https://example.com/favicon.ico",
            "status": 404
        })])
        .is_none());
    }

    #[test]
    fn test_clean_ai_mode_extracted_text_strips_noisy_dom_trace() {
        let messages = vec![
            ChatMessage::system("Answer in one clear sentence."),
            ChatMessage::user("What is the capital of France?"),
        ];
        let submitted_prompt = build_google_search_prompt(&messages, &json!([]));

        // Simulate the exact ses_1794676800555 noisy DOM extraction with collapsed whitespace
        let collapsed_prompt = submitted_prompt
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let noisy_dom_trace = format!(
            "Open sidebarNew threadSee your Search historyPersonalizationConnected Content AppsSettingsAI Mode conversation: {} Paris is the capital of France. Skip to previous promptTranscribing...Add files, tools, and select a modelMicrophoneStopRetrySendSendAsk about",
            collapsed_prompt
        );

        let cleaned = clean_ai_mode_extracted_text(&noisy_dom_trace, &submitted_prompt);
        assert_eq!(cleaned, "Paris is the capital of France.");
        assert!(!cleaned.contains("Open sidebar"));
        assert!(!cleaned.contains("CRITICAL SYSTEM INSTRUCTION"));
        assert!(!cleaned.contains("Skip to previous prompt"));
        assert!(!cleaned.contains("Transcribing..."));
        assert!(!cleaned.contains("Ask about"));
    }

    #[test]
    fn test_clean_ai_mode_extracted_text_multiline_and_footer_disclaimer() {
        let messages = vec![
            ChatMessage::system("You are a Rust helper."),
            ChatMessage::user("Show a hello world function."),
        ];
        let submitted_prompt = build_google_search_prompt(&messages, &json!([]));

        let raw = format!(
            "Open sidebar\nNew thread\nAI Mode conversation:\n{}\nHere is the code:\n\n```rust\nfn main() {{ println!(\"hi\"); }}\n```\nCopy\nShare\nAI responses may include mistakes. Learn more\nSkip to previous prompt\nTranscribing...\nAsk about",
            submitted_prompt
        );

        let cleaned = clean_ai_mode_extracted_text(&raw, &submitted_prompt);
        assert_eq!(
            cleaned,
            "Here is the code:\n\n```rust\nfn main() { println!(\"hi\"); }\n```"
        );
    }

    #[test]
    fn test_clean_ai_mode_extracted_text_returns_empty_when_only_chrome_or_prompt() {
        let messages = vec![
            ChatMessage::system("Be concise."),
            ChatMessage::user("Explain async Rust."),
        ];
        let submitted_prompt = build_google_search_prompt(&messages, &json!([]));

        let only_chrome = "Open sidebarNew threadSee your Search historyPersonalizationConnected Content AppsSettingsAI Mode conversation:";
        assert_eq!(
            clean_ai_mode_extracted_text(only_chrome, &submitted_prompt),
            ""
        );

        let only_echoed_prompt_and_toolbar = format!(
            "Open sidebarNew threadSee your Search historyPersonalizationConnected Content AppsSettingsAI Mode conversation: {} Skip to previous promptTranscribing...Add files, tools, and select a modelMicrophoneStopRetrySendSendAsk about",
            submitted_prompt
        );
        assert_eq!(
            clean_ai_mode_extracted_text(&only_echoed_prompt_and_toolbar, &submitted_prompt),
            ""
        );

        let incomplete_prompt_echo =
            "Open sidebarAI Mode conversation: CRITICAL SYSTEM INSTRUCTION: You must follow <system_prompt>...";
        assert_eq!(
            clean_ai_mode_extracted_text(incomplete_prompt_echo, &submitted_prompt),
            ""
        );
    }

    #[test]
    fn test_clean_ai_mode_extracted_text_fallback_and_clean_passthrough() {
        // Fallback when submitted_prompt isn't matched verbatim, but prompt delimiters exist
        let raw_with_delimiters = "Open sidebarNew threadAI Mode conversation: CRITICAL SYSTEM INSTRUCTION: follow rules <system_prompt>Be brief.</system_prompt>[END SYSTEM PROMPT OVERRIDE]\nThe answer is 42.\nSkip to previous promptTranscribing...Ask about";
        assert_eq!(
            clean_ai_mode_extracted_text(raw_with_delimiters, "unmatched prompt"),
            "The answer is 42."
        );

        // Clean reply without any echoed prompt or chrome passes through untouched
        let clean = "Ownership guarantees memory safety without a garbage collector.";
        assert_eq!(
            clean_ai_mode_extracted_text(clean, "What is Rust ownership?"),
            clean
        );
    }

    #[test]
    fn test_safe_streamable_prefix_holds_back_tool_tags() {
        assert_eq!(
            safe_streamable_prefix("Checking file now.\n<harness_tool_call>{\"name\":\"read_file\"}", true),
            "Checking file now."
        );
        assert_eq!(
            safe_streamable_prefix("Checking file now. <harness_to", true),
            "Checking file now."
        );
        assert_eq!(
            safe_streamable_prefix("Plain answer <not_a_tool>", false),
            "Plain answer <not_a_tool>"
        );
    }
}
