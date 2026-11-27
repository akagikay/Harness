use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

use crate::config::SearchConfig;
use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::ToolPlugin;

/// Clamp an optional `max_results` argument into the contract's advertised
/// range so a caller can neither request zero results nor flood a provider.
fn clamp_max_results(requested: Option<u64>, config_default: usize) -> usize {
    requested
        .unwrap_or(config_default as u64)
        .clamp(1, 50) as usize
}

/// Summarize an upstream provider error body as `status + short message`.
///
/// Raw provider payloads (which routinely embed API keys, session tokens,
/// internal URLs or megabytes of HTML) must never be echoed back to the model,
/// so structured bodies are withheld and only the HTTP status is reported.
pub fn redact_provider_error(status: reqwest::StatusCode, body: &str) -> String {
    let trimmed = body.trim();
    let structured = trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with('<');
    if structured {
        return format!("{} (response body withheld)", status);
    }

    let first_line = trimmed.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return format!("{} (response body withheld)", status);
    }

    let excerpt: String = first_line.chars().take(120).collect();
    format!("{}: {}", status, excerpt)
}

pub struct WebSearchPlugin {
    pub config: SearchConfig,
}

impl WebSearchPlugin {
    pub fn new(config: SearchConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolPlugin for WebSearchPlugin {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the live web for technical documentation, code snippets, error fixes, or live information using configured search providers (TinyFish, DuckDuckGo, Tavily, Brave, SearXNG)."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "web".to_string(),
            permission_level: ToolPermission::ReadOnly,
            fields: vec![
                ToolFieldSpec::required_string(
                    "query",
                    "The search query string.",
                    "Provide a non-empty search query string.",
                )
                .non_empty(),
                ToolFieldSpec::optional_int(
                    "max_results",
                    "Optional maximum number of search results to return (default: 5).",
                )
                .with_range(Some(1.0), Some(50.0))
                .with_default(json!(5))
                .with_error_hint("Provide an integer between 1 and 50 for max_results."),
            ],
            expected_output: OutputContract::new(
                "markdown",
                "Numbered markdown list of web search results with titles, URLs, and snippets.",
                "### Web Search Results (DuckDuckGo) for: 'rust async'\n\n1. [Asynchronous Programming in Rust](https://rust-lang.github.io/async-book/)\n   Official Rust async book.",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_api_key",
                    "Selected search provider (Tavily or Brave) has no API key configured",
                    "Tavily/Brave Search API key is not configured.",
                    "Configure the provider API key in Settings or switch search provider to DuckDuckGo/SearXNG.",
                ),
                ToolFailureMode::new(
                    "network_or_timeout",
                    "HTTP request fails or exceeds the 15-second timeout",
                    "Web search request failed: <network error>",
                    "Check network connectivity and retry with a concise query.",
                ),
                ToolFailureMode::new(
                    "provider_api_error",
                    "Upstream search provider returns a non-2xx HTTP status or invalid JSON",
                    "Search API error <status>: <short redacted message>",
                    "Retry later or switch to a fallback search provider.",
                ),
            ],
            timeout_secs: 20,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, _sandbox: &Path) -> Result<String, String> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'query'".to_string())?;

        let max_results = clamp_max_results(
            args.get("max_results").and_then(|v| v.as_u64()),
            self.config.max_results,
        );

        execute_search(&self.config, query, max_results).await
    }
}

pub async fn execute_search(config: &SearchConfig, query: &str, max_results: usize) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    match config.active_provider.as_str() {
        "tavily" => search_tavily(&client, config, query, max_results).await,
        "brave" => search_brave(&client, config, query, max_results).await,
        "duckduckgo" => search_duckduckgo(&client, query, max_results).await,
        "searxng" => search_searxng(&client, config, query, max_results).await,
        _ => {
            // Default to TinyFish
            if !config.tinyfish_api_key.is_empty() {
                search_tinyfish(&client, config, query, max_results).await
            } else {
                // If no TinyFish key, fallback to DuckDuckGo
                search_duckduckgo(&client, query, max_results).await
            }
        }
    }
}

async fn search_tinyfish(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    max_results: usize,
) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    if !config.tinyfish_api_key.is_empty() {
        if let Ok(hv) = HeaderValue::from_str(&config.tinyfish_api_key) {
            headers.insert("X-API-Key", hv);
        }
    }

    let encoded_query = urlencoding::encode(query);
    let url = format!("https://api.search.tinyfish.ai?query={}", encoded_query);

    let res = client.get(&url).headers(headers).send().await;

    match res {
        Ok(response) => {
            if response.status().is_success() {
                let text = response.text().await.unwrap_or_default();
                if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                    let mut formatted = format!("### Web Search Results (TinyFish) for: '{}'\n\n", query);
                    let items = json_val.get("results")
                        .or_else(|| json_val.get("items"))
                        .or_else(|| json_val.as_array().map(|_| &json_val));

                    if let Some(results_arr) = items.and_then(|i| i.as_array()) {
                        for (idx, item) in results_arr.iter().take(max_results).enumerate() {
                            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("No title");
                            let link = item.get("url").or_else(|| item.get("link")).and_then(|u| u.as_str()).unwrap_or("#");
                            let snippet = item.get("snippet").or_else(|| item.get("description")).and_then(|s| s.as_str()).unwrap_or("");
                            formatted.push_str(&format!("{}. [{}]({})\n   {}\n\n", idx + 1, title, link, snippet));
                        }
                        return Ok(formatted);
                    } else {
                        return Ok(format!("Search returned:\n{}", text));
                    }
                } else {
                    return Ok(format!("Search response:\n{}", text));
                }
            } else {
                let status = response.status();
                let err_body = response.text().await.unwrap_or_default();
                // Fallback to DuckDuckGo on quota/key error
                if status.as_u16() == 401 || status.as_u16() == 429 {
                    return search_duckduckgo(client, query, max_results).await;
                }
                Err(format!(
                    "TinyFish Search API error: {}",
                    redact_provider_error(status, &err_body)
                ))
            }
        }
        Err(e) => {
            // Fallback to DuckDuckGo
            search_duckduckgo(client, query, max_results).await
                .map_err(|_| format!("Web search request failed: {}", e))
        }
    }
}

async fn search_duckduckgo(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<String, String> {
    let encoded_query = urlencoding::encode(query);
    // Use DuckDuckGo Instant Answer API with html fallback
    let url = format!("https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1", encoded_query);

    let res = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await;

    match res {
        Ok(response) if response.status().is_success() => {
            let val = response.json::<Value>().await.unwrap_or_default();
            let mut formatted = format!("### Web Search Results (DuckDuckGo) for: '{}'\n\n", query);
            let mut count = 0;

            // Heading & Abstract
            if let Some(abs) = val.get("AbstractText").and_then(|a| a.as_str()) {
                if !abs.is_empty() {
                    let source = val.get("AbstractURL").and_then(|u| u.as_str()).unwrap_or("#");
                    let heading = val.get("Heading").and_then(|h| h.as_str()).unwrap_or("Summary");
                    formatted.push_str(&format!("1. [{}]({})\n   {}\n\n", heading, source, abs));
                    count += 1;
                }
            }

            // Related topics
            if let Some(topics) = val.get("RelatedTopics").and_then(|t| t.as_array()) {
                for item in topics {
                    if count >= max_results {
                        break;
                    }
                    if let Some(text) = item.get("Text").and_then(|t| t.as_str()) {
                        let link = item.get("FirstURL").and_then(|u| u.as_str()).unwrap_or("#");
                        count += 1;
                        formatted.push_str(&format!("{}. [{}]({})\n\n", count, text, link));
                    }
                }
            }

            if count > 0 {
                Ok(formatted)
            } else {
                Ok(format!("### Web Search (DuckDuckGo)\nNo direct instant answers for '{}'. Try refining query keywords.", query))
            }
        }
        _ => Err("DuckDuckGo instant search query failed".to_string()),
    }
}

async fn search_tavily(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    max_results: usize,
) -> Result<String, String> {
    if config.tavily_api_key.is_empty() {
        return Err("Tavily API key is not configured. Please enter your Tavily key in Search Settings.".to_string());
    }

    let payload = serde_json::json!({
        "api_key": config.tavily_api_key,
        "query": query,
        "max_results": max_results,
        "search_depth": "basic",
        "include_answer": true
    });

    let res = client.post("https://api.tavily.com/search")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Tavily request failed: {}", e))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "Tavily API error: {}",
            redact_provider_error(status, &body)
        ));
    }

    let val = res.json::<Value>().await
        .map_err(|e| format!("Failed to parse Tavily JSON: {}", e))?;

    let mut formatted = format!("### Web Search Results (Tavily) for: '{}'\n\n", query);
    if let Some(answer) = val.get("answer").and_then(|a| a.as_str()) {
        formatted.push_str(&format!("**AI Summary**: {}\n\n", answer));
    }

    if let Some(results) = val.get("results").and_then(|r| r.as_array()) {
        for (idx, item) in results.iter().take(max_results).enumerate() {
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("No title");
            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("#");
            let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
            formatted.push_str(&format!("{}. [{}]({})\n   {}\n\n", idx + 1, title, url, content));
        }
    }

    Ok(formatted)
}

async fn search_brave(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    max_results: usize,
) -> Result<String, String> {
    if config.brave_api_key.is_empty() {
        return Err("Brave Search API key is not configured. Please enter your Brave key in Search Settings.".to_string());
    }

    let encoded_query = urlencoding::encode(query);
    let url = format!("https://api.search.brave.com/res/v1/web/search?q={}&count={}", encoded_query, max_results);

    let res = client.get(&url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", &config.brave_api_key)
        .send()
        .await
        .map_err(|e| format!("Brave search request failed: {}", e))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "Brave Search API error: {}",
            redact_provider_error(status, &body)
        ));
    }

    let val = res.json::<Value>().await
        .map_err(|e| format!("Failed to parse Brave JSON: {}", e))?;

    let mut formatted = format!("### Web Search Results (Brave) for: '{}'\n\n", query);
    if let Some(results) = val.get("web").and_then(|w| w.get("results")).and_then(|r| r.as_array()) {
        for (idx, item) in results.iter().take(max_results).enumerate() {
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("No title");
            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("#");
            let desc = item.get("description").and_then(|d| d.as_str()).unwrap_or("");
            formatted.push_str(&format!("{}. [{}]({})\n   {}\n\n", idx + 1, title, url, desc));
        }
    }

    Ok(formatted)
}

async fn search_searxng(
    client: &reqwest::Client,
    config: &SearchConfig,
    query: &str,
    max_results: usize,
) -> Result<String, String> {
    let base = if config.searxng_url.is_empty() {
        "https://searx.be"
    } else {
        config.searxng_url.trim_end_matches('/')
    };

    let encoded_query = urlencoding::encode(query);
    let url = format!("{}/search?q={}&format=json", base, encoded_query);

    let res = client.get(&url)
        .header("User-Agent", "DeepSeekHarness/1.0")
        .send()
        .await
        .map_err(|e| format!("SearXNG request failed: {}", e))?;

    if !res.status().is_success() {
        let status = res.status();
        return Err(format!("SearXNG API error {}", status));
    }

    let val = res.json::<Value>().await
        .map_err(|e| format!("Failed to parse SearXNG JSON: {}", e))?;

    let mut formatted = format!("### Web Search Results (SearXNG) for: '{}'\n\n", query);
    if let Some(results) = val.get("results").and_then(|r| r.as_array()) {
        for (idx, item) in results.iter().take(max_results).enumerate() {
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("No title");
            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("#");
            let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
            formatted.push_str(&format!("{}. [{}]({})\n   {}\n\n", idx + 1, title, url, content));
        }
    }

    Ok(formatted)
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    #[test]
    fn max_results_is_clamped_to_contract_range() {
        assert_eq!(clamp_max_results(None, 5), 5, "omitted falls back to config default");
        assert_eq!(clamp_max_results(None, 0), 1, "a zero config default clamps up");
        assert_eq!(clamp_max_results(None, 500), 50, "an oversized config default clamps down");
        assert_eq!(clamp_max_results(Some(10), 5), 10);
        assert_eq!(clamp_max_results(Some(50), 5), 50);
        assert_eq!(clamp_max_results(Some(51), 5), 50, "above the 1..50 contract range clamps down");
        assert_eq!(clamp_max_results(Some(0), 5), 1, "below the contract range clamps up");
        assert_eq!(clamp_max_results(Some(u64::MAX), 5), 50, "huge values clamp down");
    }

    #[test]
    fn provider_error_bodies_are_redacted() {
        let rate_limited = redact_provider_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "rate limited\nupstream detail",
        );
        assert_eq!(rate_limited, "429 Too Many Requests: rate limited");
        assert!(!rate_limited.contains("upstream detail"), "only the first line is kept");

        let json_body = r#"{"error":"quota exceeded","api_key":"sk-live-secret-token"}"#;
        let json_err = redact_provider_error(reqwest::StatusCode::UNAUTHORIZED, json_body);
        assert_eq!(json_err, "401 Unauthorized (response body withheld)");
        assert!(
            !json_err.contains("sk-live-secret-token"),
            "structured bodies must never be echoed: {}",
            json_err
        );

        let empty = redact_provider_error(reqwest::StatusCode::BAD_GATEWAY, "   ");
        assert_eq!(empty, "502 Bad Gateway (response body withheld)");

        let long_body = "x".repeat(5000);
        let long_err = redact_provider_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, &long_body);
        assert!(long_err.starts_with("500 Internal Server Error: "));
        assert!(long_err.len() <= "500 Internal Server Error: ".len() + 120);

        let html_body = "<html><body>Access denied</body></html>";
        let html_err = redact_provider_error(reqwest::StatusCode::FORBIDDEN, html_body);
        assert_eq!(html_err, "403 Forbidden (response body withheld)");
    }
}

mod urlencoding {
    pub fn encode(data: &str) -> String {
        let mut escaped = String::with_capacity(data.len() * 3);
        for b in data.bytes() {
            match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    escaped.push(b as char);
                }
                b' ' => escaped.push('+'),
                _ => escaped.push_str(&format!("%{:02X}", b)),
            }
        }
        escaped
    }
}
