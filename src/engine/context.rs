use crate::config::MemoryConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

const WORKING_MEMORY_PREFIX: &str = "[COMPRESSED WORKING MEMORY]";
const ALT_WORKING_MEMORY_PREFIX: &str = "[COMPACTED WORKING MEMORY]";
const WORKING_MEMORY_MAX_CHARS: usize = 1200;
const SUPERSEDED_TOMBSTONE: &str = "[Superseded by later identical call]";
const MESSAGE_TOKEN_OVERHEAD: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ToolFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatAttachment {
    /// "image" | "video" | "audio" | "file"
    pub kind: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub size_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
}

/// Render text/code file attachments (and optional fallback stubs for media) cleanly as plain text.
pub fn format_attachments_as_text(attachments: &[ChatAttachment], include_media_stubs: bool) -> String {
    let mut blocks = Vec::new();
    for att in attachments {
        if let Some(ref txt) = att.text_content {
            if !txt.is_empty() {
                blocks.push(format!(
                    "<attached_file name=\"{}\" mime=\"{}\">\n{}\n</attached_file>",
                    att.name, att.mime_type, txt
                ));
                continue;
            }
        }
        if include_media_stubs {
            blocks.push(format!(
                "[Attached {}: {} ({}, {} bytes)]",
                att.kind, att.name, att.mime_type, att.size_bytes
            ));
        }
    }
    blocks.join("\n\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ChatAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            attachments: Vec::new(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            attachments: Vec::new(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user_with_attachments(
        content: impl Into<String>,
        attachments: Vec<ChatAttachment>,
    ) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            attachments,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(
        content: Option<String>,
        reasoning: Option<String>,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content,
            attachments: Vec::new(),
            reasoning_content: reasoning,
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            attachments: Vec::new(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompactionReport {
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub messages_folded: usize,
    pub pruned_outputs: usize,
}

#[derive(Debug, Clone)]
pub struct ConversationContext {
    pub messages: Vec<ChatMessage>,
    pub max_tool_output_bytes: usize,
    pub max_context_chars: usize,
    pub memory_config: MemoryConfig,
    pub compactions_count: usize,
}

impl ConversationContext {
    pub fn new(max_tool_output_bytes: usize) -> Self {
        Self {
            messages: Vec::with_capacity(32),
            max_tool_output_bytes,
            max_context_chars: 128 * 1024, // 128KB fallback budget
            memory_config: MemoryConfig::default(),
            compactions_count: 0,
        }
    }

    /// Reconstruct a `ConversationContext` from saved prior messages for multi-turn continuation or forking.
    pub fn from_messages(messages: Vec<ChatMessage>, max_tool_output_bytes: usize) -> Self {
        Self {
            messages,
            max_tool_output_bytes,
            max_context_chars: 128 * 1024,
            memory_config: MemoryConfig::default(),
            compactions_count: 0,
        }
    }

    pub fn with_memory_config(mut self, mem: MemoryConfig) -> Self {
        self.memory_config = mem;
        self
    }

    pub fn push(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
        let _ = self.compact_if_needed();
    }

    pub fn truncate_tool_output(&self, raw: &str) -> (String, bool) {
        // Guard against a 0/1-byte limit: `half = 0` would emit the whole payload
        // plus a truncation notice, exceeding the limit it is meant to enforce.
        let limit = self.max_tool_output_bytes.max(64);
        if raw.len() <= limit {
            return (raw.to_string(), false);
        }

        let half = (limit / 2).max(1);
        let head_end = floor_char_boundary(raw, half.min(raw.len()));
        let tail_start = ceil_char_boundary(raw, raw.len().saturating_sub(half));
        let head = &raw[..head_end];
        let tail = &raw[tail_start..];
        let omitted = raw.len().saturating_sub(head.len() + tail.len());

        let compacted = format!(
            "{}\n\n[... Harness notice: {} bytes truncated for resource efficiency ...]\n\n{}",
            head, omitted, tail
        );
        (compacted, true)
    }

    /// Automatic two-tier compaction triggered when estimated tokens exceed the configured threshold.
    pub fn compact_if_needed(&mut self) -> Option<CompactionReport> {
        self.run_compaction(false)
    }

    /// Force two-tier compaction (used by manual compact endpoint or explicit memory pressure handling).
    pub fn force_compact(&mut self) -> Option<CompactionReport> {
        self.run_compaction(true)
    }

    fn run_compaction(&mut self, force: bool) -> Option<CompactionReport> {
        let report = compact_messages_with_config(&mut self.messages, &self.memory_config, force);
        if report.is_some() {
            self.compactions_count += 1;
        }
        report
    }

    /// Estimate token usage across all messages using char/byte-aware weighting (~3.5 chars/token + overhead).
    pub fn estimate_tokens(&self) -> usize {
        estimate_messages_tokens(&self.messages)
    }
}

fn trigger_tokens_for(memory_config: &MemoryConfig) -> usize {
    let max_tokens = memory_config.max_context_tokens.max(1);
    let pct = (memory_config.auto_compact_threshold_pct as usize).clamp(1, 100);
    (max_tokens.saturating_mul(pct) / 100).max(1)
}

/// Compact a message vector in place according to `memory_config` using tiered pruning and pair-safe folding.
pub fn compact_messages_with_config(
    messages: &mut Vec<ChatMessage>,
    memory_config: &MemoryConfig,
    force: bool,
) -> Option<CompactionReport> {
    let before_tokens = estimate_messages_tokens(messages);
    let trigger_tokens = trigger_tokens_for(memory_config);
    if !force && before_tokens <= trigger_tokens {
        return None;
    }
    if messages.is_empty() {
        return None;
    }

    let start_idx = if messages
        .first()
        .is_some_and(|m| m.role == "system" && !is_working_memory_msg(m))
    {
        1
    } else {
        0
    };

    let preserve = memory_config.preserve_recent_messages;
    let mut recent_start = if preserve == 0 {
        messages.len()
    } else {
        messages.len().saturating_sub(preserve).max(start_idx)
    };

    // Pair-safe boundary: never start the recent window on a `tool` message without its preceding `assistant` `tool_calls`.
    while recent_start > start_idx
        && recent_start < messages.len()
        && messages[recent_start].role == "tool"
    {
        recent_start -= 1;
    }

    let mut pruned_outputs = 0usize;
    let prune_limit = memory_config.prune_tool_outputs_over_bytes;

    // Tier 0: clear old reasoning traces in the prunable zone.
    for msg in &mut messages[start_idx..recent_start] {
        if msg.reasoning_content.is_some() {
            msg.reasoning_content = None;
        }
    }

    // Tier 1: deduplicate stale repeated read-only tool calls superseded by a later identical call.
    let mut call_info_by_id: HashMap<String, (String, String, usize)> = HashMap::new();
    let mut latest_readonly_seq: HashMap<(String, String), usize> = HashMap::new();
    let mut seq_order = 0usize;

    for msg in &messages[start_idx..] {
        if msg.role == "assistant" {
            if let Some(ref tcs) = msg.tool_calls {
                for tc in tcs {
                    seq_order += 1;
                    if is_deduplicatable_readonly_call(&tc.function.name, &tc.function.arguments) {
                        let canon = canonical_tool_args(&tc.function.arguments);
                        let key = (tc.function.name.clone(), canon.clone());
                        latest_readonly_seq.insert(key, seq_order);
                        call_info_by_id
                            .insert(tc.id.clone(), (tc.function.name.clone(), canon, seq_order));
                    }
                }
            }
        }
    }

    for msg in &mut messages[start_idx..recent_start] {
        if msg.role != "tool" {
            continue;
        }
        if let Some(call_id) = msg.tool_call_id.as_deref() {
            if let Some((name, canon_args, call_seq)) = call_info_by_id.get(call_id) {
                if latest_readonly_seq
                    .get(&(name.clone(), canon_args.clone()))
                    .is_some_and(|&latest| latest > *call_seq)
                {
                    if let Some(ref mut c) = msg.content {
                        if c != SUPERSEDED_TOMBSTONE && c.len() > SUPERSEDED_TOMBSTONE.len() {
                            *c = SUPERSEDED_TOMBSTONE.to_string();
                            pruned_outputs += 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Tier 2: smart line-aware head+tail pruning for oversized older tool outputs.
        if let Some(ref mut c) = msg.content {
            if c != SUPERSEDED_TOMBSTONE && c.len() > prune_limit {
                if let Some(compacted) = smart_prune_tool_output(c, prune_limit) {
                    *c = compacted;
                    pruned_outputs += 1;
                }
            }
        }
    }

    // Stage 2 (Merge working memory + pair-safe atomic group fold).
    let post_stage1_tokens = estimate_messages_tokens(messages);
    let mut existing_wms: Vec<String> = Vec::new();
    let mut middle_non_wm: Vec<ChatMessage> = Vec::new();
    for msg in &messages[start_idx..recent_start] {
        if is_working_memory_msg(msg) {
            if let Some(ref c) = msg.content {
                existing_wms.push(c.clone());
            }
        } else {
            middle_non_wm.push(msg.clone());
        }
    }

    let merged_existing_wm = if existing_wms.is_empty() {
        None
    } else {
        Some(existing_wms.join("\n"))
    };

    let groups = partition_atomic_groups(&middle_non_wm);
    let mut messages_folded = 0usize;

    let should_fold = !groups.is_empty()
        && (post_stage1_tokens > trigger_tokens || (force && groups.len() > 2));
    let should_merge_redundant_wm = existing_wms.len() > 1;

    if should_fold {
        let min_fold = if force && groups.len() > 2 {
            groups.len() - 2
        } else {
            1
        };

        let prefix_slice = &messages[..start_idx];
        let suffix_slice = &messages[recent_start..];
        let mut fold_count = min_fold.min(groups.len());

        while fold_count <= groups.len() {
            let candidate_wm = if memory_config.enable_working_memory {
                Some(ChatMessage::user(build_working_memory_block(
                    merged_existing_wm.as_deref(),
                    &groups[..fold_count],
                    memory_config.summarize_compacted_turns,
                )))
            } else {
                None
            };

            let est = estimate_tokens_parts(
                prefix_slice,
                candidate_wm.as_ref(),
                &groups[fold_count..],
                suffix_slice,
            );
            if est <= trigger_tokens || fold_count == groups.len() {
                break;
            }
            fold_count += 1;
        }

        messages_folded = groups[..fold_count].iter().map(|g| g.len()).sum();

        let mut rebuilt = Vec::with_capacity(messages.len());
        rebuilt.extend_from_slice(prefix_slice);
        if memory_config.enable_working_memory {
            rebuilt.push(ChatMessage::user(build_working_memory_block(
                merged_existing_wm.as_deref(),
                &groups[..fold_count],
                memory_config.summarize_compacted_turns,
            )));
        }
        for grp in &groups[fold_count..] {
            rebuilt.extend(grp.iter().cloned());
        }
        rebuilt.extend_from_slice(suffix_slice);
        *messages = rebuilt;
    } else if should_merge_redundant_wm {
        let prefix_slice = &messages[..start_idx];
        let suffix_slice = &messages[recent_start..];
        let mut rebuilt = Vec::with_capacity(messages.len());
        rebuilt.extend_from_slice(prefix_slice);
        if memory_config.enable_working_memory {
            rebuilt.push(ChatMessage::user(build_working_memory_block(
                merged_existing_wm.as_deref(),
                &[],
                memory_config.summarize_compacted_turns,
            )));
        }
        rebuilt.extend(middle_non_wm);
        rebuilt.extend_from_slice(suffix_slice);
        *messages = rebuilt;
    }

    let after_tokens = estimate_messages_tokens(messages);
    if pruned_outputs > 0 || messages_folded > 0 || after_tokens < before_tokens {
        Some(CompactionReport {
            before_tokens,
            after_tokens,
            messages_folded,
            pruned_outputs,
        })
    } else {
        None
    }
}

/// Estimate token usage for a single message (role + content + reasoning + serialized tool_calls + overhead).
pub fn estimate_message_tokens(msg: &ChatMessage) -> usize {
    let mut units = weighted_text_units(&msg.role);
    if let Some(ref c) = msg.content {
        units += weighted_text_units(c);
    }
    let mut media_tokens = 0usize;
    for att in &msg.attachments {
        if let Some(ref txt) = att.text_content {
            units += weighted_text_units(txt) + weighted_text_units(&att.name);
        } else {
            media_tokens += match att.kind.as_str() {
                "video" => 1024,
                "audio" => 512,
                "image" => 256,
                _ => 64,
            };
        }
    }
    if let Some(ref r) = msg.reasoning_content {
        units += weighted_text_units(r);
    }
    if let Some(ref id) = msg.tool_call_id {
        units += weighted_text_units(id);
    }
    if let Some(ref tcs) = msg.tool_calls {
        if let Ok(json) = serde_json::to_string(tcs) {
            units += weighted_text_units(&json);
        } else {
            for tc in tcs {
                units += weighted_text_units(&tc.id)
                    + weighted_text_units(&tc.r#type)
                    + weighted_text_units(&tc.function.name)
                    + weighted_text_units(&tc.function.arguments);
            }
        }
    }
    units.saturating_mul(2).div_ceil(7) + media_tokens + MESSAGE_TOKEN_OVERHEAD
}

fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

fn weighted_text_units(s: &str) -> usize {
    let bytes = s.len();
    if bytes == 0 {
        return 0;
    }
    let chars = s.chars().count();
    if bytes == chars {
        chars
    } else {
        // Multi-byte UTF-8 (e.g. CJK/emoji) tokenizes closer to byte density than pure char count.
        (chars + bytes).div_ceil(2)
    }
}

fn estimate_tokens_parts(
    prefix: &[ChatMessage],
    wm: Option<&ChatMessage>,
    remaining_groups: &[Vec<ChatMessage>],
    suffix: &[ChatMessage],
) -> usize {
    let mut total = estimate_messages_tokens(prefix);
    if let Some(w) = wm {
        total += estimate_message_tokens(w);
    }
    for grp in remaining_groups {
        total += estimate_messages_tokens(grp);
    }
    total + estimate_messages_tokens(suffix)
}

fn is_working_memory_msg(msg: &ChatMessage) -> bool {
    msg.content.as_deref().is_some_and(|c| {
        c.starts_with(WORKING_MEMORY_PREFIX) || c.starts_with(ALT_WORKING_MEMORY_PREFIX)
    })
}

fn is_deduplicatable_readonly_call(name: &str, raw_args: &str) -> bool {
    match name {
        "read_file" | "grep_search" | "list_dir" | "web_search" => true,
        "browser_ui" => serde_json::from_str::<Value>(raw_args)
            .ok()
            .and_then(|v| {
                v.get("action")
                    .and_then(|a| a.as_str())
                    .map(|a| matches!(a, "snapshot" | "inspect_dom"))
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn canonical_json_value(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), canonical_json_value(v));
            }
            let mut out = serde_json::Map::new();
            for (k, v) in sorted {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonical_json_value).collect()),
        other => other.clone(),
    }
}

fn canonical_tool_args(raw_args: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(raw_args) {
        let canon = canonical_json_value(&val);
        if let Ok(s) = serde_json::to_string(&canon) {
            return s;
        }
    }
    raw_args.trim().to_string()
}

fn smart_prune_tool_output(raw: &str, prune_limit: usize) -> Option<String> {
    if raw.len() <= prune_limit {
        return None;
    }

    let keep_budget = prune_limit.clamp(80, 900);
    let head_budget = keep_budget / 2;
    let tail_budget = keep_budget - head_budget;

    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len();

    let (head, tail, kept_lines) = if total_lines >= 4 {
        let mut head_lines: Vec<&str> = Vec::new();
        let mut head_bytes = 0usize;
        let mut head_end_line = 0usize;

        for (idx, line) in lines.iter().enumerate() {
            let needed = line.len() + usize::from(!head_lines.is_empty());
            if head_lines.is_empty() || head_bytes + needed <= head_budget {
                if line.len() > head_budget && head_lines.is_empty() {
                    let end = snap_clean_end(line, head_budget);
                    head_lines.push(&line[..end]);
                    head_end_line = idx + 1;
                    break;
                }
                head_lines.push(line);
                head_bytes += needed;
                head_end_line = idx + 1;
            } else {
                break;
            }
        }

        let mut tail_lines_rev: Vec<&str> = Vec::new();
        let mut tail_bytes = 0usize;
        for (idx, line) in lines.iter().enumerate().rev() {
            if idx < head_end_line {
                break;
            }
            let needed = line.len() + usize::from(!tail_lines_rev.is_empty());
            if tail_lines_rev.is_empty() || tail_bytes + needed <= tail_budget {
                if line.len() > tail_budget && tail_lines_rev.is_empty() {
                    let start = snap_clean_start(line, line.len().saturating_sub(tail_budget));
                    tail_lines_rev.push(&line[start..]);
                    break;
                }
                tail_lines_rev.push(line);
                tail_bytes += needed;
            } else {
                break;
            }
        }
        tail_lines_rev.reverse();

        let kept = head_lines.len() + tail_lines_rev.len();
        (head_lines.join("\n"), tail_lines_rev.join("\n"), kept)
    } else {
        // Single huge line or very few lines (e.g. minified JSON or long line + header/footer)
        let mut head_end = snap_clean_end(raw, head_budget.min(raw.len()));
        let mut tail_start = snap_clean_start(raw, raw.len().saturating_sub(tail_budget));
        if head_end >= tail_start {
            return None;
        }

        // Preserve short first/last lines if present
        if let Some(first_nl) = raw.find('\n') {
            if first_nl < head_budget {
                head_end = head_end.max(first_nl);
            }
        }
        if let Some(last_nl) = raw.rfind('\n') {
            if raw.len().saturating_sub(last_nl) < tail_budget {
                tail_start = tail_start.min(last_nl + 1);
            }
        }
        if head_end >= tail_start {
            return None;
        }

        let head_str = raw[..head_end].trim_end_matches('\r');
        let tail_str = raw[tail_start..].trim_start_matches('\r');
        let kept = head_str.lines().count() + tail_str.lines().count();
        (head_str.to_string(), tail_str.to_string(), kept)
    };

    let omitted_bytes = raw.len().saturating_sub(head.len() + tail.len());
    let omitted_lines = total_lines.saturating_sub(kept_lines);

    let compacted = format!(
        "{}\n[Tool output compacted: omitted {} bytes, {} lines]\n{}",
        head, omitted_bytes, omitted_lines, tail
    );

    (compacted.len() < raw.len()).then_some(compacted)
}

fn snap_clean_end(s: &str, max_bytes: usize) -> usize {
    let limit = floor_char_boundary(s, max_bytes.min(s.len()));
    let window_start = floor_char_boundary(s, limit.saturating_sub(24));
    if let Some(rel) = s[window_start..limit].rfind(|c: char| matches!(c, '\n' | ',' | '}' | ']' | ';' | ' ')) {
        let candidate = window_start + rel + 1;
        if candidate > 0 {
            return candidate;
        }
    }
    limit
}

fn snap_clean_start(s: &str, min_bytes: usize) -> usize {
    let start = ceil_char_boundary(s, min_bytes.min(s.len()));
    let window_end = ceil_char_boundary(s, (start + 24).min(s.len()));
    if let Some(rel) = s[start..window_end].find(|c: char| matches!(c, '\n' | ',' | '{' | '[' | ' ')) {
        let candidate = start + rel + 1;
        if candidate < s.len() {
            return candidate;
        }
    }
    start
}

/// Partition messages into atomic groups so an `assistant` with `tool_calls` and all its trailing
/// `tool` response messages always move or fold together.
fn partition_atomic_groups(slice: &[ChatMessage]) -> Vec<Vec<ChatMessage>> {
    let mut groups: Vec<Vec<ChatMessage>> = Vec::new();
    let mut idx = 0;
    while idx < slice.len() {
        let msg = &slice[idx];
        if msg.role == "assistant" && msg.tool_calls.as_ref().is_some_and(|t| !t.is_empty()) {
            let mut grp = vec![msg.clone()];
            idx += 1;
            while idx < slice.len() && slice[idx].role == "tool" {
                grp.push(slice[idx].clone());
                idx += 1;
            }
            groups.push(grp);
        } else if msg.role == "tool" {
            if let Some(last) = groups.last_mut() {
                last.push(msg.clone());
            } else {
                groups.push(vec![msg.clone()]);
            }
            idx += 1;
        } else {
            groups.push(vec![msg.clone()]);
            idx += 1;
        }
    }
    groups
}

fn build_working_memory_block(
    existing_wm: Option<&str>,
    folded_groups: &[Vec<ChatMessage>],
    summarize_turns: bool,
) -> String {
    let new_msg_count: usize = folded_groups.iter().map(|g| g.len()).sum();

    if !summarize_turns {
        return cap_chars(
            &format!(
                "{}\nEarlier conversation turns compacted ({} messages folded).",
                WORKING_MEMORY_PREFIX, new_msg_count
            ),
            WORKING_MEMORY_MAX_CHARS,
        );
    }

    let mut goals: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut tools: Vec<String> = Vec::new();
    let mut open_context: Vec<String> = Vec::new();

    if let Some(prev) = existing_wm {
        parse_existing_working_memory(prev, &mut goals, &mut files, &mut tools, &mut open_context);
    }

    for grp in folded_groups {
        for msg in grp {
            match msg.role.as_str() {
                "user" => {
                    if let Some(ref c) = msg.content {
                        let clean = collapse_ws(c);
                        if !clean.is_empty()
                            && !clean.starts_with(WORKING_MEMORY_PREFIX)
                            && !clean.starts_with(ALT_WORKING_MEMORY_PREFIX)
                        {
                            push_unique(&mut goals, cap_chars(&clean, 140));
                        }
                    }
                }
                "assistant" => {
                    if let Some(ref tcs) = msg.tool_calls {
                        for tc in tcs {
                            if let Some(file_entry) = extract_file_activity(&tc.function.name, &tc.function.arguments) {
                                push_unique(&mut files, file_entry);
                            }
                            let arg_hint = extract_tool_arg_hint(&tc.function.arguments);
                            let entry = if arg_hint.is_empty() {
                                tc.function.name.clone()
                            } else {
                                format!("{}({})", tc.function.name, arg_hint)
                            };
                            push_unique(&mut tools, entry);
                        }
                    }
                    if let Some(ref c) = msg.content {
                        let clean = collapse_ws(c);
                        if !clean.is_empty() {
                            push_unique(&mut open_context, cap_chars(&clean, 120));
                        }
                    }
                }
                "tool" => {
                    if let Some(ref c) = msg.content {
                        let clean = collapse_ws(c);
                        if !clean.is_empty() && clean != SUPERSEDED_TOMBSTONE {
                            push_unique(&mut open_context, cap_chars(&clean, 110));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Keep the most recent items in each bucket so summary stays focused and within budget
    trim_vec_tail(&mut goals, 4);
    trim_vec_tail(&mut files, 6);
    trim_vec_tail(&mut tools, 8);
    trim_vec_tail(&mut open_context, 5);

    let mut out = String::from(WORKING_MEMORY_PREFIX);
    if !goals.is_empty() {
        out.push_str("\nGoals: ");
        out.push_str(&goals.join(" | "));
    }
    if !files.is_empty() {
        out.push_str("\nFiles: ");
        out.push_str(&files.join(", "));
    }
    if !tools.is_empty() {
        out.push_str("\nTools: ");
        out.push_str(&tools.join(", "));
    }
    if !open_context.is_empty() {
        out.push_str("\nOpen Context: ");
        out.push_str(&open_context.join(" | "));
    }
    if goals.is_empty() && files.is_empty() && tools.is_empty() && open_context.is_empty() {
        out.push_str(&format!("\nFolded {} earlier messages.", new_msg_count));
    }

    cap_chars(&out, WORKING_MEMORY_MAX_CHARS)
}

fn extract_file_activity(tool_name: &str, raw_args: &str) -> Option<String> {
    let verb = match tool_name {
        "read_file" | "list_dir" | "grep_search" => "read",
        "write_file" | "edit_file" | "patch_file" | "create_file" | "delete_file" => "edited",
        _ => return None,
    };
    let val = serde_json::from_str::<Value>(raw_args).ok()?;
    let obj = val.as_object()?;
    for key in ["path", "file", "target_file"] {
        if let Some(p) = obj.get(key).and_then(|v| v.as_str()) {
            let clean = collapse_ws(p);
            if !clean.is_empty() {
                return Some(format!("{} ({})", cap_chars(&clean, 48), verb));
            }
        }
    }
    None
}

fn parse_existing_working_memory(
    prev: &str,
    goals: &mut Vec<String>,
    files: &mut Vec<String>,
    tools: &mut Vec<String>,
    open_context: &mut Vec<String>,
) {
    for line in prev.lines() {
        let t = line.trim();
        if t.is_empty() || t == WORKING_MEMORY_PREFIX || t == ALT_WORKING_MEMORY_PREFIX {
            continue;
        }
        if let Some(rest) = t.strip_prefix("Goals:") {
            for item in rest.split(" | ") {
                let s = item.trim();
                if !s.is_empty() {
                    push_unique(goals, s.to_string());
                }
            }
        } else if let Some(rest) = t.strip_prefix("Files:") {
            for item in rest.split(", ") {
                let s = item.trim();
                if !s.is_empty() {
                    push_unique(files, s.to_string());
                }
            }
        } else if let Some(rest) = t.strip_prefix("Tools:") {
            for item in rest.split(", ") {
                let s = item.trim();
                if !s.is_empty() {
                    push_unique(tools, s.to_string());
                }
            }
        } else if let Some(rest) = t
            .strip_prefix("Open Context:")
            .or_else(|| t.strip_prefix("Results:"))
        {
            for item in rest.split(" | ") {
                let s = item.trim();
                if !s.is_empty() {
                    push_unique(open_context, s.to_string());
                }
            }
        } else {
            push_unique(open_context, cap_chars(t, 120));
        }
    }
}

fn extract_tool_arg_hint(raw_args: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(raw_args) {
        if let Some(obj) = val.as_object() {
            for key in ["path", "file", "command", "query", "url", "action", "name", "pattern"] {
                if let Some(v) = obj.get(key).and_then(|x| x.as_str()) {
                    return cap_chars(&collapse_ws(v), 40);
                }
            }
        }
    }
    cap_chars(&collapse_ws(raw_args), 36)
}

fn push_unique(vec: &mut Vec<String>, item: String) {
    if !item.is_empty() && !vec.iter().any(|existing| existing == &item) {
        vec.push(item);
    }
}

fn trim_vec_tail(vec: &mut Vec<String>, max_len: usize) {
    if vec.len() > max_len {
        let drop = vec.len() - max_len;
        vec.drain(0..drop);
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cap_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            r#type: "function".to_string(),
            function: ToolFunctionCall {
                name: name.to_string(),
                arguments: args.to_string(),
            },
        }
    }

    #[test]
    fn test_estimate_message_tokens_includes_all_fields_and_overhead() {
        let m = ChatMessage::assistant(
            Some("hello world".into()),
            Some("thinking trace".into()),
            Some(vec![tc("c1", "read_file", r#"{"path":"src/main.rs"}"#)]),
        );
        let tok = estimate_message_tokens(&m);
        assert!(tok > MESSAGE_TOKEN_OVERHEAD);

        let cjk = ChatMessage::user("你好世界测试文本");
        assert!(estimate_message_tokens(&cjk) >= MESSAGE_TOKEN_OVERHEAD + 4);
    }

    #[test]
    fn test_tier1_deduplicates_identical_readonly_calls() {
        let mut msgs = vec![
            ChatMessage::system("System prompt"),
            ChatMessage::user("Check files"),
            ChatMessage::assistant(
                None,
                None,
                Some(vec![
                    tc("call_old_read", "read_file", r#"{"path": "a.rs", "start": 1}"#),
                    tc("call_old_snap", "browser_ui", r#"{"action": "snapshot"}"#),
                ]),
            ),
            ChatMessage::tool("call_old_read", "OLD FILE CONTENT ".repeat(20)),
            ChatMessage::tool("call_old_snap", "OLD SNAPSHOT ".repeat(20)),
            ChatMessage::assistant(
                None,
                None,
                Some(vec![
                    tc("call_new_read", "read_file", r#"{"start":1,"path":"a.rs"}"#),
                    tc("call_new_snap", "browser_ui", r#"{"action":"snapshot"}"#),
                ]),
            ),
            ChatMessage::tool("call_new_read", "FRESH CONTENT"),
            ChatMessage::tool("call_new_snap", "FRESH SNAPSHOT"),
        ];

        let cfg = MemoryConfig {
            max_context_tokens: 128_000,
            auto_compact_threshold_pct: 80,
            preserve_recent_messages: 3,
            prune_tool_outputs_over_bytes: 2048,
            summarize_compacted_turns: true,
            enable_working_memory: true,
            ..MemoryConfig::default()
        };

        let report = compact_messages_with_config(&mut msgs, &cfg, true)
            .expect("force compaction should deduplicate older calls");
        assert_eq!(report.pruned_outputs, 2);
        assert_eq!(
            msgs.iter()
                .find(|m| m.tool_call_id.as_deref() == Some("call_old_read"))
                .and_then(|m| m.content.as_deref()),
            Some(SUPERSEDED_TOMBSTONE)
        );
        assert_eq!(
            msgs.iter()
                .find(|m| m.tool_call_id.as_deref() == Some("call_old_snap"))
                .and_then(|m| m.content.as_deref()),
            Some(SUPERSEDED_TOMBSTONE)
        );
        assert_eq!(
            msgs.iter()
                .find(|m| m.tool_call_id.as_deref() == Some("call_new_read"))
                .and_then(|m| m.content.as_deref()),
            Some("FRESH CONTENT")
        );
    }

    #[test]
    fn test_tier2_smart_line_aware_pruning_and_wm_merge() {
        let multi_line: String = (0..40)
            .map(|i| format!("line_{:02}: some structured output payload value_{}", i, i))
            .collect::<Vec<_>>()
            .join("\n");

        let mut msgs = vec![
            ChatMessage::system("System"),
            ChatMessage::user("[COMPACTED WORKING MEMORY]\nGoals: Initial goal"),
            ChatMessage::user("Second goal"),
            ChatMessage::assistant(
                None,
                None,
                Some(vec![tc("c1", "write_file", r#"{"path":"src/lib.rs"}"#)]),
            ),
            ChatMessage::tool("c1", multi_line),
            ChatMessage::assistant(Some("Updated lib.rs".into()), None, None),
            ChatMessage::user("Final recent check"),
        ];

        let cfg = MemoryConfig {
            max_context_tokens: 200,
            auto_compact_threshold_pct: 50,
            preserve_recent_messages: 1,
            prune_tool_outputs_over_bytes: 200,
            summarize_compacted_turns: true,
            enable_working_memory: true,
            ..MemoryConfig::default()
        };

        let report = compact_messages_with_config(&mut msgs, &cfg, false).unwrap();
        assert!(report.after_tokens < report.before_tokens);
        // Only a single merged working memory block should exist
        let wm_count = msgs.iter().filter(|m| is_working_memory_msg(m)).count();
        assert_eq!(wm_count, 1);
        let wm_text = msgs[1].content.as_deref().unwrap();
        assert!(wm_text.contains("Initial goal"));
        assert!(wm_text.contains("Second goal"));
        assert!(wm_text.contains("Files: src/lib.rs (edited)"));
        assert!(wm_text.contains("Open Context:"));
    }

    #[test]
    fn test_preserve_zero_does_not_panic() {
        let mut ctx = ConversationContext::from_messages(
            vec![
                ChatMessage::system("Sys"),
                ChatMessage::user("Do task"),
                ChatMessage::assistant(Some("Done".into()), None, None),
            ],
            4096,
        )
        .with_memory_config(MemoryConfig {
            max_context_tokens: 10,
            auto_compact_threshold_pct: 50,
            preserve_recent_messages: 0,
            prune_tool_outputs_over_bytes: 100,
            summarize_compacted_turns: true,
            enable_working_memory: true,
            ..MemoryConfig::default()
        });

        let report = ctx.compact_if_needed();
        assert!(report.is_some());
    }
}
