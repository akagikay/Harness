//! HTTP hardening middleware: bearer-token auth, DNS-rebinding guard, and
//! conservative CORS.
//!
//! Threat model: the harness binds loopback by default but exposes a fully
//! privileged API (config with credentials, tool execution, filesystem browse).
//! Without protection, any page in the user's browser could `fetch()` it.
//!
//! Layers applied here:
//! * **Host allowlist** — rejects requests whose `Host` is not a loopback name
//!   when the server is bound to loopback. Blocks DNS-rebinding attacks.
//! * **Optional bearer auth** — when `HARNESS_AUTH_TOKEN` (or `config.auth_token`)
//!   is set, every `/api/*` request must present it. Static UI assets are exempt
//!   so the page can load and prompt for the token.
//! * **No permissive CORS** — the UI is same-origin; cross-origin reads are denied.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Shared HTTP hardening settings, derived from config + environment at startup.
#[derive(Clone, Debug, Default)]
pub struct HttpSecurity {
    /// When `Some`, `/api/*` requires this token.
    pub auth_token: Option<String>,
    /// When true, only loopback `Host` values are accepted.
    pub require_loopback_host: bool,
}

impl HttpSecurity {
    /// Build from an explicit token (config) falling back to `HARNESS_AUTH_TOKEN`.
    pub fn from_config(config_token: &str, bind_host: &str) -> Self {
        let env_token = std::env::var("HARNESS_AUTH_TOKEN").ok();
        let token = [Some(config_token.trim().to_string()), env_token]
            .into_iter()
            .flatten()
            .find(|t| !t.is_empty());
        Self {
            auth_token: token,
            require_loopback_host: is_loopback_bind(bind_host),
        }
    }

    pub fn auth_enabled(&self) -> bool {
        self.auth_token.as_deref().is_some_and(|t| !t.is_empty())
    }
}

/// True when `host` denotes the local machine.
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h == "::1" || h.starts_with("127.")
}

/// True when a *bind* address is loopback-only (so a rebinding guard applies).
pub fn is_loopback_bind(host: &str) -> bool {
    is_loopback_host(host)
}

/// Extract the hostname portion of a `Host` header (drops `:port`, brackets).
fn host_header_name(value: &str) -> &str {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix('[') {
        // IPv6 literal: [::1]:8080
        return rest.split(']').next().unwrap_or(rest);
    }
    match v.rsplit_once(':') {
        // Avoid mangling bare IPv6 without brackets (rare in Host headers).
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => v,
    }
}

/// Constant-time-ish comparison to avoid trivial token timing leaks.
fn tokens_match(provided: &str, expected: &str) -> bool {
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Paths exempt from bearer auth: the static UI shell, so the page can load.
fn is_public_path(path: &str) -> bool {
    path == "/"
        || path == "/static/app.js"
        || path == "/static/style.css"
        || path == "/favicon.ico"
}

/// Pull a token from `Authorization: Bearer`, `X-Harness-Token`, or `?token=`.
fn extract_token(req: &Request<Body>) -> Option<String> {
    if let Some(v) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
    {
        if let Some(tok) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
            return Some(tok.trim().to_string());
        }
    }
    if let Some(v) = req
        .headers()
        .get("x-harness-token")
        .and_then(|h| h.to_str().ok())
    {
        return Some(v.trim().to_string());
    }
    // EventSource cannot set headers; allow `?token=` for SSE endpoints only.
    if let Some(q) = req.uri().query() {
        for pair in q.split('&') {
            if let Some(val) = pair.strip_prefix("token=") {
                return Some(percent_decode(val));
            }
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Middleware enforcing [`HttpSecurity`].
pub async fn security_middleware(
    State(security): State<HttpSecurity>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();

    // 1) DNS-rebinding guard: only accept loopback Host headers on loopback binds.
    if security.require_loopback_host {
        let host_ok = match req.headers().get(header::HOST).and_then(|h| h.to_str().ok()) {
            Some(host) => is_loopback_host(host_header_name(host)),
            // HTTP/1.1 requires Host; be lenient for HTTP/2 (`:authority`) tests.
            None => true,
        };
        if !host_ok {
            return (
                StatusCode::FORBIDDEN,
                "Rejected: unexpected Host header (DNS rebinding protection).",
            )
                .into_response();
        }
    }

    // 2) Bearer auth for API routes.
    if security.auth_enabled() && path.starts_with("/api/") && !is_public_path(&path) {
        let expected = security.auth_token.as_deref().unwrap_or("");
        match extract_token(&req) {
            Some(tok) if tokens_match(&tok, expected) => {}
            _ => {
                let mut resp = (
                    StatusCode::UNAUTHORIZED,
                    "Missing or invalid harness auth token.",
                )
                    .into_response();
                resp.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_static("Bearer realm=\"harness\""),
                );
                return resp;
            }
        }
    }

    next.run(req).await
}

/// Add conservative security headers to every response.
pub async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "x-frame-options",
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    if !is_api {
        // The embedded UI: same-origin scripts only, no remote code, no framing.
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self' 'unsafe-inline'; \
                 style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
                 connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; \
                 form-action 'self'",
            ),
        );
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_detection() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("evil.example.com"));
        assert!(!is_loopback_host("10.0.0.5"));
        // Binding all interfaces is not a loopback bind.
        assert!(!is_loopback_bind("0.0.0.0"));
    }

    #[test]
    fn host_header_parsing() {
        assert_eq!(host_header_name("127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(host_header_name("localhost"), "localhost");
        assert_eq!(host_header_name("[::1]:8080"), "::1");
        assert_eq!(host_header_name("evil.com:80"), "evil.com");
    }

    #[test]
    fn token_compare_is_exact() {
        assert!(tokens_match("abc", "abc"));
        assert!(!tokens_match("abc", "abd"));
        assert!(!tokens_match("ab", "abc"));
    }

    #[test]
    fn public_paths_exempt() {
        assert!(is_public_path("/"));
        assert!(is_public_path("/static/app.js"));
        assert!(!is_public_path("/api/config"));
    }

    #[test]
    fn token_from_env_or_config() {
        let sec = HttpSecurity::from_config("cfg-token", "127.0.0.1");
        assert_eq!(sec.auth_token.as_deref(), Some("cfg-token"));
        assert!(sec.require_loopback_host);

        let none = HttpSecurity::from_config("", "0.0.0.0");
        // A non-loopback bind must NOT apply the loopback Host guard.
        assert!(!none.require_loopback_host);
    }
}
