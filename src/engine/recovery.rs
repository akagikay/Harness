use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use crate::plugins::contract::ToolContract;

/// High-level classification of a tool failure for retry and self-correction decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    Validation,
    TransientNetwork,
    NotFound,
    PermissionDenied,
    UnknownTool,
    Timeout,
    ExecutionError,
}

impl ToolErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolErrorKind::Validation => "validation",
            ToolErrorKind::TransientNetwork => "transient_network",
            ToolErrorKind::NotFound => "not_found",
            ToolErrorKind::PermissionDenied => "permission_denied",
            ToolErrorKind::UnknownTool => "unknown_tool",
            ToolErrorKind::Timeout => "timeout",
            ToolErrorKind::ExecutionError => "execution_error",
        }
    }
}

impl fmt::Display for ToolErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured diagnosis produced for a failed tool invocation.
#[derive(Debug, Clone)]
pub struct ToolErrorDiagnosis {
    pub kind: ToolErrorKind,
    pub is_retryable: bool,
    pub summary: String,
    pub recovery_hint: String,
}

/// Attempts to parse raw tool arguments as JSON, repairing common LLM formatting glitches
/// such as markdown code fences, trailing commas, and surrounding prose.
pub fn parse_and_repair_tool_args(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "Invalid tool arguments: received empty string; expected a JSON object (e.g. `{}`)."
                .to_string(),
        );
    }

    // 1. Fast path: strict JSON parse
    match serde_json::from_str::<Value>(trimmed) {
        Ok(val) => return Ok(unwrap_double_encoded_object(val)),
        Err(first_err) => {
            // 2. Strip markdown fences (```json ... ``` or ``` ... ```)
            let unfenced = strip_markdown_fences(trimmed);
            let mut candidates: Vec<String> = Vec::with_capacity(4);

            candidates.push(strip_trailing_commas(&unfenced));

            if let Some(obj_slice) = extract_balanced_json_object(&unfenced) {
                candidates.push(obj_slice.to_string());
                candidates.push(strip_trailing_commas(obj_slice));
            }

            if unfenced != trimmed {
                if let Some(obj_slice) = extract_balanced_json_object(trimmed) {
                    candidates.push(strip_trailing_commas(obj_slice));
                }
            }

            let mut last_err = first_err.to_string();
            for candidate in candidates {
                let c = candidate.trim();
                if c.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(c) {
                    Ok(val) => return Ok(unwrap_double_encoded_object(val)),
                    Err(e) => last_err = e.to_string(),
                }
            }

            let snippet: String = trimmed.chars().take(180).collect();
            Err(format!(
                "Failed to parse tool arguments as valid JSON ({}). Received snippet: {:?}",
                last_err, snippet
            ))
        }
    }
}

fn unwrap_double_encoded_object(val: Value) -> Value {
    if let Value::String(ref inner) = val {
        let inner_trimmed = inner.trim();
        if inner_trimmed.starts_with('{') && inner_trimmed.ends_with('}') {
            if let Ok(parsed @ Value::Object(_)) = serde_json::from_str::<Value>(inner_trimmed) {
                return parsed;
            }
        }
    }
    val
}

fn strip_markdown_fences(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(start_fence) = trimmed.find("```") {
        let after_open = &trimmed[start_fence + 3..];
        // Skip optional language tag on the opening fence line (e.g. ```json)
        let body_start = match after_open.find('\n') {
            Some(nl) => &after_open[nl + 1..],
            None => after_open.trim_start_matches(|c: char| c.is_ascii_alphanumeric()),
        };
        if let Some(end_fence) = body_start.rfind("```") {
            return body_start[..end_fence].trim().to_string();
        }
        return body_start.trim().to_string();
    }
    trimmed.to_string()
}

/// Removes trailing commas immediately preceding `}` or `]` outside JSON string literals.
fn strip_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let ch = input[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();

        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += ch_len;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += ch_len;
            continue;
        }

        if ch == ',' {
            let mut j = i + ch_len;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                // Skip trailing comma
                i += ch_len;
                continue;
            }
        }

        out.push(ch);
        i += ch_len;
    }

    out
}

/// Extracts the first balanced `{...}` JSON object while respecting string literals.
fn extract_balanced_json_object(input: &str) -> Option<&str> {
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut start_idx: Option<usize> = None;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start_idx = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = start_idx {
                            return Some(&input[start..=idx]);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Returns true only for idempotent read/web/MCP search tools safe to auto-retry on transient failures.
pub fn is_idempotent_retryable_tool(tool: &str) -> bool {
    matches!(
        tool,
        "web_search"
            | "web_fetch"
            | "read_file"
            | "list_dir"
            | "grep_search"
            | "sys_info"
    ) || tool.starts_with("mcp_google_ai_search_")
}

/// Classifies a tool error and computes concrete recovery advice combining matching
/// `ToolContract` failure modes, expected schema fields, and tool-specific guidance.
pub fn diagnose_tool_error(
    tool: &str,
    err: &str,
    contract: Option<&ToolContract>,
) -> ToolErrorDiagnosis {
    let lower = err.to_lowercase();
    let kind = classify_error_kind(tool, &lower);
    let is_retryable = matches!(
        kind,
        ToolErrorKind::TransientNetwork | ToolErrorKind::Timeout
    ) && is_idempotent_retryable_tool(tool);

    let summary = format!(
        "Tool '{}' failed with {} error{}",
        tool,
        kind.as_str(),
        if is_retryable { " (retryable)" } else { "" }
    );

    let mut hints: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. Contract-level failure mode mitigations
    if let Some(c) = contract {
        for fm in &c.failure_modes {
            if failure_mode_matches(&fm.code, &fm.condition, &fm.error_message, &lower) {
                push_unique_hint(&mut hints, &mut seen, fm.recovery_hint.clone());
            }
        }

        // 2. For validation errors, surface expected required/optional fields
        if kind == ToolErrorKind::Validation && !c.fields.is_empty() {
            let required_fields: Vec<String> = c
                .fields
                .iter()
                .filter(|f| f.required)
                .map(|f| format!("{} ({})", f.name, f.field_type))
                .collect();
            let optional_fields: Vec<String> = c
                .fields
                .iter()
                .filter(|f| !f.required)
                .map(|f| format!("{} ({})", f.name, f.field_type))
                .collect();

            let mut schema_hint = String::new();
            if !required_fields.is_empty() {
                schema_hint.push_str(&format!(
                    "Expected required fields: [{}]",
                    required_fields.join(", ")
                ));
            }
            if !optional_fields.is_empty() {
                if !schema_hint.is_empty() {
                    schema_hint.push_str("; ");
                }
                schema_hint.push_str(&format!("optional: [{}]", optional_fields.join(", ")));
            }
            if !schema_hint.is_empty() {
                schema_hint.push('.');
                push_unique_hint(&mut hints, &mut seen, schema_hint);
            }
        }
    }

    // 3. Tool-specific actionable recovery guidance
    match tool {
        "edit_file" => {
            if lower.contains("target string not found")
                || lower.contains("target") && kind == ToolErrorKind::NotFound
            {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Call `read_file` with `start_line` and `end_line` on the file first, then copy the exact `target` text including indentation and whitespace."
                        .to_string(),
                );
            } else if kind == ToolErrorKind::NotFound {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Call `list_dir` or `grep_search` to confirm the exact file path before editing."
                        .to_string(),
                );
            }
        }
        "read_file" => {
            if kind == ToolErrorKind::NotFound {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Use `list_dir` or `grep_search` to verify the file path exists in the workspace."
                        .to_string(),
                );
            } else if lower.contains("exceeds total lines") || lower.contains("start_line") {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Re-call `read_file` with a smaller `start_line` or omit `start_line`/`end_line` to read from line 1."
                        .to_string(),
                );
            }
        }
        "list_dir" => {
            if kind == ToolErrorKind::NotFound || lower.contains("directory") {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Call `list_dir` with `path: \".\"` to inspect the workspace root and locate valid directories."
                        .to_string(),
                );
            }
        }
        "web_fetch" => {
            if lower.contains("403")
                || lower.contains("401")
                || kind == ToolErrorKind::PermissionDenied
                || kind == ToolErrorKind::TransientNetwork
                || kind == ToolErrorKind::Timeout
                || lower.contains("http error")
                || lower.contains("failed to fetch")
            {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "If `web_fetch` is blocked (e.g. 403) or fails, try `browser_ui` (`navigate` + `inspect_dom`) for JS-rendered/protected pages, or use `mcp_google_ai_search_search` / `web_search`."
                        .to_string(),
                );
            }
        }
        "web_search" => {
            if kind != ToolErrorKind::Validation {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Try `mcp_google_ai_search_search` for grounded AI search results, or broaden/simplify the query."
                        .to_string(),
                );
            }
        }
        t if t.starts_with("mcp_google_ai_search_") => {
            if kind != ToolErrorKind::Validation {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Fall back to `web_search` or `web_fetch`, or check Gemini API key configuration in Settings -> MCP Servers."
                        .to_string(),
                );
            }
        }
        "browser_ui" => {
            if kind != ToolErrorKind::Validation {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Use `http_check` to verify the local/remote URL responds, or `inspect_dom` before attempting `interact` selectors."
                        .to_string(),
                );
            }
        }
        "bash" => {
            if kind == ToolErrorKind::Timeout {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Command timed out; avoid long-running/blocking servers in `bash` or scope the command to a faster check."
                        .to_string(),
                );
            } else if kind != ToolErrorKind::Validation && kind != ToolErrorKind::PermissionDenied {
                push_unique_hint(
                    &mut hints,
                    &mut seen,
                    "Inspect stderr/exit status, verify paths relative to workspace root, and adjust flags instead of repeating the same command."
                        .to_string(),
                );
            }
        }
        _ => {}
    }

    // 4. General kind-based fallbacks if nothing specific matched yet
    if hints.is_empty() {
        let fallback = match kind {
            ToolErrorKind::UnknownTool => {
                "Call `manage_tools` with `action: \"list\"` to check exact registered tool names."
            }
            ToolErrorKind::Validation => {
                "Pass a valid JSON object matching the tool's required parameters and types."
            }
            ToolErrorKind::PermissionDenied => {
                "Respect permission/sandbox boundaries or choose an allowed alternative tool."
            }
            ToolErrorKind::Timeout | ToolErrorKind::TransientNetwork => {
                "Transient I/O or timeout failure; simplify the request or try an alternative tool."
            }
            ToolErrorKind::NotFound => {
                "Verify the target path, identifier, or resource exists before retrying."
            }
            ToolErrorKind::ExecutionError => {
                "Review the error details and modify parameters or strategy before retrying."
            }
        };
        push_unique_hint(&mut hints, &mut seen, fallback.to_string());
    }

    ToolErrorDiagnosis {
        kind,
        is_retryable,
        summary,
        recovery_hint: hints.join(" "),
    }
}

/// Formats a tool failure message with a structured `[TOOL RECOVERY HINT]` block for the LLM.
pub fn format_tool_error_with_hint(
    tool: &str,
    err: &str,
    diagnosis: &ToolErrorDiagnosis,
) -> String {
    format!(
        "Error in tool '{}' [{}]: {}\n[TOOL RECOVERY HINT]: {}",
        tool,
        diagnosis.kind.as_str(),
        err.trim(),
        diagnosis.recovery_hint
    )
}

fn push_unique_hint(hints: &mut Vec<String>, seen: &mut HashSet<String>, hint: String) {
    let normalized = hint.trim().to_string();
    if !normalized.is_empty() && seen.insert(normalized.to_lowercase()) {
        hints.push(normalized);
    }
}

fn classify_error_kind(tool: &str, lower: &str) -> ToolErrorKind {
    if lower.contains("unknown tool")
        || lower.contains("not found in registry")
        || lower.contains("unregistered tool")
    {
        return ToolErrorKind::UnknownTool;
    }

    if lower.contains("failed to parse tool arguments")
        || lower.contains("invalid tool arguments")
        || lower.contains("contract violation")
        || lower.contains("contract validation failed")
        || lower.contains("missing required field")
        || lower.contains("missing required parameter")
        || lower.contains("must be a ")
        || lower.contains("must be an ")
        || lower.contains("invalid value '")
        || lower.contains("below minimum")
        || lower.contains("exceeds maximum")
        || lower.contains("invalid json")
        || lower.contains("invalid parameters")
    {
        return ToolErrorKind::Validation;
    }

    if lower.contains("blocked by harness security policy")
        || lower.contains("denied by user permission policy")
        || lower.contains("permission denied")
        || lower.contains("access is denied")
        || lower.contains("sandbox violation")
        || lower.contains("outside workspace")
        || lower.contains("outside sandbox")
        || lower.contains("http error 401")
        || lower.contains("http error 403")
        || lower.contains("status 401")
        || lower.contains("status 403")
        || lower.contains("403 forbidden")
        || lower.contains("401 unauthorized")
        || lower.contains("eacces")
        || lower.contains("eperm")
    {
        return ToolErrorKind::PermissionDenied;
    }

    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("deadline has elapsed")
        || lower.contains("etimedout")
    {
        return ToolErrorKind::Timeout;
    }

    if lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("connection closed")
        || lower.contains("broken pipe")
        || lower.contains("dns")
        || lower.contains("name resolution")
        || lower.contains("temporarily unavailable")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("http error 429")
        || lower.contains("http error 500")
        || lower.contains("http error 502")
        || lower.contains("http error 503")
        || lower.contains("http error 504")
        || lower.contains("network error")
        || lower.contains("error sending request")
        || lower.contains("failed to send request")
        || (matches!(tool, "web_fetch" | "web_search") || tool.starts_with("mcp_"))
            && (lower.contains("failed to fetch url")
                || lower.contains("web search request failed")
                || lower.contains("reqwest"))
    {
        return ToolErrorKind::TransientNetwork;
    }

    if lower.contains("no such file")
        || lower.contains("cannot find the path")
        || lower.contains("cannot find the file")
        || lower.contains("target string not found")
        || lower.contains("does not exist")
        || lower.contains("enoent")
        || lower.contains("http error 404")
        || lower.contains("status 404")
        || lower.contains("not found")
    {
        return ToolErrorKind::NotFound;
    }

    ToolErrorKind::ExecutionError
}

fn failure_mode_matches(code: &str, condition: &str, err_pattern: &str, lower_err: &str) -> bool {
    let code_l = code.to_lowercase();
    let cond_l = condition.to_lowercase();
    let pat_l = err_pattern.to_lowercase();

    // Direct signal matching for common contract codes & error message patterns
    if code_l.contains("target_not_found") && lower_err.contains("target string not found") {
        return true;
    }
    if (code_l.contains("file_not_found") || code_l.contains("dir_not_found"))
        && (lower_err.contains("no such file")
            || lower_err.contains("cannot find")
            || lower_err.contains("not found")
            || lower_err.contains("enoent"))
    {
        return true;
    }
    if code_l.contains("http_status") && lower_err.contains("http error") {
        return true;
    }
    if (code_l.contains("network") || code_l.contains("timeout"))
        && (lower_err.contains("timed out")
            || lower_err.contains("timeout")
            || lower_err.contains("failed to fetch")
            || lower_err.contains("request failed")
            || lower_err.contains("connection"))
    {
        return true;
    }
    if code_l.contains("missing_api_key") && lower_err.contains("api key") {
        return true;
    }

    // Generic prefix/phrase match on static portion of error_message before placeholders
    let static_prefix = pat_l.split('<').next().unwrap_or("").trim();
    if static_prefix.len() >= 10 && lower_err.contains(static_prefix) {
        return true;
    }

    // Keyword overlap against condition when error mentions distinctive terms
    if cond_l.contains("4xx/5xx") && lower_err.contains("http error") {
        return true;
    }

    false
}

/// Computes a deterministic canonical signature for a tool call (`tool:canonical_json`)
/// so repeated equivalent failures are detected even if JSON key ordering or spacing differs.
pub fn normalized_failure_signature(tool: &str, args: &Value) -> String {
    let canonical = canonicalize_value(args);
    let serialized = serde_json::to_string(&canonical).unwrap_or_else(|_| "null".to_string());
    format!("{}:{}", tool.trim(), serialized)
}

fn canonicalize_value(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> = map
                .iter()
                .map(|(k, v)| (k.trim().to_string(), canonicalize_value(v)))
                .collect();
            let ordered_map: serde_json::Map<String, Value> = sorted.into_iter().collect();
            Value::Object(ordered_map)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_value).collect()),
        Value::String(s) => Value::String(s.trim().to_string()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::contract::{OutputContract, ToolContract, ToolFieldSpec, ToolPermission};

    #[test]
    fn repairs_fenced_and_trailing_comma_json() {
        let raw = "Here are the args:\n```json\n{\n  \"path\": \"src/main.rs\",\n  \"tags\": [\"a\", \"b\",],\n}\n```\n";
        let val = parse_and_repair_tool_args(raw).expect("should repair JSON");
        assert_eq!(val["path"], "src/main.rs");
        assert_eq!(val["tags"][1], "b");
    }

    #[test]
    fn preserves_commas_inside_string_literals() {
        let raw = r#"{"target": "foo, }", "path": "a.rs",}"#;
        let val = parse_and_repair_tool_args(raw).expect("should parse");
        assert_eq!(val["target"], "foo, }");
        assert_eq!(val["path"], "a.rs");
    }

    #[test]
    fn extracts_balanced_object_from_prose() {
        let raw = r#"Sure, calling tool with {"query": "rust tokio {select}", "max_results": 3} now."#;
        let val = parse_and_repair_tool_args(raw).expect("should extract object");
        assert_eq!(val["query"], "rust tokio {select}");
        assert_eq!(val["max_results"], 3);
    }

    #[test]
    fn rejects_unparseable_args_with_structured_error() {
        let err = parse_and_repair_tool_args("definitely not json").unwrap_err();
        assert!(err.contains("Failed to parse tool arguments"));
    }

    #[test]
    fn diagnoses_transient_retryable_vs_non_retryable() {
        let d_web = diagnose_tool_error("web_fetch", "HTTP error 503: https://example.com", None);
        assert_eq!(d_web.kind, ToolErrorKind::TransientNetwork);
        assert!(d_web.is_retryable);

        let d_bash = diagnose_tool_error("bash", "Execution timed out after 60s", None);
        assert_eq!(d_bash.kind, ToolErrorKind::Timeout);
        assert!(!d_bash.is_retryable);

        let d_403 = diagnose_tool_error("web_fetch", "HTTP error 403: https://example.com", None);
        assert_eq!(d_403.kind, ToolErrorKind::PermissionDenied);
        assert!(!d_403.is_retryable);
        assert!(d_403.recovery_hint.contains("browser_ui"));
        assert!(d_403.recovery_hint.contains("mcp_google_ai_search_search"));
    }

    #[test]
    fn diagnoses_validation_with_contract_fields() {
        let contract = ToolContract {
            name: "edit_file".into(),
            description: "edit".into(),
            category: "fs".into(),
            permission_level: ToolPermission::WorkspaceWrite,
            fields: vec![
                ToolFieldSpec::required_string("path", "file path", ""),
                ToolFieldSpec::required_string("target", "target", ""),
                ToolFieldSpec::optional_int("line", "optional line"),
            ],
            expected_output: OutputContract::default(),
            failure_modes: vec![],
            timeout_secs: 15,
            is_custom: false,
        };
        let d = diagnose_tool_error(
            "edit_file",
            "Missing required field 'target' (expected type: string) for tool 'edit_file'",
            Some(&contract),
        );
        assert_eq!(d.kind, ToolErrorKind::Validation);
        assert!(d.recovery_hint.contains("path (string)"));
        assert!(d.recovery_hint.contains("target (string)"));
    }

    #[test]
    fn normalized_signature_canonicalizes_key_order_and_whitespace() {
        let a = serde_json::json!({"b": 2, "a": "  hello  "});
        let b = serde_json::json!({"a": "hello", "b": 2});
        assert_eq!(
            normalized_failure_signature("read_file", &a),
            normalized_failure_signature("read_file", &b)
        );
    }
}
