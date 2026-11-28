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

use std::sync::Arc;

/// Shared HTTP hardening settings, derived from config + environment at startup.
#[derive(Clone, Default)]
pub struct HttpSecurity {
    /// When `Some`, `/api/*` requires this token.
    pub auth_token: Option<String>,
    /// When true, only loopback `Host` values are accepted.
    pub require_loopback_host: bool,
    /// Live token source, when one is installed: the middleware re-reads it on
    /// every request so a rotated token takes effect immediately.
    token_source: Option<TokenSource>,
}

/// A source of the *current* auth token.
///
/// The token is rotatable at runtime through `POST /api/config`, so an
/// `HttpSecurity` built once at router construction would keep the old (possibly
/// leaked) token valid until restart - and `/api/security/status` would report
/// a rotation that never happened. Returning `None` means "could not read the
/// live value" (the config write lock is held); the middleware then falls back
/// to the snapshot token instead of disabling auth.
pub type TokenSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

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
            token_source: None,
        }
    }

    /// Install a live token source; the middleware then re-reads the token per request.
    pub fn with_live_token(mut self, source: TokenSource) -> Self {
        self.token_source = Some(source);
        self
    }

    /// The token in force right now (live when a source is installed).
    pub fn effective_token(&self) -> Option<String> {
        match &self.token_source {
            Some(source) => source().or_else(|| self.auth_token.clone()),
            None => self.auth_token.clone(),
        }
    }

    pub fn auth_enabled(&self) -> bool {
        self.effective_token().is_some_and(|t| !t.is_empty())
    }
}

/// True when `host` denotes the local machine.
///
/// The check parses the value instead of matching a string prefix: a *name*
/// that merely starts with `127.` (`127.0.0.1.evil.com`) is exactly the
/// DNS-rebinding shape, so only an exact loopback IP (127.0.0.0/8 or `::1`) or
/// the literal name `localhost` is accepted.
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim();
    // IPv6 literals arrive bracketed (`[::1]:8080`).
    let unbracketed = match h.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => h,
    };
    // Drop a `:port` suffix. A bare (unbracketed) IPv6 literal has more than one
    // colon, so it is left alone for `parse` to judge.
    let name = match unbracketed.rsplit_once(':') {
        Some((head, tail))
            if !head.is_empty()
                && !head.contains(':')
                && tail.chars().all(|c| c.is_ascii_digit()) =>
        {
            head
        }
        _ => unbracketed,
    };
    // A trailing dot (`localhost.`) is the same name to a resolver.
    let name = name.strip_suffix('.').unwrap_or(name);

    if name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match name.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback(),
        // Any other name is refused: the rebinding attack is precisely a name
        // that *resolves* to 127.0.0.1 on the attacker's DNS.
        Err(_) => false,
    }
}

/// Reject a cross-origin state-changing request.
///
/// The app is served same-origin, so a state-changing request with an `Origin`
/// that is not this server is always an attempt to drive the UI from another
/// page: DNS rebinding (a page on `evil.com` whose A-record points at
/// `127.0.0.1`) or an ordinary CSRF. The loopback `Host` rule does NOT catch
/// this - the browser sends the correct `Host: 127.0.0.1:8290` for a rebound
/// request, so the check passes. `Sec-Fetch-Site: same-origin` is the modern
/// signal and is what current browsers send; `Origin` is the portable one.
///
/// This is deliberately independent of CORS: today the protection is an
/// accident (the `Json` extractor's content-type check forces a preflight that
/// no `CorsLayer` answers, so the browser blocks the request). Adding a
/// permissive `CorsLayer` for a legitimate reason - it is one line, and the
/// dependency is already enabled - would instantly hand any website the ability
/// to enable an unjailed shell, flip the permission policy to yolo, or overwrite
/// a run. The boundary should not depend on an extractor detail.
///
/// Returns `Some(origin)` when the request must be refused.
pub fn cross_origin_state_change(req: &Request<Body>) -> Option<String> {
    let headers = req.headers();
    // `Sec-Fetch-Site` is authoritative when present, but ONLY `same-origin` is
    // acceptable. `same-site` is NOT: for a localhost app, every other localhost
    // port is same-site (ports are not part of a site), so accepting `same-site`
    // let a page on `http://127.0.0.1:9999` flip `POST /api/security/

    if let Some(site) = headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase())
    {
        if site == "same-origin" {
            return None;
        }
        return Some(format!("Sec-Fetch-Site: {site}"));
    }
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase())
    else {
        // No Origin and no Sec-Fetch-Site: a non-browser client (curl, the harness
        // client/server, tests). The bearer-token rule still applies.
        return None;
    };
    if origin.is_empty() || origin == "null" {
        return Some("opaque Origin".to_string());
    }
    // A same-origin request's Origin is exactly the server's own origin: the
    // `Host` we are serving, on whichever scheme the bind uses.
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if host.is_empty() {
        // No Host to compare against: nothing can prove same-origin, and this is
        // a state-changing request, so refuse rather than trust.
        return Some(origin);
    }
    if origin == format!("http://{host}") || origin == format!("https://{host}") {
        return None;
    }
    Some(origin)
}

/// True when a *bind* address is loopback-only (so a rebinding guard applies).
pub fn is_loopback_bind(host: &str) -> bool {
    is_loopback_host(host)
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

/// True for the server-sent-event routes (`/api/sessions/<id>/events`,
/// `/api/server/runs/<id>/events`).
///
/// `EventSource` cannot set request headers, so these are the only paths where a
/// token may ride in the query string. Everywhere else a query token is ignored
/// (and the request is rejected) so the secret does not end up in URLs, browser
/// history, or access logs.
fn is_event_stream_path(path: &str) -> bool {
    (path.starts_with("/api/sessions/") || path.starts_with("/api/server/runs/"))
        && path.ends_with("/events")
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
    // EventSource cannot set headers; a query token is honoured ONLY on the SSE
    // routes, never on a JSON endpoint.
    if is_event_stream_path(req.uri().path()) {
        if let Some(q) = req.uri().query() {
            for pair in q.split('&') {
                if let Some(val) = pair.strip_prefix("token=") {
                    return Some(percent_decode(val));
                }
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
    let is_state_change = !matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS);

    // 1) Cross-origin state-changing requests are refused outright. The app is
    //    same-origin, so an Origin/Sec-Fetch-Site that says otherwise is a
    //    rebinding page or a CSRF trying to flip a security boundary (enable the
    //    unjailed shell, set yolo, overwrite a run). See `cross_origin_state_change`.
    if is_state_change {
        if let Some(origin) = cross_origin_state_change(&req) {
            return (
                StatusCode::FORBIDDEN,
                format!("Rejected: cross-origin request ({origin}). The Harness UI must be driven from its own origin."),
            )
                .into_response();
        }
    }

    // 2) DNS-rebinding guard: only accept loopback Host headers on loopback binds.
    if security.require_loopback_host {
        // `is_loopback_host` parses the value (brackets, port, IP), so a raw
        // `Host` of `127.0.0.1.evil.com:80` is rejected as it must be.
        let host_ok = match req.headers().get(header::HOST).and_then(|h| h.to_str().ok()) {
            Some(host) => is_loopback_host(host),
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
        // Read the token in force *now*: it is rotatable at runtime, so a
        // snapshot taken at router construction would keep the old token valid.
        let expected = security.effective_token().unwrap_or_default();
        match extract_token(&req) {
            Some(tok) if tokens_match(&tok, &expected) => {}
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

/// Content-Security-Policy for the embedded UI document.
///
/// `script-src` is `'self'` with NO 'unsafe-inline': every button the UI renders
/// is wired through a delegated listener on `data-*` attributes (see the
/// delegation block at the bottom of app.js), so an injected inline script or
/// handler attribute cannot execute. `style-src` keeps 'unsafe-inline' because
/// the markup still uses `style="..."` layout attributes; styles cannot execute,
/// so that is a cosmetic exception.
pub const UI_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
     connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; \
     form-action 'self'";

/// The harness never uses these device APIs; refuse them for every origin.
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=(), \
     usb=(), serial=(), display-capture=(), midi=()";

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
    // The harness never uses these device APIs; refuse them for any origin.
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    if !is_api {
        // The embedded UI: same-origin scripts only, no remote code, no framing.
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(UI_CONTENT_SECURITY_POLICY),
        );
        // Assets are compiled into the binary, so an update can change HTML and
        // JS at the same URLs. Never let the browser mix an old app.js with new
        // markup (or vice versa); always revalidate from the running binary.
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, max-age=0"),
        );
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_csp_forbids_inline_script_but_allows_layout_styles() {
        // No 'unsafe-inline' for scripts: an injected inline handler or script
        // tag must not execute.
        assert!(
            UI_CONTENT_SECURITY_POLICY.contains("script-src 'self';"),
            "script-src must be 'self' only: {UI_CONTENT_SECURITY_POLICY}"
        );
        assert!(
            !UI_CONTENT_SECURITY_POLICY.contains("script-src 'self' 'unsafe-inline'"),
            "'unsafe-inline' must not return to script-src: {UI_CONTENT_SECURITY_POLICY}"
        );
        // Everything else that keeps the UI same-origin and unframed.
        for directive in [
            "default-src 'self'",
            "style-src 'self' 'unsafe-inline'",
            "connect-src 'self'",
            "frame-ancestors 'none'",
            "base-uri 'none'",
            "form-action 'self'",
        ] {
            assert!(
                UI_CONTENT_SECURITY_POLICY.contains(directive),
                "CSP must keep {directive}: {UI_CONTENT_SECURITY_POLICY}"
            );
        }
        // No remote origin may ever be loaded by the UI document.
        assert!(!UI_CONTENT_SECURITY_POLICY.contains("https://"));
    }

    #[test]
    fn permissions_policy_disables_device_apis() {
        for feature in [
            "camera=()",
            "microphone=()",
            "geolocation=()",
            "payment=()",
            "usb=()",
            "display-capture=()",
        ] {
            assert!(
                PERMISSIONS_POLICY.contains(feature),
                "Permissions-Policy must disable {feature}: {PERMISSIONS_POLICY}"
            );
        }
    }

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
        // `is_loopback_host` handles the port and the brackets itself, so the
        // raw `Host` header value can be passed straight to it.
        assert_eq!(is_loopback_host("127.0.0.1:8080"), true);
        assert_eq!(is_loopback_host("localhost"), true);
        assert_eq!(is_loopback_host("[::1]:8080"), true);
        assert_eq!(is_loopback_host("evil.com:80"), false);
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

    /// The rebinding guard must judge a *parsed* host, never a string prefix:
    /// `127.0.0.1.evil.com` starts with `127.` and resolves to the attacker.
    #[test]
    fn loopback_host_rejects_prefixed_lookalikes() {
        for accepted in [
            "127.0.0.1",
            "localhost",
            "LOCALHOST",
            "localhost.",
            "::1",
            "[::1]",
            "[::1]:8080",
            "127.0.0.1:8080",
            "127.0.0.2:9000",
            "127.255.255.254",
        ] {
            assert!(
                is_loopback_host(accepted),
                "{accepted} is loopback and must be accepted"
            );
        }
        for rejected in [
            "127.0.0.1.evil.com",
            "127.evil.com",
            "0.0.0.1.evil.com",
            "evil.example.com",
            "10.0.0.5",
            "0.0.0.0",
            "8.8.8.8",
            "2130706433",
            "::2",
            "localhost.evil.com",
            "not-a-host",
        ] {
            assert!(
                !is_loopback_host(rejected),
                "{rejected} must never be treated as loopback"
            );
        }
        // Whole `/8` is loopback; the rest of 127/8's neighbours are not.
        assert!(is_loopback_host("127.0.0.1"));
        assert!(!is_loopback_host("126.0.0.1"));
        assert!(!is_loopback_host("128.0.0.1"));
    }

    /// A query-string token is only honoured on the event-stream routes, so the
    /// secret cannot end up in URLs for JSON endpoints.
    #[test]
    fn query_tokens_are_limited_to_event_stream_routes() {
        for sse in [
            "/api/sessions/ses_123/events",
            "/api/server/runs/run_9/events",
            "/api/sessions/abc/events",
        ] {
            assert!(is_event_stream_path(sse), "{sse} must accept a query token");
        }
        for json_path in [
            "/api/config",
            "/api/security/status",
            "/api/sessions",
            "/api/workspace/file",
            "/api/update/apply",
            "/api/sessions/ses_1/history",
            "/api/server/runs/run_9/stop",
            "/",
        ] {
            assert!(
                !is_event_stream_path(json_path),
                "{json_path} must NOT accept a query token"
            );
        }
    }

    /// Rotation must be immediate: a live token source replaces the snapshot
    /// token for every request, and an unreadable live value falls back to the
    /// snapshot instead of silently disabling auth.
    #[test]
    fn effective_token_follows_the_live_source() {
        let rotated = std::sync::Arc::new(std::sync::Mutex::new("old-token".to_string()));
        let holder = std::sync::Arc::clone(&rotated);
        let sec = HttpSecurity::from_config("old-token", "127.0.0.1").with_live_token(Arc::new(
            move || Some(holder.lock().unwrap().clone()),
        ));
        assert_eq!(sec.effective_token().as_deref(), Some("old-token"));

        *rotated.lock().unwrap() = "new-token".to_string();
        assert_eq!(
            sec.effective_token().as_deref(),
            Some("new-token"),
            "a rotated token must take effect without a restart"
        );
        assert!(sec.auth_enabled());

        // Clearing the token disables auth (no fallback to the snapshot).
        *rotated.lock().unwrap() = String::new();
        assert_eq!(sec.effective_token().as_deref(), Some(""));
        assert!(!sec.auth_enabled());

        // A source that cannot read (None) keeps the snapshot token in force
        // rather than dropping auth.
        let sec = HttpSecurity::from_config("snapshot-token", "127.0.0.1")
            .with_live_token(Arc::new(|| None));
        assert_eq!(sec.effective_token().as_deref(), Some("snapshot-token"));
    }
}
