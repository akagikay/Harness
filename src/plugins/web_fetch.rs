use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::ToolPlugin;

/// Hard cap on the buffered response body (4 MB). Anything larger is read
/// partially and reported back with a visible truncation notice.
const MAX_FETCH_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Validate a fetch target URL: only http/https may be requested, which keeps
/// local files (`file:`), plaintext/credentialed schemes (`ftp:`, `data:`) and
/// SSRF attempts through internal service schemes out of the tool.
fn validate_fetch_url(url_str: &str) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url_str).map_err(|_| {
        format!(
            "Invalid URL '{}': provide an http:// or https:// URL",
            url_str
        )
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(format!(
            "Invalid URL '{}': scheme '{}:' is not supported, only http/https may be fetched",
            url_str, other
        )),
    }
}

/// Read at most `cap` bytes of a response body (streamed, so a malicious
/// `Content-Length` cannot exhaust memory) and report whether it was truncated.
async fn read_body_capped(
    res: reqwest::Response,
    cap: usize,
) -> Result<(String, bool), String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut stream = res.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                return Err(format!("Failed to read response body: {}", e));
            }
        };
        if buf.len() + chunk.len() > cap {
            let keep = cap.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..keep]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }

    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

/// Append the truncation notice when a body hit the hard byte cap.
fn with_truncation_notice(body: String, truncated: bool, cap: usize) -> String {
    if truncated {
        let mut out = body;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("[truncated: response exceeded {} bytes]", cap));
        return out;
    }
    body
}

pub struct WebFetchPlugin {
    pub api_key: String,
}

impl WebFetchPlugin {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl ToolPlugin for WebFetchPlugin {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch and convert a web page into clean LLM-ready markdown using TinyFish Fetch (or native HTTP fallback)."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "web".to_string(),
            permission_level: ToolPermission::ReadOnly,
            fields: vec![
                ToolFieldSpec::required_string(
                    "url",
                    "The HTTP or HTTPS URL to fetch and convert to markdown.",
                    "Provide a non-empty URL starting with http:// or https://.",
                )
                .non_empty(),
                ToolFieldSpec::optional_string(
                    "purpose",
                    "Optional short summary of what information you want to extract from this page.",
                )
                .with_default(json!("Extract relevant technical content"))
                .with_error_hint("Provide a short string describing what information to extract."),
            ],
            expected_output: OutputContract::new(
                "markdown",
                "Clean markdown or stripped plain-text content extracted from the fetched web page.",
                "### Content from https://example.com:\n\nExample Domain\nThis domain is for use in illustrative examples...",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "network_or_dns_failure",
                    "Target URL is malformed, unreachable, or times out after 30s",
                    "Failed to fetch URL '<url>': <error>",
                    "Verify the URL includes http:// or https:// and is reachable from this host.",
                ),
                ToolFailureMode::new(
                    "http_status_error",
                    "Server returns a non-success HTTP status code (4xx/5xx)",
                    "HTTP error <status>: <url>",
                    "Check that the URL is valid and publicly accessible, or locate another source with web_search.",
                ),
                ToolFailureMode::new(
                    "body_decode_error",
                    "Response body cannot be decoded as text",
                    "Failed to read response body from '<url>': <error>",
                    "Ensure the URL points to an HTML or text resource rather than a binary download.",
                ),
            ],
            timeout_secs: 30,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, _sandbox: &Path) -> Result<String, String> {
        let url_str = args.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'url'".to_string())?;

        let purpose = args.get("purpose").and_then(|v| v.as_str()).unwrap_or("Extract relevant technical content");

        // Reject non-http(s) targets before any network call happens (no
        // file://, ftp://, data: or other local/secret-reading schemes).
        let target_url = validate_fetch_url(url_str)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // 1. Try TinyFish Fetch if API key is provided
        if !self.api_key.is_empty() {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            if let Ok(hv) = HeaderValue::from_str(&self.api_key) {
                headers.insert("X-API-Key", hv);
            }

            let payload = json!({
                "urls": [url_str],
                "format": "markdown",
                "purpose": purpose,
                "links": false,
                "image_links": false
            });

            if let Ok(res) = client.post("https://api.fetch.tinyfish.ai")
                .headers(headers)
                .json(&payload)
                .send()
                .await
            {
                if res.status().is_success() {
                    let (text, truncated) = read_body_capped(res, MAX_FETCH_BODY_BYTES)
                        .await
                        .unwrap_or_default();
                    let text = with_truncation_notice(text, truncated, MAX_FETCH_BODY_BYTES);
                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                        if let Some(content) = val.get("content")
                            .or_else(|| val.get("markdown"))
                            .or_else(|| val.get("results").and_then(|r| r.get(0)).and_then(|r| r.get("content")))
                            .and_then(|c| c.as_str())
                        {
                            return Ok(content.to_string());
                        }
                    }
                    return Ok(text);
                }
            }
        }

        // 2. High-performance native fallback HTTP fetch
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HeaderValue::from_static("DeepSeek-Harness-Agent/0.1.0"));

        let res = client.get(target_url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch URL '{}': {}", url_str, e))?;

        if !res.status().is_success() {
            return Err(format!("HTTP error {}: {}", res.status(), url_str));
        }

        let (raw_html, truncated) = read_body_capped(res, MAX_FETCH_BODY_BYTES)
            .await
            .map_err(|e| format!("Failed to read response body from '{}': {}", url_str, e))?;
        if truncated {
            let clean_text = strip_html_tags(&raw_html);
            return Ok(format!(
                "### Content from {}:\n\n{}\n\n[truncated: response exceeded {} bytes]",
                url_str, clean_text, MAX_FETCH_BODY_BYTES
            ));
        }

        let clean_text = strip_html_tags(&raw_html);
        Ok(format!("### Content from {}:\n\n{}", url_str, clean_text))
    }
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    #[test]
    fn only_http_https_urls_are_accepted() {
        for ok in [
            "https://example.com",
            "http://example.com/docs?a=1#frag",
            "https://127.0.0.1:8080/health",
        ] {
            let parsed = validate_fetch_url(ok)
                .unwrap_or_else(|e| panic!("'{}' should be accepted: {}", ok, e));
            assert_eq!(parsed.scheme(), ok.split(':').next().unwrap());
        }

        for rejected in [
            "",
            "not a url",
            "file:///C:/Windows/win.ini",
            "file:///etc/passwd",
            "ftp://example.com/file",
            "data:text/html,<h1>hi</h1>",
            "javascript:alert(1)",
            "redis://127.0.0.1:6379",
        ] {
            let err = validate_fetch_url(rejected)
                .expect_err(&format!("'{}' must be rejected", rejected));
            assert!(
                err.contains("Invalid URL"),
                "'{}' should report an invalid URL, got: {}",
                rejected,
                err
            );
        }
    }

    #[test]
    fn truncation_notice_is_appended_once() {
        assert_eq!(with_truncation_notice("body".into(), false, 4), "body");
        assert_eq!(
            with_truncation_notice("body".into(), true, 4),
            "body\n[truncated: response exceeded 4 bytes]"
        );
        assert_eq!(
            with_truncation_notice(String::new(), true, 4),
            "[truncated: response exceeded 4 bytes]"
        );
    }
}

/// Lightweight, zero-dependency HTML to plain-text / readable markdown converter
fn strip_html_tags(html: &str) -> String {
    let mut output = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script_or_style = false;
    let mut current_tag = String::new();

    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            in_tag = true;
            current_tag.clear();
        } else if c == '>' && in_tag {
            in_tag = false;
            let tag_lower = current_tag.to_lowercase();
            let tag_name = tag_lower.split_whitespace().next().unwrap_or("");

            if tag_name == "script" || tag_name == "style" {
                in_script_or_style = true;
            } else if tag_name == "/script" || tag_name == "/style" {
                in_script_or_style = false;
            } else if tag_name == "br" || tag_name == "p" || tag_name == "/p" || tag_name == "div" || tag_name == "/div" || tag_name == "li" || tag_name == "h1" || tag_name == "h2" || tag_name == "h3" {
                output.push('\n');
            }
        } else if in_tag {
            current_tag.push(c);
        } else if !in_script_or_style {
            output.push(c);
        }
    }

    // Clean up excessive blank lines
    let mut compacted = String::new();
    let mut last_was_empty = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !last_was_empty {
                compacted.push('\n');
                last_was_empty = true;
            }
        } else {
            compacted.push_str(trimmed);
            compacted.push('\n');
            last_was_empty = false;
        }
    }

    compacted
}
