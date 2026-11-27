use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::ToolPlugin;

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 800;
const MAX_WS_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Hard cap on the cumulative size of one reassembled fragmented WebSocket
/// message (4x the per-frame cap, still far below any real CDP payload).
const MAX_WS_MESSAGE_BYTES: usize = MAX_WS_FRAME_BYTES * 4;
/// Hard cap on HTTP response bodies (http_check), CLI `--dump-dom` stdout, and
/// files served by the ephemeral workspace server, to prevent unbounded
/// buffering / OOM on hostile or runaway responses.
const MAX_HTTP_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_PREVIEW_CHARS: usize = 3000;
const MAX_CONSOLE_ERRORS: usize = 50;

/// Hidden harness-internal directories that must never be exposed by the
/// ephemeral workspace server, plus any path whose first segment starts with a
/// leading dot (dotfile/dot-directory such as `.git`, `.harness`,
/// `.harness_artifacts`).
fn is_hidden_harness_path(rel: &Path) -> bool {
    let Some(first) = rel.components().next() else {
        return false;
    };
    let first = first.as_os_str().to_string_lossy();
    let lower = first.to_ascii_lowercase();
    lower.starts_with(".harness") || first.starts_with('.')
}

/// True when appending a WebSocket payload of `next` bytes to a reassembly
/// buffer already holding `current` bytes would exceed the cumulative
/// fragmented-message budget.
fn ws_message_would_exceed(current: usize, next: usize) -> bool {
    current + next > MAX_WS_MESSAGE_BYTES
}

/// True when `bytes` begins with the exact PNG magic signature
/// (`\x89PNG\r\n\x1a\n`), used to validate screenshots captured by the
/// headless CLI `--screenshot` fallback before reporting success.
fn looks_like_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// Reads from `stream` until EOF, aborting with an error once more than
/// `max_bytes` have been buffered so a runaway/hostile response cannot
/// exhaust memory. Returns the buffered bytes on success.
async fn read_to_end_bounded<R: tokio::io::AsyncRead + Unpin>(
    stream: &mut R,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("Failed to read stream: {}", e))?;
        if n == 0 {
            return Ok(buf);
        }
        if buf.len() + n > max_bytes {
            return Err(format!(
                "Response exceeded the {} byte read limit",
                max_bytes
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Reads a reqwest response body with a hard cap: an oversized or absent-body
/// Content-Length is rejected up front and the streamed bytes are never
/// buffered past `max_bytes`. Returns `(body, truncated)` where `truncated`
/// reports a hard cut at `max_bytes` (never exceeded).
async fn read_body_bounded(
    mut res: reqwest::Response,
    max_bytes: usize,
) -> Result<(String, bool), String> {
    let too_large = |len: u64| len > max_bytes as u64;
    if let Some(len) = res.content_length() {
        if too_large(len) {
            return Err(format!(
                "Response body too large ({} bytes > {} byte limit)",
                len, max_bytes
            ));
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("Failed to read HTTP body: {}", e))?
    {
        if buf.len() + chunk.len() > max_bytes {
            let remaining = max_bytes - buf.len();
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

/// Autonomous, zero-idle-RAM Web & UI Navigation/Testing plugin.
///
/// Discovers Edge/Chrome/Chromium lazily on execution, communicates via a
/// lightweight pure-Rust Chrome DevTools Protocol (CDP) WebSocket client over
/// `tokio::net::TcpStream` (with `--dump-dom` and `reqwest` fallbacks), and
/// guarantees browser process cleanup after every invocation.
pub struct BrowserUiPlugin;

impl BrowserUiPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrowserUiPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolPlugin for BrowserUiPlugin {
    fn name(&self) -> &str {
        "browser_ui"
    }

    fn description(&self) -> &str {
        "Autonomous Web & UI debugging tool: capture indexed interactive DOM snapshots ('snapshot'), run comprehensive CDP UI/network/layout/perf diagnostics ('debug_audit'), inspect/mutate cookies & web storage ('storage'), execute multi-step workflows in one browser launch ('navigate'/'interact' with 'steps'), capture viewport/full-page/element PNG screenshots into .harness_artifacts/screenshots/, evaluate JS, inspect DOM, or run fast HTTP checks. For CDP actions, workspace-relative paths (e.g. 'index.html', './dist/index.html') and workspace file:/// URLs are automatically served over an ephemeral loopback http://127.0.0.1 server."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "browser".to_string(),
            permission_level: ToolPermission::ShellOrNetwork,
            fields: vec![
                ToolFieldSpec::required_string(
                    "action",
                    "The browser/UI testing or debugging action to perform.",
                    "Choose one of: navigate, screenshot, eval, inspect_dom, interact, snapshot, debug_audit, storage, http_check.",
                )
                .with_enum([
                    "navigate",
                    "screenshot",
                    "eval",
                    "inspect_dom",
                    "interact",
                    "snapshot",
                    "debug_audit",
                    "storage",
                    "http_check",
                ])
                .non_empty(),
                ToolFieldSpec::required_string(
                    "url",
                    "Target URL or workspace-relative file path (e.g. http://127.0.0.1:3000, https://example.com, 'index.html', './dist/index.html', or a workspace file:/// URL). For CDP actions, workspace-relative paths and workspace file:/// URLs are automatically served over an ephemeral loopback http://127.0.0.1 server.",
                    "Provide a non-empty URL starting with http://, https://, about:, data:, a workspace file:/// URL, or a workspace-relative path like 'index.html' (http_check requires http:// or https://).",
                )
                .non_empty(),
                ToolFieldSpec::optional_string(
                    "preset",
                    "Optional viewport preset: 'desktop' (1440x900), 'laptop' (1280x800), 'tablet' (768x1024), or 'mobile' (390x844). Explicit width/height override preset dimensions.",
                )
                .with_enum(["desktop", "laptop", "tablet", "mobile"])
                .with_error_hint("Choose one of: desktop, laptop, tablet, mobile."),
                ToolFieldSpec::optional_int("width", "Viewport width in pixels (default: 1280, or preset width).")
                    .with_range(Some(100.0), Some(7680.0))
                    .with_default(json!(1280))
                    .with_error_hint("Provide an integer viewport width between 100 and 7680."),
                ToolFieldSpec::optional_int("height", "Viewport height in pixels (default: 800, or preset height).")
                    .with_range(Some(100.0), Some(4320.0))
                    .with_default(json!(800))
                    .with_error_hint("Provide an integer viewport height between 100 and 4320."),
                ToolFieldSpec::optional_int(
                    "wait_ms",
                    "Extra milliseconds to wait after page load or interaction for JS/UI rendering (default: 800 for navigate/eval/snapshot/debug_audit, 300 for interact).",
                )
                .with_min(0.0)
                .with_error_hint("Provide a non-negative integer >= 0 for wait_ms."),
                ToolFieldSpec::optional_int(
                    "timeout_secs",
                    "Maximum overall seconds before aborting and killing the browser (default: 25, max: 90).",
                )
                .with_range(Some(3.0), Some(90.0))
                .with_default(json!(25))
                .with_error_hint("Provide an integer between 3 and 90 seconds."),
                ToolFieldSpec::optional_bool(
                    "screenshot",
                    "For 'navigate', 'interact', 'snapshot', or 'debug_audit', also save a PNG screenshot to .harness_artifacts/screenshots/ (default: false).",
                )
                .with_default(json!(false))
                .with_error_hint("Pass a boolean true or false for screenshot."),
                ToolFieldSpec::optional_string(
                    "screenshot_name",
                    "Optional filename stem (alphanumeric, '-', '_' only) for the PNG screenshot inside .harness_artifacts/screenshots/.",
                )
                .with_error_hint(
                    "Use only ASCII letters, digits, '_', and '-' (no path separators or '..').",
                ),
                ToolFieldSpec::optional_bool(
                    "full_page",
                    "Capture full scrollable page instead of only the viewport (default: false).",
                )
                .with_default(json!(false))
                .with_error_hint("Pass a boolean true or false for full_page."),
                ToolFieldSpec::optional_string(
                    "element_selector",
                    "Optional CSS selector (or '[ref_id]') to capture a clipped PNG screenshot of a specific element's bounding box.",
                )
                .with_error_hint("Provide a CSS selector such as '#card' or '.modal' for clipped element screenshots."),
                ToolFieldSpec::optional_string(
                    "expression",
                    "JavaScript expression to evaluate on the page (for 'eval' action).",
                )
                .with_error_hint("Provide a JavaScript expression string when action='eval'."),
                ToolFieldSpec::optional_string(
                    "selector",
                    "CSS selector to inspect (for 'inspect_dom') or target (for 'interact' / 'http_check').",
                )
                .with_error_hint("Provide a valid CSS selector string such as '#app' or 'button.submit'."),
                ToolFieldSpec::optional_int(
                    "ref_id",
                    "Optional 1-based interactive element ref_id from 'snapshot' to target (alternative to CSS 'selector').",
                )
                .with_min(1.0)
                .with_error_hint("Provide a positive integer ref_id (>= 1) matching a snapshot element."),
                ToolFieldSpec::optional_string(
                    "text",
                    "Optional visible text fallback to locate a target element when 'selector' and 'ref_id' are omitted.",
                )
                .with_error_hint("Provide visible element text such as 'Sign In' or 'Submit'."),
                ToolFieldSpec::optional_string(
                    "interaction",
                    "Single interaction operation to run on target (for 'interact' when 'steps' is omitted).",
                )
                .with_enum([
                    "click",
                    "dblclick",
                    "fill",
                    "type",
                    "press_key",
                    "select_option",
                    "select",
                    "hover",
                    "wait_for_selector",
                    "wait_ms",
                    "scroll_by",
                    "scroll_to",
                    "eval",
                    "screenshot",
                    "assert_text",
                    "assert_selector",
                ])
                .with_error_hint("Choose one of: click, dblclick, fill, type, press_key, select_option, select, hover, wait_for_selector, wait_ms, scroll_by, scroll_to, eval, screenshot, assert_text, assert_selector."),
                ToolFieldSpec::optional_string(
                    "value",
                    "Value to fill/type/select, key name for press_key, JS expression for eval, or expected text for assert_text.",
                )
                .with_error_hint("Provide a string value for fill/type/select_option/press_key or expected substring for assert_text."),
                ToolFieldSpec::optional_array(
                    "steps",
                    "Optional sequence of workflow steps [{interaction/action, selector, ref_id, text, value, wait_ms, x, y, expression, screenshot_name, full_page, element_selector}] executed sequentially in one CDP browser launch (accepted by 'interact' and 'navigate').",
                )
                .with_error_hint("Provide a non-empty array of step objects with 'interaction' (or 'action') and step parameters."),
                ToolFieldSpec::optional_array(
                    "actions",
                    "Alias for 'steps': sequential workflow steps executed in one CDP browser launch (accepted by 'interact' and 'navigate').",
                )
                .with_error_hint("Provide a non-empty array of step objects with 'interaction' (or 'action') and step parameters."),
                ToolFieldSpec::optional_string(
                    "storage_op",
                    "Operation for 'storage' action: 'get' (inspect), 'set', 'remove', or 'clear' (default: 'get').",
                )
                .with_enum(["get", "set", "remove", "clear"])
                .with_default(json!("get"))
                .with_error_hint("Choose one of: get, set, remove, clear."),
                ToolFieldSpec::optional_string(
                    "storage_type",
                    "Storage area for 'storage' action: 'all', 'local' (localStorage), 'session' (sessionStorage), or 'cookie' (default: 'all' for get/clear, 'local' for set/remove).",
                )
                .with_enum(["all", "local", "session", "cookie"])
                .with_default(json!("all"))
                .with_error_hint("Choose one of: all, local, session, cookie."),
                ToolFieldSpec::optional_string(
                    "storage_key",
                    "Key or cookie name for 'storage' action when storage_op is 'set' or 'remove'.",
                )
                .with_error_hint("Provide a non-empty storage key or cookie name."),
                ToolFieldSpec::optional_string(
                    "assert_text",
                    "Optional expected substring in body/element text (for 'http_check').",
                )
                .with_error_hint("Provide the expected substring to verify in the response."),
                ToolFieldSpec::optional_int(
                    "assert_status",
                    "Optional expected HTTP status code (for 'http_check', e.g. 200).",
                )
                .with_range(Some(100.0), Some(599.0))
                .with_error_hint("Provide a valid HTTP status code integer between 100 and 599."),
                ToolFieldSpec::optional_object(
                    "headers",
                    "Optional HTTP headers map for 'http_check'.",
                )
                .with_error_hint("Provide a JSON object mapping header names to string values."),
            ],
            expected_output: OutputContract::new(
                "json",
                "Markdown summary plus JSON payload reporting page title, URL, indexed snapshot elements, debug_audit diagnostics, storage state, workflow step results, console errors, failed requests, HTTP check assertions, or screenshot path.",
                "```json\n{\"action\":\"snapshot\",\"page_title\":\"Harness UI\",\"url\":\"http://127.0.0.1:3000/\",\"overflow_x\":false,\"elements\":[{\"ref_id\":1,\"tag\":\"button\",\"role\":\"button\",\"text\":\"Submit\",\"selector\":\"#submit\",\"visible\":true,\"enabled\":true}],\"console_errors\":[],\"failed_requests\":[]}\n```",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "invalid_url_or_sandbox_guard",
                    "Disallowed URL scheme, file:// or relative path escaping workspace root, missing local workspace file, or path traversal in screenshot_name",
                    "Invalid URL '<url>' / Sandbox violation / Local target '<url>' does not exist in workspace / Invalid screenshot_name: path separators and '..' are forbidden",
                    "Use http://, https://, about:, data:, a workspace-relative path (e.g. 'index.html'), or a file:/// URL strictly inside the workspace (http_check requires http:// or https://), and alphanumeric/'_'/'-' characters for screenshot_name.",
                ),
                ToolFailureMode::new(
                    "browser_unavailable_or_cdp_error",
                    "Headless Edge/Chrome/Chromium not installed or CDP WebSocket connection failed",
                    "No headless Chromium/Edge binary found or CDP handshake failed",
                    "Use action='http_check' for non-JS HTTP verification or ensure Edge/Chrome is installed.",
                ),
                ToolFailureMode::new(
                    "selector_or_assertion_failed",
                    "Target CSS selector, ref_id, or visible text missing, or assert_text/assert_selector/assert_status check failed",
                    "Selector '<selector>' / ref_id not found or assertion mismatch",
                    "Run action='snapshot' or action='inspect_dom' to discover live ref_ids and CSS selectors.",
                ),
                ToolFailureMode::new(
                    "invalid_step_or_storage_args",
                    "Missing or invalid workflow step parameters or storage_key/value arguments",
                    "steps[i] is missing 'interaction'/'action' or storage_op requires 'storage_key'",
                    "Provide valid step objects and required target/value fields per the tool contract.",
                ),
                ToolFailureMode::new(
                    "timeout",
                    "Navigation, script evaluation, wait_for_selector, or interaction workflow exceeded timeout_secs",
                    "Browser action timed out after <N>s",
                    "Ensure the target URL is responsive and increase timeout_secs (up to 90).",
                ),
            ],
            timeout_secs: 25,
            is_custom: false,
        }
    }

    fn parameter_schema(&self) -> Value {
        let mut schema = self.contract().to_json_schema();
        let step_items = json!({
            "type": "object",
            "properties": {
                "interaction": {
                    "type": "string",
                    "enum": [
                        "click",
                        "dblclick",
                        "fill",
                        "type",
                        "press_key",
                        "select_option",
                        "select",
                        "hover",
                        "wait_for_selector",
                        "wait_ms",
                        "scroll_by",
                        "scroll_to",
                        "eval",
                        "screenshot",
                        "assert_text",
                        "assert_selector"
                    ]
                },
                "action": { "type": "string" },
                "selector": { "type": "string" },
                "ref_id": { "type": "integer", "minimum": 1 },
                "text": { "type": "string" },
                "value": { "type": "string" },
                "expression": { "type": "string" },
                "wait_ms": { "type": "integer", "minimum": 0 },
                "x": {
                    "type": "number",
                    "description": "X coordinate in CSS px: viewport X when no selector/ref_id/text is given, or element-relative X offset from element top-left when combined with selector/ref_id/text."
                },
                "y": {
                    "type": "number",
                    "description": "Y coordinate in CSS px: viewport Y when no selector/ref_id/text is given, or element-relative Y offset from element top-left when combined with selector/ref_id/text."
                },
                "screenshot_name": { "type": "string" },
                "full_page": { "type": "boolean" },
                "element_selector": { "type": "string" }
            }
        });
        if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
            for field_name in ["steps", "actions"] {
                if let Some(arr_prop) = props.get_mut(field_name).and_then(|s| s.as_object_mut()) {
                    arr_prop.insert("items".to_string(), step_items.clone());
                }
            }
        }
        schema
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'action'".to_string())?;

        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'url'".to_string())?
            .trim();

        validate_url(url, action == "http_check")?;

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(25)
            .clamp(3, 90);

        match action {
            "http_check" => execute_http_check(url, &args, timeout_secs).await,
            "navigate" | "screenshot" | "eval" | "inspect_dom" | "interact" | "snapshot"
            | "debug_audit" | "storage" => {
                execute_browser_action(action, url, &args, sandbox, timeout_secs).await
            }
            other => Err(format!(
                "Unknown browser_ui action '{}'. Expected one of: navigate, screenshot, eval, inspect_dom, interact, snapshot, debug_audit, storage, http_check",
                other
            )),
        }
    }
}

fn validate_url(url: &str, http_only: bool) -> Result<(), String> {
    if url.is_empty() {
        return Err("Parameter 'url' must not be empty".to_string());
    }
    if url.contains('\0') {
        return Err("Invalid URL: null byte is forbidden".to_string());
    }
    let lower = url.to_ascii_lowercase();
    if http_only {
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(format!(
                "Invalid URL '{}' for http_check: must start with http:// or https://",
                url
            ));
        }
        return Ok(());
    }

    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("about:")
        || lower.starts_with("data:")
    {
        return Ok(());
    }

    let (base, _) = split_url_suffix(url);
    if base.to_ascii_lowercase().starts_with("file://") {
        parse_file_url_path(base)?;
        return Ok(());
    }

    parse_workspace_relative_path(base, url)?;
    Ok(())
}

fn split_url_suffix(url: &str) -> (&str, &str) {
    match url.find(['?', '#']) {
        Some(idx) => (&url[..idx], &url[idx..]),
        None => (url, ""),
    }
}

fn is_windows_drive_prefix(seg: &str) -> bool {
    let b = seg.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn looks_like_url_scheme(first_seg: &str) -> bool {
    let Some(colon_idx) = first_seg.find(':') else {
        return false;
    };
    let scheme = &first_seg[..colon_idx];
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    // Single-letter prefixes like `C:` are Windows drive prefixes, not URL schemes.
    scheme.len() >= 2
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn percent_decode_str(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(format!("Invalid percent-encoding in '{}'", input));
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .map_err(|_| format!("Invalid percent-encoding in '{}'", input))?;
            let val = u8::from_str_radix(hex, 16)
                .map_err(|_| format!("Invalid percent-encoding '%{}' in '{}'", hex, input))?;
            if val == 0 {
                return Err("Null byte (%00) is forbidden in path".to_string());
            }
            out.push(val);
            i += 3;
        } else {
            if bytes[i] == 0 {
                return Err("Null byte is forbidden in path".to_string());
            }
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("Non-UTF-8 percent-encoded path in '{}'", input))
}

fn percent_encode_path_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for &b in segment.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn parse_file_url_path(raw_file_url: &str) -> Result<PathBuf, String> {
    let prefix = raw_file_url.get(..7).unwrap_or("");
    if !prefix.eq_ignore_ascii_case("file://") {
        return Err(format!("Invalid file:// URL '{}'", raw_file_url));
    }
    let rest = &raw_file_url[7..];
    if rest.is_empty() {
        return Err(format!("Invalid file:// URL '{}': missing path", raw_file_url));
    }

    let path_part = if rest.starts_with('/') || rest.starts_with('\\') {
        rest
    } else {
        let (host_raw, tail) = match rest.find(['/', '\\']) {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, ""),
        };
        if host_raw.eq_ignore_ascii_case("localhost") {
            if tail.is_empty() {
                return Err(format!(
                    "Invalid file:// URL '{}': missing path after localhost",
                    raw_file_url
                ));
            }
            tail
        } else if cfg!(windows) {
            let host_decoded = percent_decode_str(host_raw)?;
            if is_windows_drive_prefix(&host_decoded) && !tail.is_empty() {
                rest
            } else {
                return Err(format!(
                    "Sandbox violation: remote or invalid file:// host '{}' in '{}'",
                    host_raw, raw_file_url
                ));
            }
        } else {
            return Err(format!(
                "Sandbox violation: remote or invalid file:// host '{}' in '{}'",
                host_raw, raw_file_url
            ));
        }
    };

    let mut decoded = percent_decode_str(path_part)?;
    if decoded.contains('\0') {
        return Err("Null byte is forbidden in file:// URL".to_string());
    }

    #[cfg(windows)]
    {
        let b = decoded.as_bytes();
        if b.len() >= 3
            && (b[0] == b'/' || b[0] == b'\\')
            && b[1].is_ascii_alphabetic()
            && b[2] == b':'
        {
            decoded.remove(0);
        }
        decoded = decoded.replace('/', "\\");
    }

    if decoded.is_empty() {
        return Err(format!("Invalid file:// URL '{}': empty path", raw_file_url));
    }
    Ok(PathBuf::from(decoded))
}

fn parse_workspace_relative_path(base: &str, full_url: &str) -> Result<PathBuf, String> {
    if base.is_empty() {
        return Err(format!("Invalid URL '{}': empty path", full_url));
    }
    if base.starts_with('/') || base.starts_with('\\') {
        return Err(format!(
            "Invalid URL '{}': absolute filesystem paths are not allowed; use a workspace-relative path (e.g. 'index.html') or a file:/// URL inside the workspace",
            full_url
        ));
    }

    let normalized_slashes = base.replace('\\', "/");
    let first_seg = normalized_slashes.split('/').next().unwrap_or("");
    if is_windows_drive_prefix(first_seg)
        || (first_seg.len() >= 2
            && first_seg.as_bytes()[0].is_ascii_alphabetic()
            && first_seg.as_bytes()[1] == b':')
    {
        return Err(format!(
            "Invalid URL '{}': drive paths are not allowed directly; use a workspace-relative path (e.g. 'index.html') or a file:/// URL inside the workspace",
            full_url
        ));
    }
    if looks_like_url_scheme(first_seg) {
        return Err(format!(
            "Invalid URL '{}': must start with http://, https://, about:, data:, a workspace file:/// URL, or a workspace-relative path",
            full_url
        ));
    }

    let mut rel_buf = PathBuf::new();
    for seg in normalized_slashes.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(format!(
                "Sandbox violation: '..' path traversal is forbidden in workspace path '{}'",
                full_url
            ));
        }
        rel_buf.push(seg);
    }

    Ok(rel_buf)
}

async fn check_nonexistent_inside_sandbox(
    candidate: &Path,
    canon_sandbox: &Path,
) -> Result<(), String> {
    let mut normalized = PathBuf::new();
    for comp in candidate.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(std::path::Component::Normal(_))
                ) {
                    normalized.pop();
                } else {
                    return Err(format!(
                        "Sandbox violation: '{}' resolves outside workspace root '{}'",
                        candidate.display(),
                        canon_sandbox.display()
                    ));
                }
            }
            other => normalized.push(other),
        }
    }

    let mut probe = normalized.as_path();
    loop {
        if tokio::fs::try_exists(probe).await.unwrap_or(false) {
            let canon_ancestor = tokio::fs::canonicalize(probe)
                .await
                .map_err(|e| format!("Failed to resolve path '{}': {}", probe.display(), e))?;
            if !canon_ancestor.starts_with(canon_sandbox) {
                return Err(format!(
                    "Sandbox violation: '{}' resolves outside workspace root '{}'",
                    candidate.display(),
                    canon_sandbox.display()
                ));
            }
            return Ok(());
        }
        match probe.parent() {
            Some(p) if !p.as_os_str().is_empty() => probe = p,
            _ => {
                return Err(format!(
                    "Sandbox violation: '{}' resolves outside workspace root '{}'",
                    candidate.display(),
                    canon_sandbox.display()
                ));
            }
        }
    }
}

fn mime_type_for_path(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Ephemeral read-only loopback HTTP/1.1 static server scoped strictly to `canon_sandbox`.
/// Automatically signals shutdown and aborts its accept task on drop.
struct EphemeralWorkspaceServer {
    port: u16,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    accept_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for EphemeralWorkspaceServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(handle) = self.accept_handle.take() {
            handle.abort();
        }
    }
}

impl EphemeralWorkspaceServer {
    async fn start(canon_sandbox: PathBuf) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind ephemeral loopback HTTP server: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to read ephemeral server address: {}", e))?
            .port();

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let conn_shutdown_rx = shutdown_rx.clone();

        let accept_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                let root = canon_sandbox.clone();
                                let rx = conn_shutdown_rx.clone();
                                tokio::spawn(async move {
                                    serve_ephemeral_http_connection(stream, root, rx).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok(Self {
            port,
            shutdown_tx: Some(shutdown_tx),
            accept_handle: Some(accept_handle),
        })
    }

    fn port(&self) -> u16 {
        self.port
    }
}

async fn read_http_request_head(
    stream: &mut TcpStream,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Option<String> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if *shutdown_rx.borrow() {
            return None;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return None;
        }
        let remaining = deadline - now;
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return None;
                }
            }
            res = timeout(remaining, stream.read(&mut chunk)) => {
                match res {
                    Ok(Ok(0)) => {
                        if buf.is_empty() {
                            return None;
                        }
                        break;
                    }
                    Ok(Ok(n)) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n")
                            || buf.windows(2).any(|w| w == b"\n\n")
                        {
                            break;
                        }
                        if buf.len() >= 16 * 1024 {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

async fn write_http_response(
    stream: &mut TcpStream,
    status_line: &str,
    content_type: &str,
    extra_headers: &str,
    body: &[u8],
    send_body: bool,
) {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{}Connection: close\r\n\r\n",
        status_line,
        content_type,
        body.len(),
        extra_headers
    );
    if stream.write_all(header.as_bytes()).await.is_ok() && send_body && !body.is_empty() {
        let _ = stream.write_all(body).await;
    }
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

async fn serve_ephemeral_http_connection(
    mut stream: TcpStream,
    canon_sandbox: PathBuf,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let Some(head) = read_http_request_head(&mut stream, &mut shutdown_rx).await else {
        return;
    };

    let first_line = head.lines().next().unwrap_or("").trim();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_target = parts.next().unwrap_or("");

    if method.is_empty() || raw_target.is_empty() {
        write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            "",
            b"Bad Request",
            true,
        )
        .await;
        return;
    }

    if method != "GET" && method != "HEAD" {
        write_http_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "Allow: GET, HEAD\r\n",
            b"Method Not Allowed",
            true,
        )
        .await;
        return;
    }
    let send_body = method == "GET";

    let path_only = if let Some(rest) = raw_target
        .strip_prefix("http://")
        .or_else(|| raw_target.strip_prefix("https://"))
    {
        match rest.find('/') {
            Some(idx) => &rest[idx..],
            None => "/",
        }
    } else {
        raw_target
    };
    let path_no_frag = path_only.split('#').next().unwrap_or(path_only);
    let path_no_query = path_no_frag.split('?').next().unwrap_or(path_no_frag);

    let decoded = match percent_decode_str(path_no_query) {
        Ok(d) => d,
        Err(_) => {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                "",
                b"Bad Request",
                send_body,
            )
            .await;
            return;
        }
    };

    if decoded.contains('\0') || decoded.contains('\\') {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            "",
            b"Forbidden",
            send_body,
        )
        .await;
        return;
    }

    let mut rel_path = PathBuf::new();
    for seg in decoded.trim_start_matches('/').split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            write_http_response(
                &mut stream,
                "403 Forbidden",
                "text/plain; charset=utf-8",
                "",
                b"Forbidden",
                send_body,
            )
            .await;
            return;
        }
        rel_path.push(seg);
    }

    // Harness-internal state ('.harness', '.harness_artifacts') and any other
    // leading-dot path must never be served, even though they live inside the
    // sandbox directory.
    if is_hidden_harness_path(&rel_path) {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            "",
            b"Forbidden",
            send_body,
        )
        .await;
        return;
    }

    if rel_path.as_os_str().is_empty() {
        rel_path.push("index.html");
    }

    let candidate = canon_sandbox.join(&rel_path);
    let mut canon_file = match tokio::fs::canonicalize(&candidate).await {
        Ok(p) => p,
        Err(_) => {
            // Browsers automatically request /favicon.ico; answer 204 when absent so clean apps avoid spurious 404 console errors.
            if rel_path == std::path::Path::new("favicon.ico") {
                write_http_response(
                    &mut stream,
                    "204 No Content",
                    "image/x-icon",
                    "",
                    b"",
                    false,
                )
                .await;
                return;
            }
            write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                "",
                b"Not Found",
                send_body,
            )
            .await;
            return;
        }
    };

    if !canon_file.starts_with(&canon_sandbox) {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            "",
            b"Forbidden",
            send_body,
        )
        .await;
        return;
    }

    let Ok(meta) = tokio::fs::metadata(&canon_file).await else {
        write_http_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "",
            b"Not Found",
            send_body,
        )
        .await;
        return;
    };

    if meta.is_dir() {
        let index_candidate = canon_file.join("index.html");
        match tokio::fs::canonicalize(&index_candidate).await {
            Ok(p) if p.starts_with(&canon_sandbox) && p.is_file() => {
                canon_file = p;
            }
            _ => {
                write_http_response(
                    &mut stream,
                    "404 Not Found",
                    "text/plain; charset=utf-8",
                    "",
                    b"Not Found",
                    send_body,
                )
                .await;
                return;
            }
        }
    } else if !meta.is_file() {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            "",
            b"Forbidden",
            send_body,
        )
        .await;
        return;
    }

    let mut file = match tokio::fs::File::open(&canon_file).await {
        Ok(f) => f,
        Err(_) => {
            write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                "",
                b"Not Found",
                send_body,
            )
            .await;
            return;
        }
    };

    match read_to_end_bounded(&mut file, MAX_HTTP_BODY_BYTES).await {
        Ok(bytes) => {
            let mime = mime_type_for_path(&canon_file);
            write_http_response(&mut stream, "200 OK", mime, "", &bytes, send_body).await;
        }
        Err(_) => {
            write_http_response(
                &mut stream,
                "413 Content Too Large",
                "text/plain; charset=utf-8",
                "",
                b"Payload Too Large",
                send_body,
            )
            .await;
        }
    }
}

/// Resolves `url` for CDP browser actions:
/// - `http://`, `https://`, `about:`, `data:` pass through unchanged with no local server.
/// - Workspace `file://` URLs and workspace-relative paths are canonicalized strictly inside `sandbox`
///   and served via an ephemeral read-only loopback HTTP server (`http://127.0.0.1:<port>/<rel_path>`).
async fn resolve_target_url(
    url: &str,
    sandbox: &Path,
) -> Result<(String, Option<EphemeralWorkspaceServer>), String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("about:")
        || lower.starts_with("data:")
    {
        return Ok((url.to_string(), None));
    }

    let canon_sandbox = tokio::fs::canonicalize(sandbox)
        .await
        .map_err(|e| format!("Failed to canonicalize sandbox '{}': {}", sandbox.display(), e))?;

    let (base, suffix) = split_url_suffix(url);
    let candidate = if base.to_ascii_lowercase().starts_with("file://") {
        parse_file_url_path(base)?
    } else {
        let rel_buf = parse_workspace_relative_path(base, url)?;
        if rel_buf.as_os_str().is_empty() {
            canon_sandbox.join("index.html")
        } else {
            canon_sandbox.join(&rel_buf)
        }
    };

    let mut canon_target = match tokio::fs::canonicalize(&candidate).await {
        Ok(p) => p,
        Err(e) => {
            check_nonexistent_inside_sandbox(&candidate, &canon_sandbox).await?;
            return Err(format!(
                "Local target '{}' does not exist in workspace: {}",
                url, e
            ));
        }
    };

    if !canon_target.starts_with(&canon_sandbox) {
        return Err(format!(
            "Sandbox violation: '{}' resolves outside workspace root '{}'",
            url,
            canon_sandbox.display()
        ));
    }

    let meta = tokio::fs::metadata(&canon_target)
        .await
        .map_err(|e| format!("Failed to inspect local target '{}': {}", url, e))?;
    if meta.is_dir() {
        let index_candidate = canon_target.join("index.html");
        canon_target = tokio::fs::canonicalize(&index_candidate).await.map_err(|_| {
            format!(
                "Local target '{}' is a directory without index.html in workspace",
                url
            )
        })?;
        if !canon_target.starts_with(&canon_sandbox) {
            return Err(format!(
                "Sandbox violation: '{}/index.html' resolves outside workspace root",
                url
            ));
        }
        let idx_meta = tokio::fs::metadata(&canon_target)
            .await
            .map_err(|e| format!("Failed to inspect '{}/index.html': {}", url, e))?;
        if !idx_meta.is_file() {
            return Err(format!(
                "Local target '{}/index.html' is not a regular file",
                url
            ));
        }
    } else if !meta.is_file() {
        return Err(format!("Local target '{}' is not a regular file", url));
    }

    let rel = canon_target
        .strip_prefix(&canon_sandbox)
        .map_err(|_| format!("Sandbox violation: '{}' is outside workspace root", url))?;
    let mut encoded_segments = Vec::new();
    for comp in rel.components() {
        if let std::path::Component::Normal(seg) = comp {
            let s = seg.to_string_lossy();
            encoded_segments.push(percent_encode_path_segment(&s));
        }
    }
    let rel_url_path = encoded_segments.join("/");

    let server = EphemeralWorkspaceServer::start(canon_sandbox).await?;
    let mapped_url = format!(
        "http://127.0.0.1:{}/{}{}",
        server.port(),
        rel_url_path,
        suffix
    );
    Ok((mapped_url, Some(server)))
}

/// Tight sandbox validation: screenshots may ONLY be written inside `<sandbox>/.harness_artifacts/screenshots/`.
async fn resolve_screenshot_path(
    sandbox: &Path,
    custom_name: Option<&str>,
) -> Result<(PathBuf, String, String), String> {
    if !sandbox.exists() {
        tokio::fs::create_dir_all(sandbox)
            .await
            .map_err(|e| format!("Failed to access sandbox root '{}': {}", sandbox.display(), e))?;
    }

    let canon_sandbox = tokio::fs::canonicalize(sandbox)
        .await
        .map_err(|e| format!("Failed to canonicalize sandbox '{}': {}", sandbox.display(), e))?;

    let screenshots_dir = canon_sandbox.join(".harness_artifacts").join("screenshots");
    tokio::fs::create_dir_all(&screenshots_dir)
        .await
        .map_err(|e| format!("Failed to create screenshots directory: {}", e))?;

    let canon_screenshots = tokio::fs::canonicalize(&screenshots_dir)
        .await
        .map_err(|e| format!("Failed to canonicalize screenshots directory: {}", e))?;

    if !canon_screenshots.starts_with(&canon_sandbox) {
        return Err("Sandbox violation: screenshots directory escaped workspace root".to_string());
    }

    let filename = match custom_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => {
            if raw.contains("..") || raw.contains('/') || raw.contains('\\') || raw.contains('\0') {
                return Err(format!(
                    "Invalid screenshot_name '{}': path separators and '..' are forbidden",
                    raw
                ));
            }
            let stem = raw.strip_suffix(".png").unwrap_or(raw);
            if stem.is_empty()
                || !stem
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "Invalid screenshot_name '{}': only ASCII letters, digits, '_', and '-' are allowed",
                    raw
                ));
            }
            format!("{}.png", stem)
        }
        None => {
            let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
            format!("shot_{}_{}.png", Utc::now().format("%Y%m%d_%H%M%S_%3f"), seq)
        }
    };

    let full_path = canon_screenshots.join(&filename);
    if full_path.parent() != Some(canon_screenshots.as_path()) {
        return Err("Sandbox violation: resolved screenshot path left screenshots directory".to_string());
    }

    let rel_path = format!(".harness_artifacts/screenshots/{}", filename);
    let url_path = format!("/api/artifacts/screenshots/{}", filename);
    Ok((full_path, rel_path, url_path))
}

/// Lazily locate Edge/Chrome/Chromium on Windows, macOS, or Linux.
pub fn discover_browser() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files\Chromium\Application\chrome.exe"),
        ];
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(local_app_data);
            candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
            candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            candidates.push(base.join(r"Chromium\Application\chrome.exe"));
        }
        for path in candidates {
            if path.is_file() {
                return Some(path);
            }
        }
        return find_in_path(&["msedge.exe", "chrome.exe", "chromium.exe", "msedge", "chrome", "chromium"]);
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for p in candidates {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some(path);
            }
        }
        return find_in_path(&["msedge", "google-chrome", "chrome", "chromium"]);
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(p) = find_in_path(&[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "msedge",
            "microsoft-edge",
            "chrome",
        ]) {
            return Some(p);
        }
        let candidates = [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
            "/snap/bin/chromium",
            "/opt/google/chrome/chrome",
            "/opt/microsoft/msedge/msedge",
        ];
        for p in candidates {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some(path);
            }
        }
        None
    }
}

/// Prefer Google Chrome (then Chromium, then Edge) when launching Chrome Incognito sessions
/// such as Google Search AI Mode (`udm=50`).
pub(crate) fn discover_chrome_browser() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files\Chromium\Application\chrome.exe"),
        ];
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(local_app_data);
            candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            candidates.push(base.join(r"Chromium\Application\chrome.exe"));
        }
        for path in candidates {
            if path.is_file() {
                return Some(path);
            }
        }
        return find_in_path(&["chrome.exe", "chrome", "chromium.exe", "chromium"])
            .or_else(discover_browser);
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for p in candidates {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some(path);
            }
        }
        return find_in_path(&["google-chrome", "chrome", "chromium"])
            .or_else(discover_browser);
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        return find_in_path(&[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "chrome",
        ])
        .or_else(discover_browser);
    }
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ============================================================================
// HTTP Check (Zero Browser Overhead)
// ============================================================================

async fn execute_http_check(url: &str, args: &Value, timeout_secs: u64) -> Result<String, String> {
    let mut header_map = HeaderMap::new();
    header_map.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static("DeepSeek-Harness-BrowserUI/0.1.0"),
    );

    if let Some(obj) = args.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(val_str) = v.as_str() {
                if let (Ok(hn), Ok(hv)) = (
                    HeaderName::from_bytes(k.as_bytes()),
                    HeaderValue::from_str(val_str),
                ) {
                    header_map.insert(hn, hv);
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Failed to construct HTTP client: {}", e))?;

    let start = Instant::now();
    let res = client
        .get(url)
        .headers(header_map)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed for '{}': {}", url, e))?;
    let elapsed_ms = start.elapsed().as_millis() as u64;

    let status = res.status().as_u16();
    let final_url = res.url().to_string();
    let mut resp_headers = serde_json::Map::new();
    for (k, v) in res.headers() {
        if let Ok(val) = v.to_str() {
            resp_headers.insert(k.to_string(), Value::String(val.to_string()));
        }
    }

    let (body, body_truncated) = read_body_bounded(res, MAX_HTTP_BODY_BYTES)
        .await
        .map_err(|e| format!("Failed to read HTTP body from '{}': {}", url, e))?;

    let title = extract_html_title(&body).unwrap_or_default();
    let visible_text = truncate_chars(&strip_html_to_text(&body), 2000);

    let mut assertions = Vec::new();
    let mut all_passed = true;

    if let Some(expected_status) = args.get("assert_status").and_then(|v| v.as_u64()) {
        let passed = status as u64 == expected_status;
        if !passed {
            all_passed = false;
        }
        assertions.push(json!({
            "type": "status",
            "expected": expected_status,
            "actual": status,
            "passed": passed
        }));
    }

    if let Some(expected_text) = args.get("assert_text").and_then(|v| v.as_str()) {
        let plain = strip_html_to_text(&body);
        let passed = plain.contains(expected_text) || body.contains(expected_text);
        if !passed {
            all_passed = false;
        }
        assertions.push(json!({
            "type": "text",
            "expected": expected_text,
            "passed": passed
        }));
    }

    let selector_matches = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(|sel| count_html_selector_matches(&body, sel));

    if let Some(count) = selector_matches {
        assertions.push(json!({
            "type": "selector_exists",
            "selector": args.get("selector").and_then(|v| v.as_str()).unwrap_or(""),
            "count": count,
            "passed": count > 0
        }));
        if count == 0 {
            all_passed = false;
        }
    }

    let dom_summary = format!(
        "HTTP {} in {}ms | title='{}' | {} bytes{}",
        status,
        elapsed_ms,
        title,
        body.len(),
        selector_matches
            .map(|c| format!(" | selector_matches={}", c))
            .unwrap_or_default()
            + if body_truncated {
                " | body_truncated=8MB"
            } else {
                ""
            }
    );

    let summary_json = json!({
        "action": "http_check",
        "mode": "reqwest",
        "url": final_url,
        "title": title,
        "http_status": status,
        "response_time_ms": elapsed_ms,
        "content_length": body.len(),
        "headers": Value::Object(resp_headers),
        "visible_text": visible_text,
        "selector_matches": selector_matches,
        "assertions": assertions,
        "assertions_passed": all_passed,
        "console_errors": Vec::<String>::new(),
        "screenshot_url": Value::Null,
        "dom_summary": dom_summary
    });

    Ok(format_tool_output("http_check", &summary_json))
}

// ============================================================================
// Browser Action Orchestrator (CDP Primary + --dump-dom / reqwest Fallback)
// ============================================================================

#[derive(Debug, Clone)]
struct ViewportConfig {
    width: u32,
    height: u32,
    preset: Option<String>,
    is_mobile: bool,
}

fn resolve_viewport(args: &Value) -> Result<ViewportConfig, String> {
    let preset = args
        .get("preset")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (preset_w, preset_h, is_mobile) = match preset.as_deref() {
        Some("desktop") => (1440u32, 900u32, false),
        Some("laptop") => (1280u32, 800u32, false),
        Some("tablet") => (768u32, 1024u32, true),
        Some("mobile") => (390u32, 844u32, true),
        Some(other) => {
            return Err(format!(
                "Invalid viewport preset '{}'. Expected one of: desktop, laptop, tablet, mobile",
                other
            ));
        }
        None => (DEFAULT_WIDTH, DEFAULT_HEIGHT, false),
    };

    let width = args
        .get("width")
        .and_then(|v| v.as_u64())
        .unwrap_or(preset_w as u64)
        .clamp(100, 7680) as u32;
    let height = args
        .get("height")
        .and_then(|v| v.as_u64())
        .unwrap_or(preset_h as u64)
        .clamp(100, 4320) as u32;

    Ok(ViewportConfig {
        width,
        height,
        preset,
        is_mobile,
    })
}

async fn execute_browser_action(
    action: &str,
    url: &str,
    args: &Value,
    sandbox: &Path,
    timeout_secs: u64,
) -> Result<String, String> {
    let viewport = resolve_viewport(args)?;
    let full_page = args
        .get("full_page")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let element_selector = args
        .get("element_selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let default_wait = if action == "interact" { 300 } else { 800 };
    let wait_ms = args
        .get("wait_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(default_wait)
        .clamp(0, 15000);
    let want_screenshot = action == "screenshot"
        || args
            .get("screenshot")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let custom_shot_name = args.get("screenshot_name").and_then(|v| v.as_str());

    // Resolve workspace file:// URLs or relative paths into an ephemeral loopback HTTP server
    // scoped strictly to canonicalized `sandbox`; dropped automatically when this call returns.
    let (effective_url, _ephemeral_server) = resolve_target_url(url, sandbox).await?;

    // Pre-validate screenshot path before launching any browser process!
    let screenshot_target = if want_screenshot {
        Some(resolve_screenshot_path(sandbox, custom_shot_name).await?)
    } else {
        None
    };

    let has_steps = args
        .get("steps")
        .or_else(|| args.get("actions"))
        .and_then(|v| v.as_array())
        .is_some();

    let browser_path = match discover_browser() {
        Some(p) => p,
        None => {
            return execute_no_browser_fallback(action, &effective_url, args, timeout_secs).await;
        }
    };

    let cdp_res = run_cdp_action(
        &browser_path,
        action,
        &effective_url,
        args,
        sandbox,
        &viewport,
        wait_ms,
        full_page,
        element_selector,
        screenshot_target.as_ref(),
        timeout_secs,
    )
    .await;

    match cdp_res {
        Ok(summary_json) => Ok(format_tool_output(action, &summary_json)),
        Err(cdp_err) => {
            if cdp_err.contains("timed out after") {
                return Err(cdp_err);
            }
            // Fallback for simple read-only / screenshot actions if CDP port wasn't available
            if matches!(action, "navigate" | "inspect_dom" | "screenshot")
                && !has_steps
                && element_selector.is_none()
            {
                if let Ok(fallback) = run_cli_browser_fallback(
                    &browser_path,
                    action,
                    &effective_url,
                    args,
                    viewport.width,
                    viewport.height,
                    screenshot_target.as_ref(),
                    timeout_secs,
                    &cdp_err,
                )
                .await
                {
                    return Ok(fallback);
                }
            }
            Err(cdp_err)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_cdp_action(
    browser_path: &Path,
    action: &str,
    url: &str,
    args: &Value,
    sandbox: &Path,
    viewport: &ViewportConfig,
    wait_ms: u64,
    full_page: bool,
    element_selector: Option<&str>,
    screenshot_target: Option<&(PathBuf, String, String)>,
    timeout_secs: u64,
) -> Result<Value, String> {
    let total_budget = Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + total_budget;

    let mut session = match timeout(
        total_budget,
        CdpBrowserSession::launch(browser_path, viewport.width, viewport.height),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(format!(
                "browser_ui action '{}' timed out after {}s while launching browser for '{}' (browser process cleaned up)",
                action, timeout_secs, url
            ));
        }
    };

    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let outcome = if remaining.is_zero() {
        Err(format!(
            "browser_ui action '{}' timed out after {}s on '{}' (browser process cleaned up)",
            action, timeout_secs, url
        ))
    } else {
        match timeout(
            remaining,
            execute_cdp_steps(
                &mut session,
                action,
                url,
                args,
                sandbox,
                viewport,
                wait_ms,
                full_page,
                element_selector,
                screenshot_target,
            ),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => Err(format!(
                "browser_ui action '{}' timed out after {}s on '{}' (browser process cleaned up)",
                action, timeout_secs, url
            )),
        }
    };

    // Always cleanly close the CDP session and kill the entire Chromium process tree before returning.
    session.close().await;
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn execute_cdp_steps(
    session: &mut CdpBrowserSession,
    action: &str,
    url: &str,
    args: &Value,
    sandbox: &Path,
    viewport: &ViewportConfig,
    wait_ms: u64,
    full_page: bool,
    element_selector: Option<&str>,
    screenshot_target: Option<&(PathBuf, String, String)>,
) -> Result<Value, String> {
    session.enable_domains(viewport).await?;
    session.navigate(url, wait_ms).await?;

    let mut extra = serde_json::Map::new();
    let mut dom_summary = String::new();
    let mut step_screenshots: Vec<Value> = Vec::new();

    let has_workflow_steps = args
        .get("steps")
        .or_else(|| args.get("actions"))
        .is_some();

    match action {
        "navigate" => {
            if has_workflow_steps {
                let steps = parse_interaction_steps(args, 300)?;
                let (step_results, all_assertions_passed, shots) = session
                    .run_workflow_steps(&steps, sandbox, full_page, element_selector)
                    .await?;
                step_screenshots.extend(shots);
                extra.insert("interaction_results".into(), Value::Array(step_results));
                extra.insert("assertions_passed".into(), Value::Bool(all_assertions_passed));
            }
            let inv = session.collect_page_inventory().await?;
            let title = inv.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let final_url = inv.get("url").and_then(|v| v.as_str()).unwrap_or(url).to_string();
            let counts = inv.get("counts").cloned().unwrap_or(json!({}));
            dom_summary = if let Some(results) = extra.get("interaction_results").and_then(|v| v.as_array()) {
                format!(
                    "Loaded '{}' ({}) and executed {} workflow step(s) | links={} buttons={} inputs={} | console_errors={}",
                    title,
                    final_url,
                    results.len(),
                    counts.get("links").and_then(|v| v.as_u64()).unwrap_or(0),
                    counts.get("buttons").and_then(|v| v.as_u64()).unwrap_or(0),
                    counts.get("inputs").and_then(|v| v.as_u64()).unwrap_or(0),
                    session.console_errors.len()
                )
            } else {
                format!(
                    "Loaded '{}' ({}) | links={} buttons={} inputs={} | console_errors={}",
                    title,
                    final_url,
                    counts.get("links").and_then(|v| v.as_u64()).unwrap_or(0),
                    counts.get("buttons").and_then(|v| v.as_u64()).unwrap_or(0),
                    counts.get("inputs").and_then(|v| v.as_u64()).unwrap_or(0),
                    session.console_errors.len()
                )
            };
            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            if let Some(vt) = inv.get("visibleText") {
                extra.insert("visible_text".into(), vt.clone());
            }
            extra.insert("counts".into(), counts);
            if let Some(links) = inv.get("links") {
                extra.insert("links".into(), links.clone());
            }
            if let Some(buttons) = inv.get("buttons") {
                extra.insert("buttons".into(), buttons.clone());
            }
            if let Some(inputs) = inv.get("inputs") {
                extra.insert("inputs".into(), inputs.clone());
            }
        }
        "screenshot" => {
            if has_workflow_steps {
                let steps = parse_interaction_steps(args, 300)?;
                let (step_results, all_assertions_passed, shots) = session
                    .run_workflow_steps(&steps, sandbox, full_page, element_selector)
                    .await?;
                step_screenshots.extend(shots);
                extra.insert("interaction_results".into(), Value::Array(step_results));
                extra.insert("assertions_passed".into(), Value::Bool(all_assertions_passed));
            }
            let meta = session.collect_basic_meta().await.unwrap_or_else(|_| json!({}));
            let title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let final_url = meta.get("url").and_then(|v| v.as_str()).unwrap_or(url).to_string();
            let clip_note = element_selector
                .map(|s| format!(" element '{}'", s))
                .unwrap_or_default();
            dom_summary = format!(
                "Captured {}x{}{}{} screenshot of '{}' ({}) | console_errors={}",
                viewport.width,
                viewport.height,
                if full_page { " full-page" } else { "" },
                clip_note,
                title,
                final_url,
                session.console_errors.len()
            );
            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            extra.insert(
                "viewport".into(),
                json!({
                    "width": viewport.width,
                    "height": viewport.height,
                    "preset": viewport.preset,
                    "is_mobile": viewport.is_mobile,
                    "full_page": full_page,
                    "element_selector": element_selector
                }),
            );
        }
        "eval" => {
            let expr = args
                .get("expression")
                .or_else(|| args.get("script"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Action 'eval' requires 'expression' parameter".to_string())?;
            let eval_val = session.eval_js(expr).await?;
            let meta = session.collect_basic_meta().await.unwrap_or_else(|_| json!({}));
            let title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let final_url = meta.get("url").and_then(|v| v.as_str()).unwrap_or(url).to_string();
            dom_summary = format!(
                "Evaluated JS on '{}' | console_errors={}",
                final_url,
                session.console_errors.len()
            );
            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            extra.insert("eval_result".into(), eval_val);
        }
        "inspect_dom" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or("a, button, input, select, textarea, h1, h2, h3, [role='button']");
            let inspect = session.inspect_dom(selector).await?;
            let title = inspect.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let final_url = inspect.get("url").and_then(|v| v.as_str()).unwrap_or(url).to_string();
            let count = inspect.get("totalCount").and_then(|v| v.as_u64()).unwrap_or(0);
            dom_summary = format!(
                "Inspected '{}' -> {} element(s) matched on '{}' | console_errors={}",
                selector,
                count,
                final_url,
                session.console_errors.len()
            );
            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            extra.insert("selector".into(), Value::String(selector.to_string()));
            extra.insert("total_count".into(), Value::from(count));
            if let Some(elems) = inspect.get("elements") {
                extra.insert("elements".into(), elems.clone());
            }
        }
        "interact" => {
            let steps = parse_interaction_steps(args, wait_ms)?;
            let (step_results, all_assertions_passed, shots) = session
                .run_workflow_steps(&steps, sandbox, full_page, element_selector)
                .await?;
            step_screenshots.extend(shots);

            let inv = session.collect_page_inventory().await?;
            let title = inv.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let final_url = inv.get("url").and_then(|v| v.as_str()).unwrap_or(url).to_string();
            dom_summary = format!(
                "Executed {} interaction step(s) on '{}' ({}) | assertions_passed={} | console_errors={}",
                step_results.len(),
                title,
                final_url,
                all_assertions_passed,
                session.console_errors.len()
            );
            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            extra.insert("interaction_results".into(), Value::Array(step_results));
            extra.insert("assertions_passed".into(), Value::Bool(all_assertions_passed));
            if let Some(vt) = inv.get("visibleText") {
                extra.insert("visible_text".into(), vt.clone());
            }
        }
        "snapshot" => {
            if has_workflow_steps {
                let steps = parse_interaction_steps(args, 300)?;
                let (step_results, all_assertions_passed, shots) = session
                    .run_workflow_steps(&steps, sandbox, full_page, element_selector)
                    .await?;
                step_screenshots.extend(shots);
                extra.insert("interaction_results".into(), Value::Array(step_results));
                extra.insert("assertions_passed".into(), Value::Bool(all_assertions_passed));
            }
            let snap = session.collect_snapshot(120).await?;
            let page_title = snap
                .get("page_title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let final_url = snap
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or(url)
                .to_string();
            let overflow_x = snap
                .get("overflow_x")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let elements = snap
                .get("elements")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let elem_count = elements.as_array().map(|a| a.len()).unwrap_or(0);
            let total_interactive = snap
                .get("total_interactive_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(elem_count as u64);

            let mut vp_obj = snap.get("viewport").cloned().unwrap_or_else(|| json!({}));
            if let Some(m) = vp_obj.as_object_mut() {
                m.insert("configured_width".into(), json!(viewport.width));
                m.insert("configured_height".into(), json!(viewport.height));
                m.insert("preset".into(), json!(viewport.preset));
                m.insert("is_mobile".into(), json!(viewport.is_mobile));
            }

            dom_summary = format!(
                "Snapshot: {} interactive element(s) (of {}) on '{}' ({}) | overflow_x={} | console_errors={} | failed_requests={}",
                elem_count,
                total_interactive,
                page_title,
                final_url,
                overflow_x,
                session.console_errors.len(),
                session.failed_requests.len()
            );

            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(page_title.clone()));
            extra.insert("page_title".into(), Value::String(page_title));
            extra.insert("viewport".into(), vp_obj);
            extra.insert("overflow_x".into(), Value::Bool(overflow_x));
            extra.insert("element_count".into(), Value::from(elem_count));
            extra.insert("total_interactive_count".into(), Value::from(total_interactive));
            extra.insert("elements".into(), elements);
        }
        "debug_audit" => {
            if has_workflow_steps {
                let steps = parse_interaction_steps(args, 300)?;
                let (step_results, all_assertions_passed, shots) = session
                    .run_workflow_steps(&steps, sandbox, full_page, element_selector)
                    .await?;
                step_screenshots.extend(shots);
                extra.insert("interaction_results".into(), Value::Array(step_results));
                extra.insert("assertions_passed".into(), Value::Bool(all_assertions_passed));
            }
            let audit = session.collect_debug_audit().await?;
            let title = audit
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let final_url = audit
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or(url)
                .to_string();
            let broken_images = audit
                .get("broken_images")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let overflow = audit.get("overflow").cloned().unwrap_or_else(|| json!({}));
            let overflow_x = overflow
                .get("overflow_x")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let zero_size = audit
                .get("zero_size_controls")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let clipped = audit
                .get("clipped_controls")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let overlapping = audit
                .get("overlapping_controls")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let performance = audit.get("performance").cloned().unwrap_or_else(|| json!({}));

            let issues_count = session.console_errors.len()
                + session.uncaught_exceptions.len()
                + session.failed_requests.len()
                + broken_images.as_array().map(|a| a.len()).unwrap_or(0)
                + usize::from(overflow_x)
                + zero_size.as_array().map(|a| a.len()).unwrap_or(0)
                + clipped.as_array().map(|a| a.len()).unwrap_or(0)
                + overlapping.as_array().map(|a| a.len()).unwrap_or(0);
            let audit_passed = issues_count == 0;

            dom_summary = format!(
                "Debug audit on '{}' ({}): issues={} | console_errors={} warnings={} exceptions={} | failed_requests={} | broken_images={} overflow_x={} zero_size={} clipped={} overlapping={}",
                title,
                final_url,
                issues_count,
                session.console_errors.len(),
                session.console_warnings.len(),
                session.uncaught_exceptions.len(),
                session.failed_requests.len(),
                broken_images.as_array().map(|a| a.len()).unwrap_or(0),
                overflow_x,
                zero_size.as_array().map(|a| a.len()).unwrap_or(0),
                clipped.as_array().map(|a| a.len()).unwrap_or(0),
                overlapping.as_array().map(|a| a.len()).unwrap_or(0)
            );

            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            extra.insert("audit_passed".into(), Value::Bool(audit_passed));
            extra.insert("issues_count".into(), Value::from(issues_count));
            extra.insert(
                "console_warnings".into(),
                Value::Array(
                    session
                        .console_warnings
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
            extra.insert(
                "uncaught_exceptions".into(),
                Value::Array(session.uncaught_exceptions.clone()),
            );
            extra.insert("broken_images".into(), broken_images);
            extra.insert("overflow_x".into(), Value::Bool(overflow_x));
            extra.insert("overflow".into(), overflow);
            extra.insert("zero_size_controls".into(), zero_size);
            extra.insert("clipped_controls".into(), clipped);
            extra.insert("overlapping_controls".into(), overlapping);
            extra.insert("performance".into(), performance);
            extra.insert(
                "viewport".into(),
                json!({
                    "width": viewport.width,
                    "height": viewport.height,
                    "preset": viewport.preset,
                    "is_mobile": viewport.is_mobile
                }),
            );
        }
        "storage" => {
            let storage_res = session.execute_storage_action(url, args).await?;
            let title = storage_res
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let final_url = storage_res
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or(url)
                .to_string();
            let op = storage_res
                .get("storage_op")
                .and_then(|v| v.as_str())
                .unwrap_or("get");
            let counts = storage_res.get("counts").cloned().unwrap_or_else(|| json!({}));
            dom_summary = format!(
                "Storage ({}) on '{}' ({}) | localStorage={} sessionStorage={} cookies={}",
                op,
                title,
                final_url,
                counts.get("local_storage").and_then(|v| v.as_u64()).unwrap_or(0),
                counts.get("session_storage").and_then(|v| v.as_u64()).unwrap_or(0),
                counts.get("cookies").and_then(|v| v.as_u64()).unwrap_or(0)
            );
            extra.insert("final_url".into(), Value::String(final_url));
            extra.insert("title".into(), Value::String(title));
            if let Some(obj) = storage_res.as_object() {
                for (k, v) in obj {
                    if k != "title" && k != "url" {
                        extra.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        _ => {}
    }

    let (mut screenshot_path, mut screenshot_url) =
        if let Some((full_path, rel_path, url_path)) = screenshot_target {
            let png_bytes = session
                .capture_screenshot_png(full_page, element_selector)
                .await?;
            tokio::fs::write(full_path, &png_bytes)
                .await
                .map_err(|e| format!("Failed to write screenshot '{}': {}", rel_path, e))?;
            (Some(rel_path.clone()), Some(url_path.clone()))
        } else {
            (None, None)
        };

    if screenshot_path.is_none() {
        if let Some(last_shot) = step_screenshots.last() {
            screenshot_path = last_shot
                .get("screenshot_path")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            screenshot_url = last_shot
                .get("screenshot_url")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }
    if !step_screenshots.is_empty() {
        extra.insert("screenshots".into(), Value::Array(step_screenshots));
    }

    let final_url = extra
        .remove("final_url")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string());
    let title = extra
        .remove("title")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();

    let mut root = serde_json::Map::new();
    root.insert("action".into(), Value::String(action.to_string()));
    root.insert("mode".into(), Value::String("cdp".to_string()));
    root.insert("url".into(), Value::String(final_url));
    root.insert("title".into(), Value::String(title));
    root.insert(
        "screenshot_url".into(),
        screenshot_url.map(Value::String).unwrap_or(Value::Null),
    );
    root.insert(
        "screenshot_path".into(),
        screenshot_path.map(Value::String).unwrap_or(Value::Null),
    );
    root.insert(
        "console_errors".into(),
        Value::Array(session.console_errors.iter().cloned().map(Value::String).collect()),
    );
    root.insert(
        "failed_requests".into(),
        Value::Array(session.failed_requests.clone()),
    );
    root.insert("dom_summary".into(), Value::String(dom_summary));
    for (k, v) in extra {
        root.insert(k, v);
    }

    Ok(Value::Object(root))
}

#[derive(Debug, Clone)]
struct InteractionStep {
    interaction: String,
    selector: Option<String>,
    ref_id: Option<u64>,
    text: Option<String>,
    value: Option<String>,
    expression: Option<String>,
    wait_ms: u64,
    x: Option<f64>,
    y: Option<f64>,
    screenshot_name: Option<String>,
    full_page: Option<bool>,
    element_selector: Option<String>,
}

fn canonical_step_name(raw: &str) -> &str {
    match raw {
        "select" => "select_option",
        other => other,
    }
}

fn parse_ref_id_value(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64().filter(|&id| id >= 1),
        Value::String(s) => {
            let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
            trimmed.parse::<u64>().ok().filter(|&id| id >= 1)
        }
        _ => None,
    }
}

fn parse_interaction_steps(args: &Value, default_wait_ms: u64) -> Result<Vec<InteractionStep>, String> {
    if let Some(arr_val) = args.get("steps").or_else(|| args.get("actions")) {
        let arr = arr_val
            .as_array()
            .ok_or_else(|| "Parameter 'steps'/'actions' must be an array".to_string())?;
        if arr.is_empty() {
            return Err("Parameter 'steps' array must not be empty".to_string());
        }
        let mut steps = Vec::with_capacity(arr.len());
        for (i, item) in arr.iter().enumerate() {
            let raw_op = item
                .get("interaction")
                .or_else(|| item.get("action"))
                .or_else(|| item.get("type"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    format!("steps[{}] is missing 'interaction' (or 'action')", i)
                })?;
            let interaction = canonical_step_name(raw_op.trim()).to_string();
            let selector = item
                .get("selector")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let ref_id = parse_ref_id_value(item.get("ref_id").or_else(|| item.get("ref")));
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let value = item
                .get("value")
                .or_else(|| item.get("key"))
                .or_else(|| item.get("assert_text"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                });
            let expression = item
                .get("expression")
                .or_else(|| item.get("script"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let step_default_wait = if interaction == "wait_ms" {
                value
                    .as_deref()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(500)
            } else {
                default_wait_ms
            };
            let wait_ms = item
                .get("wait_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(step_default_wait)
                .clamp(0, 15000);
            let x = item.get("x").or_else(|| item.get("dx")).and_then(|v| v.as_f64());
            let y = item.get("y").or_else(|| item.get("dy")).and_then(|v| v.as_f64());
            let screenshot_name = item
                .get("screenshot_name")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let full_page = item.get("full_page").and_then(|v| v.as_bool());
            let element_selector = item
                .get("element_selector")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            let step = InteractionStep {
                interaction,
                selector,
                ref_id,
                text,
                value,
                expression,
                wait_ms,
                x,
                y,
                screenshot_name,
                full_page,
                element_selector,
            };
            validate_step(&step)?;
            steps.push(step);
        }
        return Ok(steps);
    }

    let raw_op = args
        .get("interaction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Action 'interact' requires 'interaction' (or 'steps') parameter".to_string())?;
    let interaction = canonical_step_name(raw_op.trim()).to_string();
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let ref_id = parse_ref_id_value(args.get("ref_id").or_else(|| args.get("ref")));
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let value = args
        .get("value")
        .or_else(|| args.get("assert_text"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let expression = args
        .get("expression")
        .or_else(|| args.get("script"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let step = InteractionStep {
        interaction,
        selector,
        ref_id,
        text,
        value,
        expression,
        wait_ms: default_wait_ms,
        x: args.get("x").and_then(|v| v.as_f64()),
        y: args.get("y").and_then(|v| v.as_f64()),
        screenshot_name: args
            .get("screenshot_name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        full_page: args.get("full_page").and_then(|v| v.as_bool()),
        element_selector: args
            .get("element_selector")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };
    validate_step(&step)?;
    Ok(vec![step])
}

fn has_target(step: &InteractionStep) -> bool {
    step.ref_id.is_some()
        || step.selector.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
        || step.text.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
}

fn has_coords(step: &InteractionStep) -> bool {
    step.x.is_some() && step.y.is_some()
}

fn validate_step(step: &InteractionStep) -> Result<(), String> {
    match step.interaction.as_str() {
        "click" | "dblclick" | "hover" => {
            if !has_target(step) && !has_coords(step) {
                return Err(format!(
                    "Interaction '{}' requires 'selector', 'ref_id', 'text', or viewport ('x' and 'y')",
                    step.interaction
                ));
            }
        }
        "wait_for_selector" | "assert_selector" => {
            if !has_target(step) {
                return Err(format!(
                    "Interaction '{}' requires 'selector', 'ref_id', or 'text'",
                    step.interaction
                ));
            }
        }
        "fill" | "type" | "select_option" => {
            if !has_target(step) {
                return Err(format!(
                    "Interaction '{}' requires 'selector', 'ref_id', or 'text'",
                    step.interaction
                ));
            }
            if step.value.is_none() {
                return Err(format!("Interaction '{}' requires 'value'", step.interaction));
            }
        }
        "press_key" => {
            if step.value.as_deref().unwrap_or("").is_empty() {
                return Err("Interaction 'press_key' requires 'value' (or 'key')".to_string());
            }
        }
        "eval" => {
            let expr = step
                .expression
                .as_deref()
                .or(step.value.as_deref())
                .unwrap_or("");
            if expr.trim().is_empty() {
                return Err("Interaction 'eval' requires 'expression' or 'value'".to_string());
            }
        }
        "assert_text" => {
            let expected = step.value.as_deref().or(step.text.as_deref()).unwrap_or("");
            if expected.is_empty() {
                return Err("Interaction 'assert_text' requires 'value'".to_string());
            }
        }
        "wait_ms" | "scroll_by" | "scroll_to" | "screenshot" => {}
        other => {
            return Err(format!(
                "Unknown interaction '{}'. Expected one of: click, dblclick, fill, type, press_key, select_option, select, hover, wait_for_selector, wait_ms, scroll_by, scroll_to, eval, screenshot, assert_text, assert_selector",
                other
            ));
        }
    }
    Ok(())
}

// ============================================================================
// Pure-Rust CDP Browser Session + WebSocket Client
// ============================================================================

const INTERACTIVE_QUERY_SELECTOR: &str = "a[href], button, input:not([type=\"hidden\"]), select, textarea, summary, [role=\"button\"], [role=\"link\"], [role=\"checkbox\"], [role=\"radio\"], [role=\"switch\"], [role=\"tab\"], [role=\"menuitem\"], [role=\"option\"], [role=\"combobox\"], [role=\"textbox\"], [role=\"searchbox\"], [role=\"slider\"], [contenteditable=\"true\"], [contenteditable=\"\"], [tabindex]:not([tabindex=\"-1\"])";

/// Shared JS snippet providing `collectInteractive(maxCount)`, `buildCssSelector(el)`, and `resolveTarget(refId, selector, text)`.
fn js_target_helpers() -> String {
    format!(
        r#"
  const trim = (s, n) => (s || '').replace(/\s+/g, ' ').trim().slice(0, n);
  const INTERACTIVE_SEL = {interactive_sel};
  const isElementVisible = (el) => {{
    if (!el || !el.isConnected) return false;
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden') return false;
    if (el.closest && el.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  }};
  const cssEscapeIdent = (s) => {{
    if (typeof CSS !== 'undefined' && typeof CSS.escape === 'function') return CSS.escape(s);
    return String(s).replace(/([^\w-])/g, '\\$1');
  }};
  const buildCssSelector = (el) => {{
    if (!el || el.nodeType !== 1) return '';
    if (el.id) {{
      const idSel = '#' + cssEscapeIdent(el.id);
      try {{ if (document.querySelectorAll(idSel).length === 1) return idSel; }} catch (_) {{}}
    }}
    const tag = el.tagName.toLowerCase();
    for (const attr of ['data-testid', 'data-test', 'name', 'aria-label']) {{
      const v = el.getAttribute(attr);
      if (v) {{
        const cand = tag + '[' + attr + '=' + JSON.stringify(v) + ']';
        try {{ if (document.querySelectorAll(cand).length === 1) return cand; }} catch (_) {{}}
      }}
    }}
    const parts = [];
    let cur = el;
    for (let depth = 0; cur && cur.nodeType === 1 && depth < 5; depth++) {{
      if (cur.id) {{
        const idSel = '#' + cssEscapeIdent(cur.id);
        try {{
          if (document.querySelectorAll(idSel).length === 1) {{
            parts.unshift(idSel);
            break;
          }}
        }} catch (_) {{}}
      }}
      const t = cur.tagName.toLowerCase();
      let nth = 1;
      let sib = cur.previousElementSibling;
      while (sib) {{
        if (sib.tagName === cur.tagName) nth++;
        sib = sib.previousElementSibling;
      }}
      const sameTagSibs = cur.parentElement
        ? Array.from(cur.parentElement.children).filter(c => c.tagName === cur.tagName).length
        : 1;
      parts.unshift(sameTagSibs > 1 ? (t + ':nth-of-type(' + nth + ')') : t);
      if (t === 'body' || t === 'html') break;
      cur = cur.parentElement;
    }}
    return parts.join(' > ');
  }};
  const collectInteractive = (maxCount) => {{
    const raw = Array.from(document.querySelectorAll(INTERACTIVE_SEL));
    const out = [];
    for (const el of raw) {{
      if (!isElementVisible(el)) continue;
      out.push(el);
      if (out.length >= maxCount) break;
    }}
    return out;
  }};
  const elementLabelText = (el) => {{
    const aria = el.getAttribute('aria-label');
    if (aria) return trim(aria, 120);
    const labelledBy = el.getAttribute('aria-labelledby');
    if (labelledBy) {{
      const parts = labelledBy.split(/\s+/).map(id => document.getElementById(id)).filter(Boolean).map(n => n.innerText || n.textContent || '');
      const joined = trim(parts.join(' '), 120);
      if (joined) return joined;
    }}
    if (el.id) {{
      try {{
        const lbl = document.querySelector('label[for=' + JSON.stringify(el.id) + ']');
        if (lbl) {{
          const t = trim(lbl.innerText || lbl.textContent, 120);
          if (t) return t;
        }}
      }} catch (_) {{}}
    }}
    if (el.closest) {{
      const wrapLbl = el.closest('label');
      if (wrapLbl) {{
        const t = trim(wrapLbl.innerText || wrapLbl.textContent, 120);
        if (t) return t;
      }}
    }}
    return trim(el.innerText || el.value || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || '', 120);
  }};
  const findByVisibleText = (needleRaw) => {{
    const needle = trim(needleRaw, 200).toLowerCase();
    if (!needle) return null;
    const interactive = collectInteractive(300);
    for (const el of interactive) {{
      const cand = trim(el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('title') || '', 200).toLowerCase();
      if (cand === needle) return el;
    }}
    for (const el of interactive) {{
      const cand = trim(el.innerText || el.value || el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('title') || '', 200).toLowerCase();
      if (cand.includes(needle)) return el;
    }}
    const all = Array.from(document.querySelectorAll('a, button, label, h1, h2, h3, h4, h5, h6, p, span, div, li, td, th, summary'));
    let bestSub = null;
    for (const el of all) {{
      if (!isElementVisible(el)) continue;
      const t = trim(el.innerText || el.textContent, 200).toLowerCase();
      if (t === needle) return el;
      if (!bestSub && t.includes(needle) && el.children.length <= 2) bestSub = el;
    }}
    return bestSub;
  }};
  const resolveTarget = (refId, selector, text) => {{
    if (typeof refId === 'number' && refId >= 1) {{
      const list = collectInteractive(Math.max(120, refId));
      const el = list[refId - 1] || null;
      if (!el) return {{ el: null, error: 'Snapshot ref_id [' + refId + '] not found (page has ' + list.length + ' interactive elements)' }};
      return {{ el }};
    }}
    if (selector && selector.length > 0) {{
      if (selector.startsWith('text=')) {{
        const el = findByVisibleText(selector.slice(5));
        return el ? {{ el }} : {{ el: null, error: 'No visible element found matching text "' + selector.slice(5) + '"' }};
      }}
      const refMatch = /^\[(\d+)\]$/.exec(selector.trim());
      if (refMatch) {{
        const idx = parseInt(refMatch[1], 10);
        const list = collectInteractive(Math.max(120, idx));
        const el = list[idx - 1] || null;
        if (!el) return {{ el: null, error: 'Snapshot ref_id [' + idx + '] not found (page has ' + list.length + ' interactive elements)' }};
        return {{ el }};
      }}
      try {{
        const el = document.querySelector(selector);
        if (el) return {{ el }};
      }} catch (_) {{}}
      if (!text && !/[#.\[\]>:+~=]/.test(selector)) {{
        const fb = findByVisibleText(selector);
        if (fb) return {{ el: fb }};
      }}
      return {{ el: null, error: 'Selector "' + selector + '" matched no element' }};
    }}
    if (text && text.length > 0) {{
      const el = findByVisibleText(text);
      return el ? {{ el }} : {{ el: null, error: 'No visible element found matching text "' + text + '"' }};
    }}
    return {{ el: null, error: 'No selector, ref_id, or text provided' }};
  }};
"#,
        interactive_sel = serde_json::to_string(INTERACTIVE_QUERY_SELECTOR).unwrap_or_default()
    )
}

struct TempUserDataDir {
    path: PathBuf,
    cleaned: bool,
}

impl TempUserDataDir {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            cleaned: false,
        }
    }
}

impl Drop for TempUserDataDir {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        // Recursively removing a Chromium profile directory blocks on file
        // syscalls; run it on a blocking thread instead of the async executor.
        let path = self.path.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let _ = handle.spawn_blocking(move || {
                let _ = std::fs::remove_dir_all(&path);
            });
        } else {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[derive(Debug, Clone)]
struct PendingRequestInfo {
    url: String,
    method: String,
    resource_type: String,
}

pub(crate) struct CdpBrowserSession {
    child: Child,
    _user_data_dir: TempUserDataDir,
    ws: WsClient,
    next_id: u64,
    console_errors: Vec<String>,
    console_warnings: Vec<String>,
    uncaught_exceptions: Vec<Value>,
    failed_requests: Vec<Value>,
    pending_requests: std::collections::HashMap<String, PendingRequestInfo>,
    page_loaded: bool,
    closed: bool,
}

/// Force-kills a browser process (and its process tree on Windows).
///
/// Blocking: `.status()` waits for the kill helper to exit, so callers must run
/// this on a blocking thread rather than an async executor thread.
fn kill_browser_process(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .status();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{}", pid)])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

impl Drop for CdpBrowserSession {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        // taskkill/kill wait on the OS; never run them on the async executor.
        if let Some(pid) = self.child.id() {
            let perform = move || kill_browser_process(pid);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let _ = handle.spawn_blocking(perform);
            } else {
                perform();
            }
        }
        let _ = self.child.start_kill();
    }
}

fn has_desktop_display() -> bool {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        true
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
    }
}

impl CdpBrowserSession {
    async fn launch(browser_path: &Path, width: u32, height: u32) -> Result<Self, String> {
        Self::launch_with_options(browser_path, width, height, false, true).await
    }

    pub(crate) async fn launch_incognito(
        browser_path: &Path,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        // Real desktop incognito (positioned off-screen) avoids HeadlessChrome UA gating
        // on services such as Google AI Mode, falling back to --headless=new when headless.
        Self::launch_with_options(browser_path, width, height, true, !has_desktop_display()).await
    }

    async fn launch_with_options(
        browser_path: &Path,
        width: u32,
        height: u32,
        incognito: bool,
        headless: bool,
    ) -> Result<Self, String> {
        let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "harness_cdp_{}_{}_{}",
            std::process::id(),
            Utc::now().timestamp_millis(),
            nonce
        ));
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .map_err(|e| format!("Failed to create browser temp dir: {}", e))?;
        let user_data_guard = TempUserDataDir::new(temp_dir.clone());

        let mut cmd = Command::new(browser_path);
        if headless {
            cmd.arg("--headless=new");
        } else {
            cmd.arg("--window-position=-32000,-32000");
        }
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", temp_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-gpu")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-default-apps")
            .arg("--mute-audio")
            .arg("--disable-features=Translate,OptimizationHints,MediaRouter,DialMediaRouteProvider")
            .arg(format!("--window-size={},{}", width, height));

        if incognito {
            let is_edge = browser_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_ascii_lowercase().contains("msedge"))
                .unwrap_or(false);
            cmd.arg(if is_edge { "--inprivate" } else { "--incognito" });
        }

        #[cfg(target_os = "linux")]
        cmd.arg("--disable-dev-shm-usage");

        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

        #[cfg(unix)]
        cmd.process_group(0);

        cmd.arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn headless browser '{}': {}",
                browser_path.display(),
                e
            )
        })?;

        let stderr = child.stderr.take();
        let port = match discover_debug_port(stderr, &temp_dir).await {
            Ok(p) => p,
            Err(e) => {
                cleanup_child(&mut child).await;
                return Err(e);
            }
        };

        let ws_url = match discover_page_ws_url(port).await {
            Ok(u) => u,
            Err(e) => {
                cleanup_child(&mut child).await;
                return Err(e);
            }
        };

        let ws = match WsClient::connect(&ws_url).await {
            Ok(w) => w,
            Err(e) => {
                cleanup_child(&mut child).await;
                return Err(e);
            }
        };

        Ok(Self {
            child,
            _user_data_dir: user_data_guard,
            ws,
            next_id: 1,
            console_errors: Vec::new(),
            console_warnings: Vec::new(),
            uncaught_exceptions: Vec::new(),
            failed_requests: Vec::new(),
            pending_requests: std::collections::HashMap::new(),
            page_loaded: false,
            closed: false,
        })
    }

    pub(crate) fn failed_requests(&self) -> &[Value] {
        &self.failed_requests
    }

    pub(crate) async fn close(mut self) {
        self.closed = true;
        let _ = timeout(Duration::from_millis(350), self.call("Browser.close", json!({}))).await;
        cleanup_child(&mut self.child).await;
        // TempUserDataDir drops after child exit, removing temporary profile files
    }

    pub(crate) async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let req = json!({
            "id": id,
            "method": method,
            "params": params
        });
        self.ws.send_text(&req.to_string()).await?;

        loop {
            let msg_text = self
                .ws
                .recv_text()
                .await?
                .ok_or_else(|| format!("CDP WebSocket closed while waiting for '{}'", method))?;
            let val: Value = serde_json::from_str(&msg_text)
                .map_err(|e| format!("Invalid CDP JSON message: {}", e))?;

            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = val.get("error") {
                    return Err(format!("CDP '{}' failed: {}", method, err));
                }
                return Ok(val.get("result").cloned().unwrap_or(Value::Null));
            } else {
                self.handle_event(&val);
            }
        }
    }

    fn handle_event(&mut self, msg: &Value) {
        let Some(method) = msg.get("method").and_then(|v| v.as_str()) else {
            return;
        };
        let params = msg.get("params").unwrap_or(&Value::Null);

        match method {
            "Page.loadEventFired" => {
                self.page_loaded = true;
            }
            "Runtime.consoleAPICalled" => {
                let kind = params.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(kind, "error" | "assert" | "warning") {
                    let parts: Vec<String> = params
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|a| {
                                    a.get("value")
                                        .map(|v| match v {
                                            Value::String(s) => s.clone(),
                                            other => other.to_string(),
                                        })
                                        .or_else(|| {
                                            a.get("description")
                                                .and_then(|d| d.as_str())
                                                .map(str::to_string)
                                        })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let text = if parts.is_empty() {
                        format!("console.{}", kind)
                    } else {
                        parts.join(" ")
                    };
                    if kind == "warning" {
                        self.push_console_warning(text);
                    } else {
                        self.push_console_error(text);
                    }
                }
            }
            "Runtime.exceptionThrown" => {
                let details = params.get("exceptionDetails").unwrap_or(&Value::Null);
                let text = details
                    .get("exception")
                    .and_then(|e| e.get("description"))
                    .and_then(|d| d.as_str())
                    .or_else(|| details.get("text").and_then(|t| t.as_str()))
                    .unwrap_or("Uncaught JS exception");
                self.push_console_error(text.to_string());
                if self.uncaught_exceptions.len() < MAX_CONSOLE_ERRORS {
                    self.uncaught_exceptions.push(json!({
                        "message": truncate_chars(text, 400),
                        "url": details.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                        "line": details.get("lineNumber").and_then(|v| v.as_u64()),
                        "column": details.get("columnNumber").and_then(|v| v.as_u64()),
                    }));
                }
            }
            "Log.entryAdded" => {
                let entry = params.get("entry").unwrap_or(&Value::Null);
                let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(text) = entry.get("text").and_then(|v| v.as_str()) {
                    if level == "error" {
                        self.push_console_error(text.to_string());
                    } else if level == "warning" {
                        self.push_console_warning(text.to_string());
                    }
                }
            }
            "Network.requestWillBeSent" => {
                if let Some(req_id) = params.get("requestId").and_then(|v| v.as_str()) {
                    if self.pending_requests.len() < 2000 {
                        let req = params.get("request").unwrap_or(&Value::Null);
                        self.pending_requests.insert(
                            req_id.to_string(),
                            PendingRequestInfo {
                                url: req
                                    .get("url")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                method: req
                                    .get("method")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("GET")
                                    .to_string(),
                                resource_type: params
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            },
                        );
                    }
                }
            }
            "Network.responseReceived" => {
                let req_id = params.get("requestId").and_then(|v| v.as_str()).unwrap_or("");
                let resp = params.get("response").unwrap_or(&Value::Null);
                let status = resp.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
                let req_info = self.pending_requests.remove(req_id);
                if status >= 400 && self.failed_requests.len() < 100 {
                    let resp_url = resp
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or_else(|| req_info.as_ref().map(|r| r.url.clone()))
                        .unwrap_or_default();
                    self.failed_requests.push(json!({
                        "url": truncate_chars(&resp_url, 300),
                        "method": req_info.as_ref().map(|r| r.method.as_str()).unwrap_or("GET"),
                        "resource_type": params.get("type").and_then(|v| v.as_str()).or_else(|| req_info.as_ref().map(|r| r.resource_type.as_str())).unwrap_or(""),
                        "status": status,
                        "status_text": resp.get("statusText").and_then(|v| v.as_str()).unwrap_or(""),
                        "mime_type": resp.get("mimeType").and_then(|v| v.as_str()).unwrap_or("")
                    }));
                }
            }
            "Network.loadingFailed" => {
                let req_id = params.get("requestId").and_then(|v| v.as_str()).unwrap_or("");
                let req_info = self.pending_requests.remove(req_id);
                let canceled = params.get("canceled").and_then(|v| v.as_bool()).unwrap_or(false);
                if !canceled && self.failed_requests.len() < 100 {
                    self.failed_requests.push(json!({
                        "url": truncate_chars(req_info.as_ref().map(|r| r.url.as_str()).unwrap_or(""), 300),
                        "method": req_info.as_ref().map(|r| r.method.as_str()).unwrap_or("GET"),
                        "resource_type": params.get("type").and_then(|v| v.as_str()).or_else(|| req_info.as_ref().map(|r| r.resource_type.as_str())).unwrap_or(""),
                        "status": Value::Null,
                        "error_text": params.get("errorText").and_then(|v| v.as_str()).unwrap_or("Loading failed"),
                        "blocked_reason": params.get("blockedReason").and_then(|v| v.as_str())
                    }));
                }
            }
            _ => {}
        }
    }

    fn push_console_error(&mut self, msg: String) {
        if self.console_errors.len() < MAX_CONSOLE_ERRORS {
            let trimmed = truncate_chars(&msg, 400);
            if !self.console_errors.contains(&trimmed) {
                self.console_errors.push(trimmed);
            }
        }
    }

    fn push_console_warning(&mut self, msg: String) {
        if self.console_warnings.len() < MAX_CONSOLE_ERRORS {
            let trimmed = truncate_chars(&msg, 400);
            if !self.console_warnings.contains(&trimmed) {
                self.console_warnings.push(trimmed);
            }
        }
    }

    pub(crate) async fn pump_events(&mut self, duration_ms: u64) {
        if duration_ms == 0 {
            return;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(duration_ms);
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline - now;
            match timeout(remaining, self.ws.recv_text()).await {
                Ok(Ok(Some(text))) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        self.handle_event(&val);
                    }
                }
                _ => break,
            }
        }
    }

    pub(crate) async fn enable_default_domains(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.enable_domains(&ViewportConfig {
            width,
            height,
            preset: None,
            is_mobile: false,
        })
        .await
    }

    async fn enable_domains(&mut self, viewport: &ViewportConfig) -> Result<(), String> {
        self.call("Page.enable", json!({})).await?;
        self.call("Runtime.enable", json!({})).await?;
        let _ = self.call("Log.enable", json!({})).await;
        let _ = self.call("Network.enable", json!({})).await;
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": viewport.width,
                "height": viewport.height,
                "deviceScaleFactor": 1,
                "mobile": viewport.is_mobile
            }),
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn navigate(&mut self, url: &str, wait_ms: u64) -> Result<(), String> {
        self.page_loaded = false;
        let nav = self.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(err_text) = nav.get("errorText").and_then(|v| v.as_str()) {
            if !err_text.is_empty() {
                return Err(format!("Navigation to '{}' failed: {}", url, err_text));
            }
        }

        // Wait up to 8s for Page.loadEventFired (checking readyState as backup)
        let load_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while !self.page_loaded && tokio::time::Instant::now() < load_deadline {
            self.pump_events(120).await;
            if !self.page_loaded {
                if let Ok(ready) = self.eval_js("document.readyState").await {
                    if ready.as_str() == Some("complete") {
                        self.page_loaded = true;
                        break;
                    }
                }
            }
        }

        if wait_ms > 0 {
            self.pump_events(wait_ms).await;
        }
        Ok(())
    }

    pub(crate) async fn eval_js(&mut self, expression: &str) -> Result<Value, String> {
        let res = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await?;

        if let Some(exc) = res.get("exceptionDetails") {
            let desc = exc
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|d| d.as_str())
                .or_else(|| exc.get("text").and_then(|t| t.as_str()))
                .unwrap_or("JS evaluation threw an exception");
            return Err(format!("JS exception: {}", desc));
        }

        let result_obj = res.get("result").unwrap_or(&Value::Null);
        if let Some(val) = result_obj.get("value") {
            return Ok(val.clone());
        }
        if let Some(desc) = result_obj.get("description") {
            return Ok(desc.clone());
        }
        Ok(result_obj.clone())
    }

    async fn collect_basic_meta(&mut self) -> Result<Value, String> {
        self.eval_js("({ title: document.title || '', url: location.href })")
            .await
    }

    async fn collect_page_inventory(&mut self) -> Result<Value, String> {
        let script = format!(
            r#"(() => {{
  const trim = (s, n) => (s || '').replace(/\s+/g, ' ').trim().slice(0, n);
  const visible = (el) => {{
    const st = getComputedStyle(el);
    if (st.display === 'none' || st.visibility === 'hidden' || st.opacity === '0') return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  }};
  const allLinks = Array.from(document.querySelectorAll('a[href]'));
  const allButtons = Array.from(document.querySelectorAll('button, input[type="button"], input[type="submit"], [role="button"]'));
  const allInputs = Array.from(document.querySelectorAll('input, textarea, select'));
  return {{
    title: document.title || '',
    url: location.href,
    readyState: document.readyState,
    visibleText: trim(document.body ? document.body.innerText : '', {}),
    counts: {{
      links: allLinks.length,
      buttons: allButtons.length,
      inputs: allInputs.length,
      forms: document.forms.length,
      images: document.images.length
    }},
    links: allLinks.slice(0, 30).map(a => ({{
      text: trim(a.innerText || a.getAttribute('aria-label'), 80),
      href: a.getAttribute('href') || '',
      visible: visible(a)
    }})),
    buttons: allButtons.slice(0, 25).map(b => ({{
      text: trim(b.innerText || b.value || b.getAttribute('aria-label'), 80),
      id: b.id || '',
      type: b.getAttribute('type') || '',
      disabled: !!b.disabled,
      visible: visible(b)
    }})),
    inputs: allInputs.slice(0, 25).map(i => ({{
      tag: i.tagName.toLowerCase(),
      type: i.getAttribute('type') || '',
      name: i.getAttribute('name') || '',
      id: i.id || '',
      placeholder: i.getAttribute('placeholder') || '',
      value: trim(i.value, 60),
      visible: visible(i)
    }}))
  }};
}})()"#,
            MAX_TEXT_PREVIEW_CHARS
        );
        self.eval_js(&script).await
    }

    async fn collect_snapshot(&mut self, max_elements: usize) -> Result<Value, String> {
        let helpers = js_target_helpers();
        let script = format!(
            r#"(() => {{
{helpers}
  const implicitRole = (el) => {{
    const tag = el.tagName.toLowerCase();
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'button' || tag === 'summary') return 'button';
    if (tag === 'select') return 'combobox';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'input') {{
      const t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (t === 'submit' || t === 'button' || t === 'reset') return 'button';
      if (t === 'range') return 'slider';
      if (t === 'search') return 'searchbox';
      return 'textbox';
    }}
    if (el.isContentEditable) return 'textbox';
    return tag;
  }};
  const allVis = collectInteractive(1000);
  const subset = allVis.slice(0, {max_elements});
  const elements = subset.map((el, idx) => {{
    const r = el.getBoundingClientRect();
    const st = getComputedStyle(el);
    const vis = isElementVisible(el) && parseFloat(st.opacity || '1') > 0;
    const enabled = !el.disabled && el.getAttribute('aria-disabled') !== 'true';
    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute('role') || implicitRole(el);
    const label = elementLabelText(el);
    const text = trim(el.innerText || el.value || label, 120);
    let value = null;
    if (tag === 'input') {{
      const t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'checkbox' || t === 'radio') value = el.checked ? 'checked' : 'unchecked';
      else value = trim(String(el.value || ''), 80);
    }} else if (tag === 'textarea' || tag === 'select') {{
      value = trim(String(el.value || ''), 80);
    }} else if (el.isContentEditable) {{
      value = trim(el.innerText || '', 80);
    }}
    return {{
      ref_id: idx + 1,
      tag,
      role,
      text,
      label,
      selector: buildCssSelector(el),
      bbox: {{
        x: Math.round(r.x),
        y: Math.round(r.y),
        w: Math.round(r.width),
        h: Math.round(r.height)
      }},
      visible: vis,
      enabled,
      value
    }};
  }});
  const docEl = document.documentElement;
  const body = document.body;
  const scrollWidth = Math.max(docEl ? docEl.scrollWidth : 0, body ? body.scrollWidth : 0);
  const scrollHeight = Math.max(docEl ? docEl.scrollHeight : 0, body ? body.scrollHeight : 0);
  return {{
    page_title: document.title || '',
    url: location.href,
    overflow_x: scrollWidth > window.innerWidth,
    total_interactive_count: allVis.length,
    viewport: {{
      inner_width: window.innerWidth,
      inner_height: window.innerHeight,
      scroll_width: scrollWidth,
      scroll_height: scrollHeight,
      scroll_x: Math.round(window.scrollX),
      scroll_y: Math.round(window.scrollY)
    }},
    elements
  }};
}})()"#,
            helpers = helpers,
            max_elements = max_elements
        );
        self.eval_js(&script).await
    }

    async fn collect_debug_audit(&mut self) -> Result<Value, String> {
        let helpers = js_target_helpers();
        let script = format!(
            r#"(() => {{
{helpers}
  const innerW = window.innerWidth;
  const innerH = window.innerHeight;
  const docEl = document.documentElement;
  const body = document.body;
  const scrollW = Math.max(docEl ? docEl.scrollWidth : 0, body ? body.scrollWidth : 0);
  const scrollH = Math.max(docEl ? docEl.scrollHeight : 0, body ? body.scrollHeight : 0);
  const overflowX = scrollW > innerW;

  const overflowOffenders = [];
  if (overflowX) {{
    const nodes = Array.from(document.querySelectorAll('body *'));
    for (const el of nodes) {{
      if (!isElementVisible(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.right > innerW + 1 || r.left < -1) {{
        overflowOffenders.push({{
          tag: el.tagName.toLowerCase(),
          selector: buildCssSelector(el),
          left: Math.round(r.left),
          right: Math.round(r.right),
          inner_width: innerW,
          overflow_px: Math.round(Math.max(r.right - innerW, -r.left))
        }});
        if (overflowOffenders.length >= 15) break;
      }}
    }}
  }}

  const brokenImages = [];
  for (const img of Array.from(document.images)) {{
    if (img.complete && img.naturalWidth === 0) {{
      const r = img.getBoundingClientRect();
      brokenImages.push({{
        src: img.currentSrc || img.getAttribute('src') || '',
        alt: trim(img.getAttribute('alt') || '', 80),
        selector: buildCssSelector(img),
        bbox: {{ x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }}
      }});
      if (brokenImages.length >= 50) break;
    }}
  }}

  const ancestorHidden = (el) => {{
    let p = el;
    while (p && p.nodeType === 1) {{
      const st = getComputedStyle(p);
      if (st.display === 'none' || st.visibility === 'hidden' || p.hasAttribute('hidden')) return true;
      p = p.parentElement;
    }}
    return false;
  }};

  const controls = Array.from(document.querySelectorAll('button, a[href], input:not([type="hidden"]), select, textarea, [role="button"], [role="link"]'));
  const zeroSizeControls = [];
  const clippedControls = [];
  const visibleControls = [];

  for (const el of controls) {{
    if (ancestorHidden(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) {{
      if (zeroSizeControls.length < 30) {{
        zeroSizeControls.push({{
          tag: el.tagName.toLowerCase(),
          text: elementLabelText(el),
          selector: buildCssSelector(el),
          bbox: {{ x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }}
        }});
      }}
      continue;
    }}
    visibleControls.push({{ el, r }});

    let clipReason = null;
    const st = getComputedStyle(el);
    if ((st.overflowX === 'hidden' || st.overflowX === 'clip') && el.scrollWidth > el.clientWidth + 2) {{
      clipReason = 'text_horizontal_clip';
    }} else if ((st.overflowY === 'hidden' || st.overflowY === 'clip') && el.scrollHeight > el.clientHeight + 2) {{
      clipReason = 'text_vertical_clip';
    }} else if (r.right > innerW + 1 || r.left < -1) {{
      clipReason = 'viewport_horizontal_clip';
    }} else {{
      let anc = el.parentElement;
      while (anc && anc !== document.body && anc !== document.documentElement) {{
        const ast = getComputedStyle(anc);
        if (ast.overflowX === 'hidden' || ast.overflowX === 'clip' || ast.overflowY === 'hidden' || ast.overflowY === 'clip') {{
          const ar = anc.getBoundingClientRect();
          if (r.right > ar.right + 2 || r.left < ar.left - 2 || r.bottom > ar.bottom + 2 || r.top < ar.top - 2) {{
            clipReason = 'ancestor_overflow_clip';
            break;
          }}
        }}
        anc = anc.parentElement;
      }}
    }}
    if (clipReason && clippedControls.length < 30) {{
      clippedControls.push({{
        tag: el.tagName.toLowerCase(),
        text: elementLabelText(el),
        selector: buildCssSelector(el),
        reason: clipReason,
        bbox: {{ x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }}
      }});
    }}
  }}

  const overlappingControls = [];
  const capVis = visibleControls.slice(0, 150);
  for (let i = 0; i < capVis.length && overlappingControls.length < 25; i++) {{
    for (let j = i + 1; j < capVis.length && overlappingControls.length < 25; j++) {{
      const A = capVis[i];
      const B = capVis[j];
      if (A.el.contains(B.el) || B.el.contains(A.el)) continue;
      const ix1 = Math.max(A.r.left, B.r.left);
      const iy1 = Math.max(A.r.top, B.r.top);
      const ix2 = Math.min(A.r.right, B.r.right);
      const iy2 = Math.min(A.r.bottom, B.r.bottom);
      const ow = ix2 - ix1;
      const oh = iy2 - iy1;
      if (ow > 4 && oh > 4) {{
        overlappingControls.push({{
          a: {{ tag: A.el.tagName.toLowerCase(), text: elementLabelText(A.el), selector: buildCssSelector(A.el) }},
          b: {{ tag: B.el.tagName.toLowerCase(), text: elementLabelText(B.el), selector: buildCssSelector(B.el) }},
          overlap_w: Math.round(ow),
          overlap_h: Math.round(oh)
        }});
      }}
    }}
  }}

  const nav = performance.getEntriesByType('navigation')[0] || null;
  const paints = performance.getEntriesByType('paint') || [];
  const resources = performance.getEntriesByType('resource') || [];
  const fp = paints.find(p => p.name === 'first-paint');
  const fcp = paints.find(p => p.name === 'first-contentful-paint');
  const transferBytes = resources.reduce((acc, r) => acc + (r.transferSize || 0), 0);

  return {{
    title: document.title || '',
    url: location.href,
    broken_images: brokenImages,
    overflow: {{
      overflow_x: overflowX,
      inner_width: innerW,
      inner_height: innerH,
      scroll_width: scrollW,
      scroll_height: scrollH,
      overflow_px: Math.max(0, scrollW - innerW),
      offending_elements: overflowOffenders
    }},
    zero_size_controls: zeroSizeControls,
    clipped_controls: clippedControls,
    overlapping_controls: overlappingControls,
    performance: {{
      dom_content_loaded_ms: nav ? Math.round(nav.domContentLoadedEventEnd) : null,
      load_event_ms: nav ? Math.round(nav.loadEventEnd) : null,
      dom_interactive_ms: nav ? Math.round(nav.domInteractive) : null,
      ttfb_ms: nav ? Math.round(Math.max(0, nav.responseStart - nav.requestStart)) : null,
      first_paint_ms: fp ? Math.round(fp.startTime) : null,
      first_contentful_paint_ms: fcp ? Math.round(fcp.startTime) : null,
      resource_count: resources.length,
      total_transfer_bytes: transferBytes
    }}
  }};
}})()"#,
            helpers = helpers
        );
        self.eval_js(&script).await
    }

    async fn execute_storage_action(&mut self, url: &str, args: &Value) -> Result<Value, String> {
        let storage_op = args
            .get("storage_op")
            .and_then(|v| v.as_str())
            .unwrap_or("get");
        let default_type = if matches!(storage_op, "set" | "remove") {
            "local"
        } else {
            "all"
        };
        let storage_type = args
            .get("storage_type")
            .and_then(|v| v.as_str())
            .unwrap_or(default_type);
        let storage_key = args
            .get("storage_key")
            .or_else(|| args.get("key"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let storage_val = args.get("value").and_then(|v| v.as_str());

        let meta = self.collect_basic_meta().await.unwrap_or_else(|_| json!({}));
        let current_url = meta
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or(url)
            .to_string();
        let title = meta
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match storage_op {
            "get" => {}
            "set" => {
                let key = storage_key.ok_or_else(|| {
                    "Action 'storage' with storage_op='set' requires 'storage_key'".to_string()
                })?;
                let val = storage_val.ok_or_else(|| {
                    "Action 'storage' with storage_op='set' requires 'value'".to_string()
                })?;
                if matches!(storage_type, "local" | "all") {
                    let script = format!(
                        "(() => {{ localStorage.setItem({}, {}); return true; }})()",
                        serde_json::to_string(key).map_err(|e| e.to_string())?,
                        serde_json::to_string(val).map_err(|e| e.to_string())?,
                    );
                    self.eval_js(&script).await?;
                }
                if matches!(storage_type, "session" | "all") {
                    let script = format!(
                        "(() => {{ sessionStorage.setItem({}, {}); return true; }})()",
                        serde_json::to_string(key).map_err(|e| e.to_string())?,
                        serde_json::to_string(val).map_err(|e| e.to_string())?,
                    );
                    self.eval_js(&script).await?;
                }
                if matches!(storage_type, "cookie" | "all") {
                    let res = self
                        .call(
                            "Network.setCookie",
                            json!({
                                "name": key,
                                "value": val,
                                "url": current_url
                            }),
                        )
                        .await?;
                    if res.get("success").and_then(|v| v.as_bool()) == Some(false) {
                        return Err(format!("Failed to set cookie '{}' on '{}'", key, current_url));
                    }
                }
            }
            "remove" => {
                let key = storage_key.ok_or_else(|| {
                    "Action 'storage' with storage_op='remove' requires 'storage_key'".to_string()
                })?;
                if matches!(storage_type, "local" | "all") {
                    let script = format!(
                        "(() => {{ localStorage.removeItem({}); return true; }})()",
                        serde_json::to_string(key).map_err(|e| e.to_string())?,
                    );
                    self.eval_js(&script).await?;
                }
                if matches!(storage_type, "session" | "all") {
                    let script = format!(
                        "(() => {{ sessionStorage.removeItem({}); return true; }})()",
                        serde_json::to_string(key).map_err(|e| e.to_string())?,
                    );
                    self.eval_js(&script).await?;
                }
                if matches!(storage_type, "cookie" | "all") {
                    let _ = self
                        .call(
                            "Network.deleteCookies",
                            json!({
                                "name": key,
                                "url": current_url
                            }),
                        )
                        .await?;
                }
            }
            "clear" => {
                if matches!(storage_type, "local" | "all") {
                    let _ = self.eval_js("(() => { localStorage.clear(); return true; })()").await?;
                }
                if matches!(storage_type, "session" | "all") {
                    let _ = self.eval_js("(() => { sessionStorage.clear(); return true; })()").await?;
                }
                if matches!(storage_type, "cookie" | "all") {
                    let _ = self.call("Network.clearBrowserCookies", json!({})).await?;
                }
            }
            other => {
                return Err(format!(
                    "Unknown storage_op '{}'. Expected one of: get, set, remove, clear",
                    other
                ));
            }
        }

        let web_storage = self
            .eval_js(
                r#"(() => {
  const readStore = (s) => {
    const out = {};
    try {
      for (let i = 0; i < s.length; i++) {
        const k = s.key(i);
        if (k !== null) out[k] = s.getItem(k);
      }
    } catch (e) {
      return { __error: String(e) };
    }
    return out;
  };
  return {
    local_storage: readStore(window.localStorage),
    session_storage: readStore(window.sessionStorage)
  };
})()"#,
            )
            .await
            .unwrap_or_else(|_| json!({ "local_storage": {}, "session_storage": {} }));

        let cookie_params = if current_url.starts_with("http://") || current_url.starts_with("https://") {
            json!({ "urls": [current_url] })
        } else {
            json!({})
        };
        let cookies_res = self
            .call("Network.getCookies", cookie_params)
            .await
            .unwrap_or_else(|_| json!({ "cookies": [] }));
        let cookies = cookies_res
            .get("cookies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let local_storage = web_storage
            .get("local_storage")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let session_storage = web_storage
            .get("session_storage")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let local_count = local_storage.as_object().map(|m| m.len()).unwrap_or(0);
        let session_count = session_storage.as_object().map(|m| m.len()).unwrap_or(0);
        let cookie_count = cookies.len();

        Ok(json!({
            "url": current_url,
            "title": title,
            "storage_op": storage_op,
            "storage_type": storage_type,
            "local_storage": local_storage,
            "session_storage": session_storage,
            "cookies": cookies,
            "counts": {
                "local_storage": local_count,
                "session_storage": session_count,
                "cookies": cookie_count
            }
        }))
    }

    async fn inspect_dom(&mut self, selector: &str) -> Result<Value, String> {
        let sel_json = serde_json::to_string(selector)
            .map_err(|e| format!("Failed to encode selector: {}", e))?;
        let script = format!(
            r#"(() => {{
  const sel = {sel};
  const nodes = Array.from(document.querySelectorAll(sel));
  const trim = (s, n) => (s || '').replace(/\s+/g, ' ').trim().slice(0, n);
  return {{
    title: document.title || '',
    url: location.href,
    totalCount: nodes.length,
    elements: nodes.slice(0, 40).map((el, idx) => {{
      const r = el.getBoundingClientRect();
      const st = getComputedStyle(el);
      const vis = st.display !== 'none' && st.visibility !== 'hidden' && parseFloat(st.opacity || '1') > 0 && r.width > 0 && r.height > 0;
      return {{
        index: idx,
        tag: el.tagName.toLowerCase(),
        id: el.id || '',
        classes: typeof el.className === 'string' ? trim(el.className, 120) : '',
        text: trim(el.innerText || el.textContent, 160),
        href: el.getAttribute('href'),
        type: el.getAttribute('type'),
        name: el.getAttribute('name'),
        value: el.value !== undefined ? trim(String(el.value), 80) : null,
        disabled: !!el.disabled,
        visible: vis,
        rect: {{
          x: Math.round(r.x),
          y: Math.round(r.y),
          width: Math.round(r.width),
          height: Math.round(r.height)
        }},
        computed: {{
          display: st.display,
          visibility: st.visibility,
          opacity: st.opacity
        }}
      }};
    }})
  }};
}})()"#,
            sel = sel_json
        );
        self.eval_js(&script).await
    }

    async fn run_workflow_steps(
        &mut self,
        steps: &[InteractionStep],
        sandbox: &Path,
        default_full_page: bool,
        default_element_selector: Option<&str>,
    ) -> Result<(Vec<Value>, bool, Vec<Value>), String> {
        let mut step_results = Vec::with_capacity(steps.len());
        let mut all_assertions_passed = true;
        let mut step_screenshots = Vec::new();

        for (idx, step) in steps.iter().enumerate() {
            let res = self
                .perform_interaction(step, sandbox, default_full_page, default_element_selector)
                .await?;
            if res.get("assertion_passed").and_then(|v| v.as_bool()) == Some(false) {
                all_assertions_passed = false;
            }
            if step.interaction == "screenshot" {
                if let (Some(p), Some(u)) = (
                    res.get("screenshot_path").and_then(|v| v.as_str()),
                    res.get("screenshot_url").and_then(|v| v.as_str()),
                ) {
                    step_screenshots.push(json!({
                        "step": idx + 1,
                        "screenshot_path": p,
                        "screenshot_url": u
                    }));
                }
            }
            if step.interaction != "wait_ms" && step.wait_ms > 0 {
                self.pump_events(step.wait_ms).await;
            }
            step_results.push(json!({
                "step": idx + 1,
                "interaction": step.interaction,
                "selector": step.selector,
                "ref_id": step.ref_id,
                "text": step.text,
                "result": res
            }));
        }

        Ok((step_results, all_assertions_passed, step_screenshots))
    }

    async fn dispatch_mouse_click(&mut self, cx: f64, cy: f64, click_count: u8) -> Result<(), String> {
        self.call(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseMoved",
                "x": cx,
                "y": cy
            }),
        )
        .await?;
        for n in 1..=click_count {
            self.call(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mousePressed",
                    "x": cx,
                    "y": cy,
                    "button": "left",
                    "buttons": 1,
                    "clickCount": n
                }),
            )
            .await?;
            self.call(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseReleased",
                    "x": cx,
                    "y": cy,
                    "button": "left",
                    "buttons": 0,
                    "clickCount": n
                }),
            )
            .await?;
        }
        Ok(())
    }

    async fn dispatch_key_press(&mut self, key_name: &str) -> Result<(), String> {
        let (key, code, vk, text_opt): (&str, String, u32, Option<String>) = match key_name {
            "Enter" | "Return" => ("Enter", "Enter".into(), 13, Some("\r".into())),
            "Tab" => ("Tab", "Tab".into(), 9, None),
            "Escape" | "Esc" => ("Escape", "Escape".into(), 27, None),
            "Backspace" => ("Backspace", "Backspace".into(), 8, None),
            "Delete" => ("Delete", "Delete".into(), 46, None),
            "Space" | " " => (" ", "Space".into(), 32, Some(" ".into())),
            "ArrowUp" => ("ArrowUp", "ArrowUp".into(), 38, None),
            "ArrowDown" => ("ArrowDown", "ArrowDown".into(), 40, None),
            "ArrowLeft" => ("ArrowLeft", "ArrowLeft".into(), 37, None),
            "ArrowRight" => ("ArrowRight", "ArrowRight".into(), 39, None),
            "Home" => ("Home", "Home".into(), 36, None),
            "End" => ("End", "End".into(), 35, None),
            "PageUp" => ("PageUp", "PageUp".into(), 33, None),
            "PageDown" => ("PageDown", "PageDown".into(), 34, None),
            other => {
                let first = other.chars().next().unwrap_or(' ');
                let upper = first.to_ascii_uppercase();
                let code = if upper.is_ascii_alphabetic() {
                    format!("Key{}", upper)
                } else if upper.is_ascii_digit() {
                    format!("Digit{}", upper)
                } else {
                    other.to_string()
                };
                let text = if other.chars().count() == 1 {
                    Some(other.to_string())
                } else {
                    None
                };
                (other, code, upper as u32, text)
            }
        };

        let mut down_params = json!({
            "type": if text_opt.is_some() { "keyDown" } else { "rawKeyDown" },
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk
        });
        if let (Some(t), Some(obj)) = (text_opt, down_params.as_object_mut()) {
            obj.insert("text".into(), Value::String(t.clone()));
            obj.insert("unmodifiedText".into(), Value::String(t));
        }
        self.call("Input.dispatchKeyEvent", down_params).await?;
        self.call(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": vk,
                "nativeVirtualKeyCode": vk
            }),
        )
        .await?;
        Ok(())
    }

    async fn perform_interaction(
        &mut self,
        step: &InteractionStep,
        sandbox: &Path,
        default_full_page: bool,
        default_element_selector: Option<&str>,
    ) -> Result<Value, String> {
        let ref_json = match step.ref_id {
            Some(id) => Value::from(id),
            None => Value::Null,
        };
        let sel_json = serde_json::to_string(step.selector.as_deref().unwrap_or(""))
            .map_err(|e| format!("Failed to encode selector: {}", e))?;
        let text_json = serde_json::to_string(step.text.as_deref().unwrap_or(""))
            .map_err(|e| format!("Failed to encode text: {}", e))?;
        let val_json = serde_json::to_string(step.value.as_deref().unwrap_or(""))
            .map_err(|e| format!("Failed to encode value: {}", e))?;
        let helpers = js_target_helpers();
        let op = step.interaction.as_str();

        match op {
            "wait_ms" => {
                self.pump_events(step.wait_ms).await;
                return Ok(json!({ "ok": true, "waited_ms": step.wait_ms }));
            }
            "eval" => {
                let expr = step
                    .expression
                    .as_deref()
                    .or(step.value.as_deref())
                    .unwrap_or("");
                let eval_result = self.eval_js(expr).await?;
                return Ok(json!({ "ok": true, "eval_result": eval_result }));
            }
            "screenshot" => {
                let name_hint = step.screenshot_name.as_deref().or(step.value.as_deref());
                let (full_path, rel_path, url_path) =
                    resolve_screenshot_path(sandbox, name_hint).await?;
                let use_full = step.full_page.unwrap_or(default_full_page);
                let elem_sel = step
                    .element_selector
                    .as_deref()
                    .or(step.selector.as_deref())
                    .or(default_element_selector);
                let png_bytes = self.capture_screenshot_png(use_full, elem_sel).await?;
                tokio::fs::write(&full_path, &png_bytes)
                    .await
                    .map_err(|e| format!("Failed to write step screenshot '{}': {}", rel_path, e))?;
                return Ok(json!({
                    "ok": true,
                    "screenshot_path": rel_path,
                    "screenshot_url": url_path,
                    "bytes": png_bytes.len()
                }));
            }
            "press_key" => {
                if has_target(step) {
                    let focus_script = format!(
                        r#"(() => {{
{helpers}
  const res = resolveTarget({ref_id}, {sel}, {txt});
  if (!res.el) return {{ ok: false, error: res.error }};
  res.el.scrollIntoView({{ block: 'center', inline: 'center' }});
  if (typeof res.el.focus === 'function') res.el.focus();
  return {{ ok: true }};
}})()"#,
                        helpers = helpers,
                        ref_id = ref_json,
                        sel = sel_json,
                        txt = text_json
                    );
                    let focus_res = self.eval_js(&focus_script).await?;
                    if focus_res.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                        return Err(format!(
                            "Interaction 'press_key' failed to focus target: {}",
                            focus_res.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                        ));
                    }
                }
                let key = step.value.as_deref().unwrap_or("");
                self.dispatch_key_press(key).await?;
                return Ok(json!({ "ok": true, "key": key }));
            }
            "wait_for_selector" => {
                let wait_budget = if step.wait_ms > 0 { step.wait_ms } else { 5000 };
                let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_budget);
                let check_script = format!(
                    r#"(() => {{
{helpers}
  const res = resolveTarget({ref_id}, {sel}, {txt});
  if (!res.el) return {{ found: false }};
  return {{
    found: true,
    visible: isElementVisible(res.el),
    tag: res.el.tagName.toLowerCase(),
    selector: buildCssSelector(res.el)
  }};
}})()"#,
                    helpers = helpers,
                    ref_id = ref_json,
                    sel = sel_json,
                    txt = text_json
                );
                loop {
                    let check = self.eval_js(&check_script).await?;
                    if check.get("found").and_then(|v| v.as_bool()) == Some(true) {
                        return Ok(json!({
                            "ok": true,
                            "found": true,
                            "visible": check.get("visible").and_then(|v| v.as_bool()).unwrap_or(false),
                            "tag": check.get("tag"),
                            "selector": check.get("selector")
                        }));
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(format!(
                            "Timed out after {}ms waiting for target (selector={:?}, ref_id={:?}, text={:?})",
                            wait_budget, step.selector, step.ref_id, step.text
                        ));
                    }
                    self.pump_events(100).await;
                }
            }
            "scroll_by" => {
                let (dx, dy) = parse_scroll_offsets(step.x, step.y, step.value.as_deref(), 0.0, 500.0);
                let script = format!(
                    r#"(() => {{
{helpers}
  if ({has_target}) {{
    const res = resolveTarget({ref_id}, {sel}, {txt});
    if (!res.el) return {{ ok: false, error: res.error }};
    res.el.scrollBy({dx}, {dy});
    return {{ ok: true, scroll_left: Math.round(res.el.scrollLeft), scroll_top: Math.round(res.el.scrollTop) }};
  }}
  window.scrollBy({dx}, {dy});
  return {{ ok: true, scroll_x: Math.round(window.scrollX), scroll_y: Math.round(window.scrollY) }};
}})()"#,
                    helpers = helpers,
                    has_target = has_target(step),
                    ref_id = ref_json,
                    sel = sel_json,
                    txt = text_json,
                    dx = dx,
                    dy = dy
                );
                let res = self.eval_js(&script).await?;
                if res.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                    return Err(format!(
                        "Interaction 'scroll_by' failed: {}",
                        res.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                    ));
                }
                return Ok(res);
            }
            "scroll_to" => {
                if has_target(step) {
                    let script = format!(
                        r#"(() => {{
{helpers}
  const res = resolveTarget({ref_id}, {sel}, {txt});
  if (!res.el) return {{ ok: false, error: res.error }};
  res.el.scrollIntoView({{ block: 'center', inline: 'center' }});
  return {{ ok: true, scroll_x: Math.round(window.scrollX), scroll_y: Math.round(window.scrollY), selector: buildCssSelector(res.el) }};
}})()"#,
                        helpers = helpers,
                        ref_id = ref_json,
                        sel = sel_json,
                        txt = text_json
                    );
                    let res = self.eval_js(&script).await?;
                    if res.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                        return Err(format!(
                            "Interaction 'scroll_to' failed: {}",
                            res.get("error").and_then(|v| v.as_str()).unwrap_or("unknown")
                        ));
                    }
                    return Ok(res);
                }
                let is_bottom = step
                    .value
                    .as_deref()
                    .map(|v| v.eq_ignore_ascii_case("bottom"))
                    .unwrap_or(false);
                let (tx, ty) = parse_scroll_offsets(step.x, step.y, step.value.as_deref(), 0.0, 0.0);
                let script = if is_bottom {
                    "(() => { window.scrollTo(0, document.documentElement.scrollHeight); return { ok: true, scroll_x: Math.round(window.scrollX), scroll_y: Math.round(window.scrollY) }; })()".to_string()
                } else {
                    format!(
                        "(() => {{ window.scrollTo({tx}, {ty}); return {{ ok: true, scroll_x: Math.round(window.scrollX), scroll_y: Math.round(window.scrollY) }}; }})()",
                        tx = tx,
                        ty = ty
                    )
                };
                return self.eval_js(&script).await;
            }
            "click" | "dblclick" | "hover" => {
                if !has_target(step) {
                    let cx = step.x.unwrap_or(-1.0);
                    let cy = step.y.unwrap_or(-1.0);
                    if cx < 0.0 || cy < 0.0 {
                        return Err(format!(
                            "Interaction '{}' with viewport coordinates requires non-negative 'x' and 'y'",
                            op
                        ));
                    }
                    match op {
                        "hover" => {
                            self.call(
                                "Input.dispatchMouseEvent",
                                json!({ "type": "mouseMoved", "x": cx, "y": cy }),
                            )
                            .await?;
                        }
                        "click" => {
                            self.dispatch_mouse_click(cx, cy, 1).await?;
                        }
                        "dblclick" => {
                            self.dispatch_mouse_click(cx, cy, 2).await?;
                        }
                        _ => {}
                    }
                    self.pump_events(40).await;
                    return Ok(json!({
                        "ok": true,
                        "tag": "viewport",
                        "cx": cx,
                        "cy": cy
                    }));
                }

                let locate_script = format!(
                    r#"(() => {{
{helpers}
  const res = resolveTarget({ref_id}, {sel}, {txt});
  if (!res.el) return {{ ok: false, error: res.error }};
  const el = res.el;
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  if (typeof el.focus === 'function' && {do_focus}) el.focus();
  const r = el.getBoundingClientRect();
  return {{
    ok: true,
    tag: el.tagName.toLowerCase(),
    text: trim(el.innerText || el.value || elementLabelText(el), 120),
    selector: buildCssSelector(el),
    disabled: !!el.disabled,
    left: r.left,
    top: r.top,
    cx: r.left + r.width / 2,
    cy: r.top + r.height / 2,
    w: r.width,
    h: r.height
  }};
}})()"#,
                    helpers = helpers,
                    ref_id = ref_json,
                    sel = sel_json,
                    txt = text_json,
                    do_focus = op != "hover"
                );
                let mut loc = self.eval_js(&locate_script).await?;
                if loc.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                    return Err(format!(
                        "Interaction '{}' failed (selector={:?}, ref_id={:?}, text={:?}): {}",
                        op,
                        step.selector,
                        step.ref_id,
                        step.text,
                        loc.get("error").and_then(|v| v.as_str()).unwrap_or("Element not found")
                    ));
                }
                let left = loc.get("left").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let top = loc.get("top").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let w = loc.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let h = loc.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0);
                // When x/y are supplied alongside a target selector (e.g. clicking inside a <canvas>), treat them as element-relative offsets.
                let cx = match step.x {
                    Some(ox) => (left + ox).max(left).min(left + (w - 1.0).max(0.0)),
                    None => loc.get("cx").and_then(|v| v.as_f64()).unwrap_or(-1.0),
                };
                let cy = match step.y {
                    Some(oy) => (top + oy).max(top).min(top + (h - 1.0).max(0.0)),
                    None => loc.get("cy").and_then(|v| v.as_f64()).unwrap_or(-1.0),
                };
                if let Some(obj) = loc.as_object_mut() {
                    obj.insert("cx".to_string(), json!(cx));
                    obj.insert("cy".to_string(), json!(cy));
                }

                if w > 0.0 && h > 0.0 && cx >= 0.0 && cy >= 0.0 {
                    match op {
                        "hover" => {
                            self.call(
                                "Input.dispatchMouseEvent",
                                json!({ "type": "mouseMoved", "x": cx, "y": cy }),
                            )
                            .await?;
                        }
                        "click" => {
                            self.dispatch_mouse_click(cx, cy, 1).await?;
                        }
                        "dblclick" => {
                            self.dispatch_mouse_click(cx, cy, 2).await?;
                        }
                        _ => {}
                    }
                    self.pump_events(40).await;
                } else {
                    // Fallback to DOM synthetic click/dblclick/mouseover for zero-rect targets
                    let dom_ev = match op {
                        "hover" => "res.el.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }));",
                        "dblclick" => "res.el.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));",
                        _ => "res.el.click();",
                    };
                    let fb_script = format!(
                        "(() => {{ {helpers} const res = resolveTarget({ref_id}, {sel}, {txt}); if (res.el) {{ {dom_ev} return true; }} return false; }})()",
                        helpers = helpers,
                        ref_id = ref_json,
                        sel = sel_json,
                        txt = text_json,
                        dom_ev = dom_ev
                    );
                    let _ = self.eval_js(&fb_script).await?;
                }
                return Ok(loc);
            }
            _ => {}
        }

        let script = match op {
            "fill" | "type" => {
                let emit_keys = op == "type";
                format!(
                    r#"(() => {{
{helpers}
  const res = resolveTarget({ref_id}, {sel}, {txt});
  if (!res.el) return {{ ok: false, error: res.error }};
  const el = res.el;
  const val = {val};
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  if (typeof el.focus === 'function') el.focus();
  if (el.isContentEditable) {{
    el.innerText = val;
  }} else {{
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
    if (setter) setter.call(el, val);
    else el.value = val;
  }}
  if ({emit_keys}) {{
    el.dispatchEvent(new KeyboardEvent('keydown', {{ bubbles: true, key: val.slice(-1) || 'a' }}));
  }}
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  if ({emit_keys}) {{
    el.dispatchEvent(new KeyboardEvent('keyup', {{ bubbles: true, key: val.slice(-1) || 'a' }}));
  }}
  return {{
    ok: true,
    tag: el.tagName.toLowerCase(),
    selector: buildCssSelector(el),
    value: String(el.isContentEditable ? el.innerText : (el.value || '')).slice(0, 120)
  }};
}})()"#,
                    helpers = helpers,
                    ref_id = ref_json,
                    sel = sel_json,
                    txt = text_json,
                    val = val_json,
                    emit_keys = emit_keys
                )
            }
            "select_option" => format!(
                r#"(() => {{
{helpers}
  const res = resolveTarget({ref_id}, {sel}, {txt});
  if (!res.el) return {{ ok: false, error: res.error }};
  const el = res.el;
  const val = {val};
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  let matched = false;
  if (el.options) {{
    for (const opt of Array.from(el.options)) {{
      if (opt.value === val || trim(opt.text, 200) === val) {{
        el.value = opt.value;
        opt.selected = true;
        matched = true;
        break;
      }}
    }}
  }}
  if (!matched) el.value = val;
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return {{ ok: true, tag: el.tagName.toLowerCase(), selector: buildCssSelector(el), value: String(el.value || '') }};
}})()"#,
                helpers = helpers,
                ref_id = ref_json,
                sel = sel_json,
                txt = text_json,
                val = val_json
            ),
            "assert_text" => format!(
                r#"(() => {{
{helpers}
  const expected = {expected};
  let target = document.body;
  if ({has_tgt}) {{
    const res = resolveTarget({ref_id}, {sel}, {txt});
    if (!res.el) return {{ ok: false, error: res.error }};
    target = res.el;
  }}
  if (!target) return {{ ok: false, error: 'Assertion target element not found' }};
  const actual = (target.innerText || target.textContent || '').replace(/\s+/g, ' ').trim();
  const passed = actual.includes(expected);
  return {{
    ok: true,
    assertion_passed: passed,
    expected,
    actual_excerpt: actual.slice(0, 400)
  }};
}})()"#,
                helpers = helpers,
                expected = serde_json::to_string(
                    step.value.as_deref().or(step.text.as_deref()).unwrap_or("")
                )
                .unwrap_or_else(|_| "\"\"".to_string()),
                has_tgt = has_target(step),
                ref_id = ref_json,
                sel = sel_json,
                txt = text_json
            ),
            "assert_selector" => format!(
                r#"(() => {{
{helpers}
  const selStr = {sel};
  let count = 0;
  let visibleCount = 0;
  if (selStr && selStr.length > 0 && !selStr.startsWith('text=') && !/^\[(\d+)\]$/.test(selStr)) {{
    try {{
      const nodes = Array.from(document.querySelectorAll(selStr));
      count = nodes.length;
      visibleCount = nodes.filter(isElementVisible).length;
    }} catch (_) {{}}
  }}
  if (count === 0) {{
    const res = resolveTarget({ref_id}, selStr, {txt});
    if (res.el) {{
      count = 1;
      visibleCount = isElementVisible(res.el) ? 1 : 0;
    }}
  }}
  const passed = count > 0;
  return {{
    ok: true,
    assertion_passed: passed,
    count,
    visible_count: visibleCount
  }};
}})()"#,
                helpers = helpers,
                ref_id = ref_json,
                sel = sel_json,
                txt = text_json
            ),
            _ => return Err(format!("Unsupported interaction '{}'", op)),
        };

        let result = self.eval_js(&script).await?;
        if result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let err = result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Interaction failed");
            return Err(format!(
                "Interaction '{}' failed (selector={:?}, ref_id={:?}, text={:?}): {}",
                op, step.selector, step.ref_id, step.text, err
            ));
        }
        Ok(result)
    }

    async fn capture_screenshot_png(
        &mut self,
        full_page: bool,
        element_selector: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let mut params = serde_json::Map::new();
        params.insert("format".into(), Value::String("png".into()));

        if let Some(sel) = element_selector.map(str::trim).filter(|s| !s.is_empty()) {
            let helpers = js_target_helpers();
            let sel_json = serde_json::to_string(sel)
                .map_err(|e| format!("Failed to encode element_selector: {}", e))?;
            let rect_script = format!(
                r#"(() => {{
{helpers}
  const res = resolveTarget(null, {sel}, null);
  if (!res.el) return {{ ok: false, error: res.error }};
  res.el.scrollIntoView({{ block: 'center', inline: 'center' }});
  const r = res.el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) {{
    return {{ ok: false, error: 'Element has zero bounding box (' + r.width + 'x' + r.height + ')' }};
  }}
  return {{
    ok: true,
    x: r.left + window.scrollX,
    y: r.top + window.scrollY,
    width: r.width,
    height: r.height
  }};
}})()"#,
                helpers = helpers,
                sel = sel_json
            );
            let rect = self.eval_js(&rect_script).await?;
            if rect.get("ok").and_then(|v| v.as_bool()) == Some(false) {
                return Err(format!(
                    "Element clipped screenshot failed for '{}': {}",
                    sel,
                    rect.get("error").and_then(|v| v.as_str()).unwrap_or("not found")
                ));
            }
            let x = rect.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
            let y = rect.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
            let w = rect.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let h = rect.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if w <= 0.0 || h <= 0.0 {
                return Err(format!(
                    "Element '{}' has non-positive bounding box ({}x{})",
                    sel, w, h
                ));
            }
            params.insert(
                "clip".into(),
                json!({ "x": x, "y": y, "width": w, "height": h, "scale": 1 }),
            );
            params.insert("captureBeyondViewport".into(), Value::Bool(true));
        } else if full_page {
            params.insert("captureBeyondViewport".into(), Value::Bool(true));
        }

        let res = self.call("Page.captureScreenshot", Value::Object(params)).await?;
        let b64 = res
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "CDP Page.captureScreenshot returned no base64 data".to_string())?;
        base64_decode(b64)
    }
}

fn parse_scroll_offsets(
    x: Option<f64>,
    y: Option<f64>,
    value: Option<&str>,
    default_x: f64,
    default_y: f64,
) -> (f64, f64) {
    if x.is_some() || y.is_some() {
        return (x.unwrap_or(default_x), y.unwrap_or(default_y));
    }
    if let Some(raw) = value.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((xs, ys)) = raw.split_once(',') {
            let px = xs.trim().parse::<f64>().unwrap_or(default_x);
            let py = ys.trim().parse::<f64>().unwrap_or(default_y);
            return (px, py);
        }
        if let Ok(py) = raw.parse::<f64>() {
            return (default_x, py);
        }
    }
    (default_x, default_y)
}

async fn cleanup_child(child: &mut Child) {
    if let Ok(Ok(_)) = timeout(Duration::from_millis(250), child.wait()).await {
        return;
    }
    #[cfg(target_os = "windows")]
    if let Some(pid) = child.id() {
        let mut tk = Command::new("taskkill");
        tk.args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        let _ = timeout(Duration::from_secs(2), tk.status()).await;
    }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let mut kg = Command::new("kill");
        kg.args(["-KILL", &format!("-{}", pid)])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = timeout(Duration::from_secs(2), kg.status()).await;
    }
    let _ = child.kill().await;
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
}

async fn discover_debug_port(
    stderr: Option<tokio::process::ChildStderr>,
    temp_dir: &Path,
) -> Result<u16, String> {
    let (tx, mut rx) = tokio::sync::oneshot::channel::<u16>();

    if let Some(err_stream) = stderr {
        tokio::spawn(async move {
            let mut reader = BufReader::new(err_stream).lines();
            let mut tx_opt = Some(tx);
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_opt.is_some() {
                    if let Some(port) = parse_devtools_ws_port(&line) {
                        if let Some(sender) = tx_opt.take() {
                            let _ = sender.send(port);
                        }
                    }
                }
            }
        });
    } else {
        drop(tx);
    }

    let active_port_file = temp_dir.join("DevToolsActivePort");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    loop {
        if let Ok(port) = rx.try_recv() {
            return Ok(port);
        }
        if let Ok(contents) = tokio::fs::read_to_string(&active_port_file).await {
            if let Some(first_line) = contents.lines().next() {
                if let Ok(port) = first_line.trim().parse::<u16>() {
                    if port > 0 {
                        return Ok(port);
                    }
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Timed out waiting for headless browser DevTools debugging port".to_string());
        }
        sleep(Duration::from_millis(50)).await;
    }
}

fn parse_devtools_ws_port(line: &str) -> Option<u16> {
    let idx = line.find("ws://")?;
    let rest = &line[idx + 5..];
    let after_host = if let Some(stripped) = rest.strip_prefix("[::1]:") {
        stripped
    } else {
        let colon = rest.find(':')?;
        &rest[colon + 1..]
    };
    let port_str: String = after_host.chars().take_while(|c| c.is_ascii_digit()).collect();
    port_str.parse::<u16>().ok().filter(|&p| p > 0)
}

async fn discover_page_ws_url(port: u16) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .map_err(|e| format!("Failed to build CDP discovery HTTP client: {}", e))?;

    let list_url = format!("http://127.0.0.1:{}/json/list", port);
    let new_url = format!("http://127.0.0.1:{}/json/new?about:blank", port);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut tried_new = false;

    loop {
        if let Ok(res) = client.get(&list_url).send().await {
            if let Ok(targets) = res.json::<Vec<Value>>().await {
                if let Some(ws) = targets.iter().find_map(|t| {
                    if t.get("type").and_then(|v| v.as_str()) == Some("page") {
                        t.get("webSocketDebuggerUrl")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    } else {
                        None
                    }
                }) {
                    return Ok(ws);
                }
            }
        }

        if !tried_new {
            tried_new = true;
            if let Ok(res) = client.put(&new_url).send().await {
                if let Ok(target) = res.json::<Value>().await {
                    if let Some(ws) = target.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        return Ok(ws.to_string());
                    }
                }
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!("No CDP page target found on port {}", port));
        }
        sleep(Duration::from_millis(60)).await;
    }
}

/// Minimal pure-Rust RFC 6455 WebSocket client over localhost `tokio::net::TcpStream`.
struct WsClient {
    stream: TcpStream,
    read_buf: Vec<u8>,
    mask_seed: u64,
}

impl WsClient {
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let stripped = ws_url
            .strip_prefix("ws://")
            .ok_or_else(|| format!("Unsupported WebSocket URL '{}'", ws_url))?;
        let (host_port, path) = match stripped.find('/') {
            Some(i) => (&stripped[..i], &stripped[i..]),
            None => (stripped, "/"),
        };
        let (host, port): (&str, u16) = if let Some(rest) = host_port.strip_prefix('[') {
            let (h, p) = rest
                .split_once("]:")
                .ok_or_else(|| format!("Invalid IPv6 host:port in '{}'", ws_url))?;
            let port = p
                .parse::<u16>()
                .map_err(|_| format!("Invalid port in '{}'", ws_url))?;
            (h, port)
        } else {
            let (h, p) = host_port
                .rsplit_once(':')
                .ok_or_else(|| format!("Invalid host:port in '{}'", ws_url))?;
            let port = p
                .parse::<u16>()
                .map_err(|_| format!("Invalid port in '{}'", ws_url))?;
            (h, port)
        };

        let mut stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| format!("Failed to connect CDP TCP socket {}:{}: {}", host, port, e))?;
        let _ = stream.set_nodelay(true);

        let key_seed = Utc::now().timestamp_nanos_opt().unwrap_or(123456789) as u64
            ^ NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut raw_key = [0u8; 16];
        raw_key[..8].copy_from_slice(&key_seed.to_le_bytes());
        raw_key[8..].copy_from_slice(&key_seed.wrapping_mul(0x9E3779B97F4A7C15).to_be_bytes());
        let sec_key = base64_encode(&raw_key);

        let req = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host_port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {sec_key}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| format!("Failed to write WebSocket upgrade: {}", e))?;

        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 2048];
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            if buf.len() > 16 * 1024 {
                return Err("WebSocket handshake response exceeded 16KB".to_string());
            }
            let n = stream
                .read(&mut tmp)
                .await
                .map_err(|e| format!("Failed reading WebSocket handshake: {}", e))?;
            if n == 0 {
                return Err("EOF during WebSocket handshake".to_string());
            }
            buf.extend_from_slice(&tmp[..n]);
        };

        let header_str = String::from_utf8_lossy(&buf[..header_end]);
        let status_line = header_str.lines().next().unwrap_or("");
        if !status_line.contains("101") {
            return Err(format!("WebSocket upgrade rejected: {}", status_line));
        }

        let leftover = buf.split_off(header_end);
        Ok(Self {
            stream,
            read_buf: leftover,
            mask_seed: key_seed | 1,
        })
    }

    async fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.send_frame(0x1, text.as_bytes()).await
    }

    async fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        self.mask_seed = self
            .mask_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mask = (self.mask_seed >> 32) as u32;
        let mask_bytes = mask.to_be_bytes();

        let len = payload.len();
        let mut frame = Vec::with_capacity(14 + len);
        frame.push(0x80 | (opcode & 0x0f));

        if len < 126 {
            frame.push(0x80 | (len as u8));
        } else if len <= 65535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }

        frame.extend_from_slice(&mask_bytes);
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask_bytes[i & 3]);
        }

        self.stream
            .write_all(&frame)
            .await
            .map_err(|e| format!("WebSocket write failed: {}", e))
    }

    async fn ensure_bytes(&mut self, needed: usize) -> Result<bool, String> {
        let mut tmp = [0u8; 8192];
        while self.read_buf.len() < needed {
            let n = self
                .stream
                .read(&mut tmp)
                .await
                .map_err(|e| format!("WebSocket read failed: {}", e))?;
            if n == 0 {
                return Ok(false);
            }
            self.read_buf.extend_from_slice(&tmp[..n]);
        }
        Ok(true)
    }

    async fn recv_text(&mut self) -> Result<Option<String>, String> {
        let mut message = Vec::new();
        let mut in_fragmented_text = false;

        loop {
            if !self.ensure_bytes(2).await? {
                return Ok(None);
            }
            let b0 = self.read_buf[0];
            let b1 = self.read_buf[1];
            let fin = (b0 & 0x80) != 0;
            let opcode = b0 & 0x0f;
            let masked = (b1 & 0x80) != 0;
            let len7 = (b1 & 0x7f) as usize;

            let mut offset = 2usize;
            let payload_len: usize = if len7 < 126 {
                len7
            } else if len7 == 126 {
                if !self.ensure_bytes(offset + 2).await? {
                    return Ok(None);
                }
                let v = u16::from_be_bytes([self.read_buf[offset], self.read_buf[offset + 1]]) as usize;
                offset += 2;
                v
            } else {
                if !self.ensure_bytes(offset + 8).await? {
                    return Ok(None);
                }
                let len_u64 = u64::from_be_bytes([
                    self.read_buf[offset],
                    self.read_buf[offset + 1],
                    self.read_buf[offset + 2],
                    self.read_buf[offset + 3],
                    self.read_buf[offset + 4],
                    self.read_buf[offset + 5],
                    self.read_buf[offset + 6],
                    self.read_buf[offset + 7],
                ]);
                if len_u64 > MAX_WS_FRAME_BYTES as u64 {
                    return Err(format!("WebSocket frame too large ({} bytes)", len_u64));
                }
                offset += 8;
                len_u64 as usize
            };

            if payload_len > MAX_WS_FRAME_BYTES {
                return Err(format!("WebSocket frame too large ({} bytes)", payload_len));
            }

            let mask_key = if masked {
                if !self.ensure_bytes(offset + 4).await? {
                    return Ok(None);
                }
                let mk = [
                    self.read_buf[offset],
                    self.read_buf[offset + 1],
                    self.read_buf[offset + 2],
                    self.read_buf[offset + 3],
                ];
                offset += 4;
                Some(mk)
            } else {
                None
            };

            if !self.ensure_bytes(offset + payload_len).await? {
                return Ok(None);
            }

            let mut payload = self.read_buf[offset..offset + payload_len].to_vec();
            self.read_buf.drain(..offset + payload_len);

            if let Some(mk) = mask_key {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mk[i & 3];
                }
            }

            match opcode {
                0x8 => return Ok(None), // Close
                0x9 => {
                    // Ping -> Pong
                    self.send_frame(0xA, &payload).await?;
                }
                0xA => {} // Pong
                0x1 => {
                    if ws_message_would_exceed(0, payload.len()) {
                        return Err(format!(
                            "WebSocket message too large ({} bytes)",
                            payload.len()
                        ));
                    }
                    message = payload;
                    if fin {
                        let s = String::from_utf8(message)
                            .map_err(|e| format!("Invalid UTF-8 in WebSocket frame: {}", e))?;
                        return Ok(Some(s));
                    }
                    in_fragmented_text = true;
                }
                0x0 if in_fragmented_text => {
                    if ws_message_would_exceed(message.len(), payload.len()) {
                        return Err(format!(
                            "WebSocket message too large ({} bytes)",
                            message.len() + payload.len()
                        ));
                    }
                    message.extend_from_slice(&payload);
                    if fin {
                        let s = String::from_utf8(message)
                            .map_err(|e| format!("Invalid UTF-8 in WebSocket message: {}", e))?;
                        return Ok(Some(s));
                    }
                }
                _ => {}
            }
        }
    }
}

// ============================================================================
// Headless CLI (--dump-dom / --screenshot) & Reqwest Fallbacks
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn run_cli_browser_fallback(
    browser_path: &Path,
    action: &str,
    url: &str,
    args: &Value,
    width: u32,
    height: u32,
    screenshot_target: Option<&(PathBuf, String, String)>,
    timeout_secs: u64,
    cdp_err: &str,
) -> Result<String, String> {
    let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!(
        "harness_cli_{}_{}_{}",
        std::process::id(),
        Utc::now().timestamp_millis(),
        nonce
    ));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let _guard = TempUserDataDir::new(temp_dir.clone());

    let mut cmd = Command::new(browser_path);
    cmd.arg("--headless=new")
        .arg(format!("--user-data-dir={}", temp_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--virtual-time-budget=2500")
        .arg(format!("--window-size={},{}", width, height));

    #[cfg(target_os = "linux")]
    cmd.arg("--disable-dev-shm-usage");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    #[cfg(unix)]
    cmd.process_group(0);

    if let Some((full_path, _, _)) = screenshot_target {
        cmd.arg(format!("--screenshot={}", full_path.display()));
    } else {
        cmd.arg("--dump-dom");
    }

    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Fallback browser execution failed: {}", e))?;
    let mut stdout_pipe = child.stdout.take();

    let wait_res = timeout(Duration::from_secs(timeout_secs), async {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stdout_pipe {
            match read_to_end_bounded(pipe, MAX_HTTP_BODY_BYTES).await {
                Ok(b) => buf = b,
                Err(e) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(e);
                }
            }
        }
        let status = child
            .wait()
            .await
            .map_err(|e| format!("Failed to wait on browser process: {}", e));
        status.map(|s| (s, buf))
    })
    .await;

    let (status, stdout_bytes) = match wait_res {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            cleanup_child(&mut child).await;
            return Err(format!("Fallback browser execution failed: {}", e));
        }
        Err(_) => {
            cleanup_child(&mut child).await;
            return Err(format!(
                "Fallback browser command timed out after {}s",
                timeout_secs
            ));
        }
    };

    if let Some((full_path, _, _)) = screenshot_target {
        // Only trust the fallback artifact when it is a real PNG image
        // (browsers occasionally leave behind an empty or corrupt file).
        let ok_file = match tokio::fs::File::open(full_path).await {
            Ok(mut f) => {
                let mut magic = [0u8; 8];
                use tokio::io::AsyncReadExt;
                let _ = f.read_exact(&mut magic).await;
                looks_like_png(&magic)
            }
            Err(_) => false,
        };
        if !status.success() || !ok_file {
            return Err(format!(
                "Both CDP ({}) and headless --screenshot fallback failed (exit {:?})",
                cdp_err,
                status.code()
            ));
        }
    }

    let html = String::from_utf8_lossy(&stdout_bytes).to_string();
    let mut summary = if let Some((_, rel_path, url_path)) = screenshot_target {
        json!({
            "action": action,
            "mode": "cli_screenshot_fallback",
            "url": url,
            "title": "",
            "screenshot_url": url_path,
            "screenshot_path": rel_path,
            "console_errors": [format!("CDP unavailable ({}), used --screenshot fallback", cdp_err)],
            "dom_summary": format!("Captured {}x{} screenshot via headless CLI fallback", width, height)
        })
    } else {
        build_html_summary_json(action, "dump_dom", url, &html, args, Some(cdp_err))
    };

    if action == "navigate" && screenshot_target.is_none() {
        if let Some(obj) = summary.as_object_mut() {
            obj.insert("fallback_note".into(), Value::String(cdp_err.to_string()));
        }
    }

    Ok(format_tool_output(action, &summary))
}

async fn execute_no_browser_fallback(
    action: &str,
    url: &str,
    args: &Value,
    timeout_secs: u64,
) -> Result<String, String> {
    if matches!(
        action,
        "screenshot" | "eval" | "interact" | "snapshot" | "debug_audit" | "storage"
    ) {
        return Err(format!(
            "No Edge/Chrome/Chromium executable found for '{}' action. Install Microsoft Edge, Google Chrome, or Chromium, or use action='http_check' for HTTP checks.",
            action
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let res = client
        .get(url)
        .header("User-Agent", "DeepSeek-Harness-BrowserUI/0.1.0")
        .send()
        .await
        .map_err(|e| format!("No browser installed and HTTP fallback failed for '{}': {}", url, e))?;

    let final_url = res.url().to_string();
    let html = res
        .text()
        .await
        .map_err(|e| format!("Failed reading HTTP body from '{}': {}", url, e))?;

    let summary = build_html_summary_json(
        action,
        "reqwest_fallback",
        &final_url,
        &html,
        args,
        Some("No Edge/Chrome/Chromium binary installed; used reqwest HTML parser fallback"),
    );
    Ok(format_tool_output(action, &summary))
}

fn build_html_summary_json(
    action: &str,
    mode: &str,
    url: &str,
    html: &str,
    args: &Value,
    note: Option<&str>,
) -> Value {
    let title = extract_html_title(html).unwrap_or_default();
    let visible_text = truncate_chars(&strip_html_to_text(html), MAX_TEXT_PREVIEW_CHARS);
    let selector = args.get("selector").and_then(|v| v.as_str());
    let selector_count = selector.map(|sel| count_html_selector_matches(html, sel));

    let dom_summary = if let (Some(sel), Some(cnt)) = (selector, selector_count) {
        format!(
            "Parsed HTML '{}' ({}) | selector '{}' matched ~{} element(s)",
            title, url, sel, cnt
        )
    } else {
        format!("Parsed HTML '{}' ({}) | {} bytes", title, url, html.len())
    };

    let mut obj = json!({
        "action": action,
        "mode": mode,
        "url": url,
        "title": title,
        "visible_text": visible_text,
        "screenshot_url": Value::Null,
        "screenshot_path": Value::Null,
        "console_errors": note.map(|n| vec![n.to_string()]).unwrap_or_default(),
        "dom_summary": dom_summary
    });
    if let (Some(sel), Some(cnt), Some(map)) = (selector, selector_count, obj.as_object_mut()) {
        map.insert("selector".into(), Value::String(sel.to_string()));
        map.insert("total_count".into(), Value::from(cnt));
    }
    obj
}

// ============================================================================
// Pure-Rust Utilities: Output Formatting, Base64, HTML Light-Parser
// ============================================================================

fn format_tool_output(action: &str, summary: &Value) -> String {
    let url = summary.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let title = summary
        .get("title")
        .or_else(|| summary.get("page_title"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let dom_summary = summary.get("dom_summary").and_then(|v| v.as_str()).unwrap_or("");
    let err_count = summary
        .get("console_errors")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let failed_req_count = summary
        .get("failed_requests")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let mut out = format!("### Browser UI: `{}` ({})\n", action, url);
    if !title.is_empty() {
        out.push_str(&format!("Title: {}\n", title));
    }
    if !dom_summary.is_empty() {
        out.push_str(&format!("Summary: {}\n", dom_summary));
    }
    if let Some(shot_url) = summary.get("screenshot_url").and_then(|v| v.as_str()) {
        let shot_path = summary
            .get("screenshot_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        out.push_str(&format!("Screenshot: {} ({})\n", shot_url, shot_path));
    }
    out.push_str(&format!("Console Errors: {}\n", err_count));
    if err_count > 0 {
        if let Some(errs) = summary.get("console_errors").and_then(|v| v.as_array()) {
            for e in errs.iter().take(5) {
                if let Some(s) = e.as_str() {
                    out.push_str(&format!("  - {}\n", s));
                }
            }
        }
    }
    if failed_req_count > 0 {
        out.push_str(&format!("Failed Requests: {}\n", failed_req_count));
    }

    let pretty_json = serde_json::to_string_pretty(summary).unwrap_or_else(|_| summary.to_string());
    out.push_str(&format!("\n```json\n{}\n```", pretty_json));
    out
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut iter = s.char_indices();
    if let Some((idx, _)) = iter.nth(max_chars) {
        format!("{}... [truncated]", &s[..idx])
    } else {
        s.to_string()
    }
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let gt = lower[start..].find('>')? + start + 1;
    let end = lower[gt..].find("</title>")? + gt;
    let title = html[gt..end].split_whitespace().collect::<Vec<_>>().join(" ");
    Some(title)
}

fn strip_html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut skip_block = false;
    let mut tag_buf = String::new();

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            tag_buf.clear();
        } else if c == '>' && in_tag {
            in_tag = false;
            let lower = tag_buf.to_ascii_lowercase();
            let name = lower.split_whitespace().next().unwrap_or("");
            if name == "script" || name == "style" || name == "noscript" {
                skip_block = true;
            } else if name == "/script" || name == "/style" || name == "/noscript" {
                skip_block = false;
            } else if matches!(
                name,
                "br" | "p" | "/p" | "div" | "/div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4"
            ) {
                out.push('\n');
            } else {
                out.push(' ');
            }
        } else if in_tag {
            tag_buf.push(c);
        } else if !skip_block {
            out.push(c);
        }
    }

    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lightweight best-effort HTML selector match counter for `http_check` / non-CDP fallbacks.
fn count_html_selector_matches(html: &str, selector: &str) -> usize {
    let sel = selector.trim();
    if sel.is_empty() {
        return 0;
    }

    // Parse simple tag#id.class tokens
    let mut want_tag: Option<&str> = None;
    let mut want_id: Option<&str> = None;
    let mut want_class: Option<&str> = None;

    if let Some(rest) = sel.strip_prefix('#') {
        want_id = Some(rest.split('.').next().unwrap_or(rest));
    } else if let Some(rest) = sel.strip_prefix('.') {
        want_class = Some(rest.split('#').next().unwrap_or(rest));
    } else if let Some((t, r)) = sel.split_once('#') {
        want_tag = Some(t);
        want_id = Some(r.split('.').next().unwrap_or(r));
    } else if let Some((t, r)) = sel.split_once('.') {
        want_tag = Some(t);
        want_class = Some(r);
    } else {
        want_tag = Some(sel);
    }

    let mut count = 0usize;
    let lower = html.to_ascii_lowercase();
    let mut idx = 0;

    while let Some(lt) = lower[idx..].find('<') {
        let tag_start = idx + lt + 1;
        let Some(gt_rel) = lower[tag_start..].find('>') else {
            break;
        };
        let tag_raw = &html[tag_start..tag_start + gt_rel];
        let tag_lower = &lower[tag_start..tag_start + gt_rel];
        idx = tag_start + gt_rel + 1;

        let trimmed = tag_lower.trim_start();
        if trimmed.starts_with('/') || trimmed.starts_with('!') || trimmed.starts_with('?') {
            continue;
        }
        let tag_name = trimmed
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");

        if let Some(wt) = want_tag {
            if tag_name != wt.to_ascii_lowercase() {
                continue;
            }
        }
        if let Some(wi) = want_id {
            let needle1 = format!("id=\"{}\"", wi.to_ascii_lowercase());
            let needle2 = format!("id='{}'", wi.to_ascii_lowercase());
            if !tag_lower.contains(&needle1) && !tag_lower.contains(&needle2) {
                continue;
            }
        }
        if let Some(wc) = want_class {
            let wc_lower = wc.to_ascii_lowercase();
            let attrs = extract_class_tokens(tag_raw);
            if !attrs.iter().any(|c| c.eq_ignore_ascii_case(&wc_lower)) {
                continue;
            }
        }
        count += 1;
    }

    count
}

fn extract_class_tokens(tag: &str) -> Vec<&str> {
    let lower = tag.to_ascii_lowercase();
    for pat in ["class=\"", "class='"] {
        if let Some(start) = lower.find(pat) {
            let quote = pat.as_bytes()[6] as char;
            let val_start = start + pat.len();
            if let Some(end_rel) = tag[val_start..].find(quote) {
                return tag[val_start..val_start + end_rel]
                    .split_whitespace()
                    .collect();
            }
        }
    }
    Vec::new()
}

const B64_ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(B64_ALPHA[((triple >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHA[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHA[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHA[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;

    for b in input.bytes() {
        if b.is_ascii_whitespace() || b == b'=' {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("Invalid base64 byte: {}", b)),
        } as u32;

        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_and_schema_declarations() {
        let plugin = BrowserUiPlugin::new();
        let contract = plugin.contract();
        let schema = plugin.parameter_schema();

        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum");
        for expected in [
            "navigate",
            "screenshot",
            "eval",
            "inspect_dom",
            "interact",
            "snapshot",
            "debug_audit",
            "storage",
            "http_check",
        ] {
            assert!(
                actions.iter().any(|v| v == expected),
                "Missing action '{}'",
                expected
            );
        }

        for field in [
            "preset",
            "element_selector",
            "ref_id",
            "text",
            "steps",
            "actions",
            "storage_op",
            "storage_type",
            "storage_key",
        ] {
            assert!(
                schema["properties"].get(field).is_some(),
                "Missing property '{}' in parameter_schema",
                field
            );
        }

        assert!(schema["properties"]["steps"]["items"]["properties"].is_object());
        assert!(schema["properties"]["actions"]["items"]["properties"].is_object());

        assert!(contract
            .validate_args(&json!({
                "action": "snapshot",
                "url": "http://127.0.0.1:3000",
                "preset": "mobile"
            }))
            .is_ok());

        assert!(contract
            .validate_args(&json!({
                "action": "debug_audit",
                "url": "http://127.0.0.1:3000",
                "preset": "desktop"
            }))
            .is_ok());

        assert!(contract
            .validate_args(&json!({
                "action": "storage",
                "url": "http://127.0.0.1:3000",
                "storage_op": "set",
                "storage_type": "local",
                "storage_key": "token",
                "value": "abc"
            }))
            .is_ok());

        assert!(contract
            .validate_args(&json!({
                "action": "screenshot",
                "url": "http://127.0.0.1:3000",
                "full_page": true,
                "element_selector": "#hero-card"
            }))
            .is_ok());

        assert!(contract
            .validate_args(&json!({
                "action": "snapshot",
                "url": "http://127.0.0.1:3000",
                "preset": "invalid_preset"
            }))
            .is_err());

        assert!(contract
            .validate_args(&json!({
                "action": "interact",
                "url": "http://127.0.0.1:3000",
                "ref_id": 0
            }))
            .is_err());
    }

    #[test]
    fn test_viewport_presets_and_override() {
        let desktop = resolve_viewport(&json!({ "preset": "desktop" })).unwrap();
        assert_eq!((desktop.width, desktop.height, desktop.is_mobile), (1440, 900, false));

        let laptop = resolve_viewport(&json!({ "preset": "laptop" })).unwrap();
        assert_eq!((laptop.width, laptop.height, laptop.is_mobile), (1280, 800, false));

        let tablet = resolve_viewport(&json!({ "preset": "tablet" })).unwrap();
        assert_eq!((tablet.width, tablet.height, tablet.is_mobile), (768, 1024, true));

        let mobile = resolve_viewport(&json!({ "preset": "mobile" })).unwrap();
        assert_eq!((mobile.width, mobile.height, mobile.is_mobile), (390, 844, true));

        let overridden = resolve_viewport(&json!({ "preset": "mobile", "width": 412 })).unwrap();
        assert_eq!((overridden.width, overridden.height, overridden.is_mobile), (412, 844, true));
    }

    #[test]
    fn test_parse_multi_step_workflows_and_aliases() {
        let args = json!({
            "actions": [
                { "action": "click", "ref_id": 2 },
                { "interaction": "dblclick", "text": "Open Item" },
                { "interaction": "fill", "selector": "#email", "value": "a@b.com" },
                { "interaction": "type", "ref_id": "[3]", "value": "secret" },
                { "interaction": "press_key", "value": "Enter" },
                { "interaction": "select_option", "selector": "select#role", "value": "admin" },
                { "interaction": "select", "selector": "select#lang", "value": "en" },
                { "interaction": "hover", "selector": ".menu" },
                { "interaction": "wait_for_selector", "selector": ".toast", "wait_ms": 2000 },
                { "interaction": "wait_ms", "wait_ms": 150 },
                { "interaction": "scroll_by", "y": 400 },
                { "interaction": "scroll_to", "value": "bottom" },
                { "interaction": "eval", "expression": "document.title" },
                { "interaction": "screenshot", "screenshot_name": "after_login" },
                { "interaction": "assert_text", "selector": "h1", "value": "Welcome" },
                { "interaction": "assert_selector", "selector": ".dashboard" }
            ]
        });

        let steps = parse_interaction_steps(&args, 300).unwrap();
        assert_eq!(steps.len(), 16);
        assert_eq!(steps[0].interaction, "click");
        assert_eq!(steps[0].ref_id, Some(2));
        assert_eq!(steps[1].text.as_deref(), Some("Open Item"));
        assert_eq!(steps[3].ref_id, Some(3));
        assert_eq!(steps[6].interaction, "select_option");
        assert_eq!(steps[10].y, Some(400.0));
        assert_eq!(steps[12].expression.as_deref(), Some("document.title"));
        assert_eq!(steps[13].screenshot_name.as_deref(), Some("after_login"));

        let canvas_clicks = parse_interaction_steps(
            &json!({
                "steps": [
                    { "interaction": "click", "x": 220, "y": 140 },
                    { "interaction": "click", "selector": "canvas#game", "x": 300, "y": 300 },
                    { "interaction": "hover", "x": 15, "y": 25 }
                ]
            }),
            300,
        )
        .unwrap();
        assert_eq!(canvas_clicks.len(), 3);
        assert_eq!((canvas_clicks[0].x, canvas_clicks[0].y), (Some(220.0), Some(140.0)));
        assert_eq!(canvas_clicks[1].selector.as_deref(), Some("canvas#game"));
        assert_eq!((canvas_clicks[1].x, canvas_clicks[1].y), (Some(300.0), Some(300.0)));
    }

    #[test]
    fn test_contract_mentions_workspace_files_and_file_urls() {
        let plugin = BrowserUiPlugin::new();
        let desc = plugin.description().to_ascii_lowercase();
        assert!(desc.contains("file:///"));
        assert!(desc.contains("index.html"));

        let schema = plugin.parameter_schema();
        let url_desc = schema["properties"]["url"]["description"]
            .as_str()
            .expect("url description")
            .to_ascii_lowercase();
        assert!(url_desc.contains("file:///"));
        assert!(url_desc.contains("index.html"));
        assert!(url_desc.contains("127.0.0.1"));
    }

    #[test]
    fn test_validate_url_schemes_and_local_paths() {
        // http_check strictly requires http:// or https://
        assert!(validate_url("http://127.0.0.1:3000", true).is_ok());
        assert!(validate_url("https://example.com/app", true).is_ok());
        assert!(validate_url("file:///tmp/index.html", true).is_err());
        assert!(validate_url("index.html", true).is_err());
        assert!(validate_url("about:blank", true).is_err());
        assert!(validate_url("data:text/html,<h1>Hi</h1>", true).is_err());

        // CDP actions accept standard URLs, workspace file:// URLs, and workspace-relative paths
        assert!(validate_url("http://127.0.0.1:3000", false).is_ok());
        assert!(validate_url("https://example.com", false).is_ok());
        assert!(validate_url("about:blank", false).is_ok());
        assert!(validate_url("data:text/html,<h1>Hi</h1>", false).is_ok());
        assert!(validate_url("file:///C:/workspace/index.html", false).is_ok());
        assert!(validate_url("file://localhost/tmp/index.html", false).is_ok());
        assert!(validate_url("index.html", false).is_ok());
        assert!(validate_url("./dist/index.html", false).is_ok());
        assert!(validate_url("sub/game.html?level=2#start", false).is_ok());

        // Disallowed schemes, absolute paths, and traversal attempts
        assert!(validate_url("", false).is_err());
        assert!(validate_url("ftp://example.com/a.html", false).is_err());
        assert!(validate_url("javascript:alert(1)", false).is_err());
        assert!(validate_url("chrome://settings", false).is_err());
        assert!(validate_url("/etc/passwd", false).is_err());
        assert!(validate_url(r"C:\Windows\win.ini", false).is_err());
        assert!(validate_url("../secret.html", false).is_err());
        assert!(validate_url("sub/../../secret.html", false).is_err());
        assert!(validate_url("file://evil.com/index.html", false).is_err());
    }

    fn path_to_file_url(path: &Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        if s.starts_with('/') {
            format!("file://{}", s)
        } else {
            format!("file:///{}", s)
        }
    }

    #[tokio::test]
    async fn test_local_target_resolution_and_sandbox_guards() {
        let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "harness_browser_ui_res_test_{}_{}",
            std::process::id(),
            nonce
        ));
        let sandbox = root.join("workspace");
        let outside = root.join("outside");
        tokio::fs::create_dir_all(sandbox.join("dist"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(sandbox.join("my game"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        tokio::fs::write(sandbox.join("index.html"), "<h1>Home</h1>")
            .await
            .unwrap();
        tokio::fs::write(sandbox.join("dist").join("app.html"), "<h1>App</h1>")
            .await
            .unwrap();
        tokio::fs::write(sandbox.join("my game").join("play.html"), "<h1>Game</h1>")
            .await
            .unwrap();
        tokio::fs::write(outside.join("secret.html"), "<h1>Secret</h1>")
            .await
            .unwrap();

        // Pass-through URLs do not spawn a server
        let (u, srv) = resolve_target_url("https://example.com/a", &sandbox)
            .await
            .unwrap();
        assert_eq!(u, "https://example.com/a");
        assert!(srv.is_none());

        // Relative paths inside sandbox
        let (u_rel, srv_rel) = resolve_target_url("index.html", &sandbox).await.unwrap();
        assert!(u_rel.starts_with("http://127.0.0.1:"));
        assert!(u_rel.ends_with("/index.html"));
        drop(srv_rel);

        let (u_sub, srv_sub) = resolve_target_url("./dist/app.html?mode=1#run", &sandbox)
            .await
            .unwrap();
        assert!(u_sub.ends_with("/dist/app.html?mode=1#run"));
        drop(srv_sub);

        // file:// URL inside sandbox (including space percent-encoding)
        let canon_game = tokio::fs::canonicalize(sandbox.join("my game").join("play.html"))
            .await
            .unwrap();
        // Strip Windows \\?\ verbatim prefix to simulate real user/model file:/// URLs
        let clean_game_str = canon_game
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .map(str::to_string)
            .unwrap_or_else(|| canon_game.to_string_lossy().into_owned());
        let file_url = path_to_file_url(Path::new(&clean_game_str)).replace(' ', "%20");
        let (u_file, srv_file) = resolve_target_url(&file_url, &sandbox).await.unwrap();
        assert!(u_file.ends_with("/my%20game/play.html"));
        drop(srv_file);

        // Reject file:// outside sandbox (existing and non-existing)
        let outside_file_url = path_to_file_url(&outside.join("secret.html"));
        let err_out = resolve_target_url(&outside_file_url, &sandbox)
            .await
            .err()
            .expect("outside file:// must be rejected");
        assert!(
            err_out.contains("Sandbox violation"),
            "Unexpected error: {}",
            err_out
        );

        let outside_missing_url = path_to_file_url(&outside.join("does_not_exist.html"));
        let err_out_missing = resolve_target_url(&outside_missing_url, &sandbox)
            .await
            .err()
            .expect("outside nonexistent file:// must be rejected as sandbox violation");
        assert!(
            err_out_missing.contains("Sandbox violation"),
            "Unexpected error: {}",
            err_out_missing
        );

        // Reject traversal and nonexistent inside sandbox
        let err_trav = resolve_target_url("../outside/secret.html", &sandbox)
            .await
            .err()
            .expect(".. traversal must fail");
        assert!(err_trav.contains("Sandbox violation"), "{}", err_trav);

        let err_missing = resolve_target_url("missing.html", &sandbox)
            .await
            .err()
            .expect("missing relative file must fail");
        assert!(err_missing.contains("does not exist"), "{}", err_missing);

        // http_check plugin execute still rejects local paths
        let plugin = BrowserUiPlugin::new();
        let http_check_err = plugin
            .execute(
                json!({ "action": "http_check", "url": "index.html" }),
                &sandbox,
            )
            .await
            .err()
            .expect("http_check must reject relative paths");
        assert!(http_check_err.contains("http:// or https://"));

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn test_ephemeral_workspace_server_serves_mime_types_and_shuts_down() {
        let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let sandbox = std::env::temp_dir().join(format!(
            "harness_browser_ui_srv_test_{}_{}",
            std::process::id(),
            nonce
        ));
        tokio::fs::create_dir_all(sandbox.join("assets"))
            .await
            .unwrap();

        let files: &[(&str, &[u8], &str)] = &[
            ("index.html", b"<!doctype html><script type='module' src='./main.mjs'></script>", "text/html"),
            ("app.js", b"console.log('js');", "application/javascript"),
            ("main.mjs", b"export const x = 1;", "application/javascript"),
            ("style.css", b"body { color: red; }", "text/css"),
            ("data.json", br#"{"ok":true}"#, "application/json"),
            ("assets/logo.svg", b"<svg></svg>", "image/svg+xml"),
            ("assets/img.png", b"\x89PNG\r\n\x1a\n", "image/png"),
            ("assets/photo.jpg", b"\xFF\xD8\xFF", "image/jpeg"),
            ("assets/pic.webp", b"RIFFWEBP", "image/webp"),
            ("mod.wasm", b"\x00asm\x01\x00\x00\x00", "application/wasm"),
            ("readme.txt", b"hello world", "text/plain"),
        ];
        for (rel, bytes, _) in files {
            tokio::fs::write(sandbox.join(rel), bytes).await.unwrap();
        }

        let (mapped_url, server_opt) = resolve_target_url("index.html", &sandbox).await.unwrap();
        let server = server_opt.expect("ephemeral server must be created");
        let base_url = format!("http://127.0.0.1:{}", server.port());
        assert_eq!(mapped_url, format!("{}/index.html", base_url));

        let client = reqwest::Client::new();
        for (rel, expected_bytes, expected_mime_prefix) in files {
            let resp = client
                .get(format!("{}/{}", base_url, rel))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200, "GET /{} failed", rel);
            let ct = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(
                ct.starts_with(expected_mime_prefix),
                "Unexpected Content-Type '{}' for '{}'",
                ct,
                rel
            );
            let body = resp.bytes().await.unwrap();
            assert_eq!(body.as_ref(), *expected_bytes, "Body mismatch for '{}'", rel);
        }

        // Missing /favicon.ico returns 204 No Content so browsers don't emit spurious 404 console errors
        let resp_fav = client
            .get(format!("{}/favicon.ico", base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(resp_fav.status(), 204);

        // 404 for nonexistent workspace file
        let resp_404 = client
            .get(format!("{}/missing.js", base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(resp_404.status(), 404);

        // 405 for non-GET/HEAD method
        let resp_405 = client
            .post(format!("{}/index.html", base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(resp_405.status(), 405);

        // Raw socket traversal attempt must be rejected with 403 Forbidden
        let mut raw_stream = TcpStream::connect(format!("127.0.0.1:{}", server.port()))
            .await
            .unwrap();
        raw_stream
            .write_all(b"GET /%2e%2e/outside.txt HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut raw_resp = Vec::new();
        raw_stream.read_to_end(&mut raw_resp).await.unwrap();
        let raw_resp_str = String::from_utf8_lossy(&raw_resp);
        assert!(
            raw_resp_str.starts_with("HTTP/1.1 403 Forbidden"),
            "Expected 403 Forbidden, got: {}",
            raw_resp_str
        );

        let port = server.port();
        drop(server);
        tokio::task::yield_now().await;
        sleep(Duration::from_millis(30)).await;

        // Server socket must no longer accept connections after drop
        let post_drop = TcpStream::connect(format!("127.0.0.1:{}", port)).await;
        assert!(
            post_drop.is_err(),
            "Ephemeral server port {} remained open after drop",
            port
        );

        let _ = tokio::fs::remove_dir_all(&sandbox).await;
    }

    #[test]
    fn test_static_server_denies_hidden_harness_paths() {
        // Harness-internal state and other dotfiles must never be served
        for denied in [
            ".harness",
            ".harness/tools/example.json",
            ".harness/skills/browser/SKILL.md",
            ".harness_artifacts",
            ".harness_artifacts/screenshots/shot.png",
            ".git",
            ".git/config",
            ".gitignore",
            ".env",
        ] {
            let rel = Path::new(denied);
            assert!(
                is_hidden_harness_path(rel),
                "'{}' must be treated as a hidden harness path",
                denied
            );
        }

        // Ordinary workspace files (including nested ones) must still be served
        for allowed in [
            "index.html",
            "dist/app.js",
            "assets/logo.svg",
            "favicon.ico",
            "my game/play.html",
        ] {
            let rel = Path::new(allowed);
            assert!(
                !is_hidden_harness_path(rel),
                "'{}' must remain servable",
                allowed
            );
        }
    }

    #[test]
    fn test_static_server_denylist_actually_404s() {
        // End-to-end: the ephemeral server must answer 403 (not 200) for
        // .harness/.harness_artifacts/dotfile paths that exist on disk.
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
                let sandbox = std::env::temp_dir().join(format!(
                    "harness_browser_ui_denylist_test_{}_{}",
                    std::process::id(),
                    nonce
                ));
                tokio::fs::create_dir_all(sandbox.join(".harness").join("tools"))
                    .await
                    .unwrap();
                tokio::fs::create_dir_all(sandbox.join(".harness_artifacts"))
                    .await
                    .unwrap();
                tokio::fs::write(sandbox.join("index.html"), "<h1>ok</h1>")
                    .await
                    .unwrap();
                tokio::fs::write(
                    sandbox.join(".harness").join("tools").join("t.json"),
                    b"{\"secret\":true}",
                )
                .await
                .unwrap();
                tokio::fs::write(sandbox.join(".harness_artifacts").join("out.txt"), b"x")
                    .await
                    .unwrap();

                let (_, server_opt) = resolve_target_url("index.html", &sandbox).await.unwrap();
                let server = server_opt.expect("server");
                let base = format!("http://127.0.0.1:{}", server.port());
                let client = reqwest::Client::new();

                for rel in [
                    "/.harness/tools/t.json",
                    "/.harness_artifacts/out.txt",
                    "/.harness/tools/t.json?x=1",
                ] {
                    let resp = client.get(format!("{}{}", base, rel)).send().await.unwrap();
                    assert_eq!(resp.status(), 403, "GET {} must be 403 Forbidden", rel);
                    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"Forbidden");
                }

                // The visible index.html keeps working
                let ok = client.get(format!("{}/index.html", base)).send().await.unwrap();
                assert_eq!(ok.status(), 200);

                drop(server);
                let _ = tokio::fs::remove_dir_all(&sandbox).await;
            });
    }

    #[test]
    fn test_png_magic_signature_helper() {
        assert!(looks_like_png(b"\x89PNG\r\n\x1a\n"));
        assert!(looks_like_png(
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRxxxxxxxx"
        ));
        assert!(!looks_like_png(b""));
        assert!(!looks_like_png(b"\x89PNG\r\n\x1a")); // truncated magic
        assert!(!looks_like_png(b"\x89PNG\r\n\x1a\x00")); // wrong 8th byte
        assert!(!looks_like_png(b"RIFFWEBP"));
        assert!(!looks_like_png(b"\xFF\xD8\xFF")); // JPEG magic
        assert!(!looks_like_png(b"not-an-image-at-all"));
    }

    #[test]
    fn test_ws_fragmented_message_total_budget() {
        let max = MAX_WS_MESSAGE_BYTES;
        assert!(!ws_message_would_exceed(0, 1));
        assert!(!ws_message_would_exceed(0, max));
        assert!(!ws_message_would_exceed(max - 1, 1));
        // Crossing the cumulative cap across continuation frames must be rejected
        assert!(ws_message_would_exceed(max - 1, 2));
        assert!(ws_message_would_exceed(max, 1));
        assert!(ws_message_would_exceed(max, max));
        // Per-frame cap alone does not trip the cumulative budget
        assert_eq!(max, MAX_WS_FRAME_BYTES * 4);
    }

    #[tokio::test]
    async fn test_read_to_end_bounded_enforces_cap() {
        let data = vec![0x41u8; 64];
        let mut reader = &data[..];
        let out = read_to_end_bounded(&mut reader, 1_000_000).await.unwrap();
        assert_eq!(out.len(), 64);

        // Exactly at the limit still succeeds; one byte over must fail
        let mut exact = &data[..];
        let ok_exact = read_to_end_bounded(&mut exact, 64).await.unwrap();
        assert_eq!(ok_exact.len(), 64);

        let mut over = &data[..];
        let err = read_to_end_bounded(&mut over, 63).await.unwrap_err();
        assert!(err.contains("read limit"), "unexpected error: {}", err);

        // A large payload far past the 8 MB body cap must fail, not buffer forever
        let big = vec![0x42u8; MAX_HTTP_BODY_BYTES + 1024];
        let mut big_reader = &big[..];
        let big_err = read_to_end_bounded(&mut big_reader, MAX_HTTP_BODY_BYTES)
            .await
            .unwrap_err();
        assert!(big_err.contains("read limit"), "unexpected error: {}", big_err);
        assert_eq!(MAX_HTTP_BODY_BYTES, 8 * 1024 * 1024);
    }
}
