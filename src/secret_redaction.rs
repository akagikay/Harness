//! Secret redaction for API responses.
//!
//! `GET` endpoints must never return stored credentials. But the UI re-POSTs the
//! whole config, so a redacted value that comes back on input must be recognized
//! as "unchanged" rather than overwriting the stored secret with a placeholder.
//!
//! Contract:
//! * [`SECRET_REDACTED`] is returned in place of any non-empty secret.
//! * An empty secret stays empty (no credential configured).
//! * On input, a field equal to [`SECRET_REDACTED`] restores the stored value;
//!   an explicit `""` clears it.

use crate::config::{
    CustomProvider, HarnessConfig, McpServerConfig, SearchConfig, ServerNodeConfig,
};

/// Placeholder returned instead of a secret value.
pub const SECRET_REDACTED: &str = "__harness_redacted__";

/// Heuristic: does this environment-variable name hold a credential?
pub fn looks_secret_key(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    const NEEDLES: [&str; 8] = [
        "KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH", "PRIVATE",
    ];
    NEEDLES.iter().any(|n| upper.contains(n))
}

fn redact_value(value: &mut String) {
    if !value.is_empty() {
        *value = SECRET_REDACTED.to_string();
    }
}

fn restore_value(value: &mut String, existing: &str) {
    if value == SECRET_REDACTED {
        // A placeholder with no matching stored secret (e.g. a renamed provider)
        // must never be persisted or sent upstream — clear it instead.
        *value = existing.to_string();
    }
}

impl SearchConfig {
    pub fn redact_secrets(&mut self) {
        redact_value(&mut self.tinyfish_api_key);
        redact_value(&mut self.tavily_api_key);
        redact_value(&mut self.brave_api_key);
    }

    pub fn restore_secrets(&mut self, existing: &SearchConfig) {
        restore_value(&mut self.tinyfish_api_key, &existing.tinyfish_api_key);
        restore_value(&mut self.tavily_api_key, &existing.tavily_api_key);
        restore_value(&mut self.brave_api_key, &existing.brave_api_key);
    }
}

impl McpServerConfig {
    /// Redact credential-looking `env` values; leave non-secret env untouched.
    pub fn redact_secrets(&mut self) {
        for (k, v) in self.env.iter_mut() {
            if looks_secret_key(k) {
                redact_value(v);
            }
        }
    }

    pub fn restore_secrets(&mut self, existing: &McpServerConfig) {
        for (k, v) in self.env.iter_mut() {
            if *v == SECRET_REDACTED {
                if let Some(orig) = existing.env.get(k) {
                    *v = orig.clone();
                }
            }
        }
    }
}

impl ServerNodeConfig {
    pub fn redact_secrets(&mut self) {
        redact_value(&mut self.api_token);
    }

    pub fn restore_secrets(&mut self, existing: &ServerNodeConfig) {
        restore_value(&mut self.api_token, &existing.api_token);
    }
}

impl CustomProvider {
    pub fn redact_secrets(&mut self) {
        redact_value(&mut self.api_key);
    }

    pub fn restore_secrets(&mut self, existing: &CustomProvider) {
        restore_value(&mut self.api_key, &existing.api_key);
    }
}

impl HarnessConfig {
    /// Replace every stored credential with [`SECRET_REDACTED`] for API output.
    pub fn redact_secrets(&mut self) {
        redact_value(&mut self.api_key);
        redact_value(&mut self.tinyfish_api_key);
        redact_value(&mut self.auth_token);
        self.search.redact_secrets();
        for p in self.custom_providers.iter_mut() {
            p.redact_secrets();
        }
        for m in self.mcp_servers.iter_mut() {
            m.redact_secrets();
        }
        for s in self.servers.iter_mut() {
            s.redact_secrets();
        }
    }

    /// Undo [`HarnessConfig::redact_secrets`] on an incoming payload by restoring
    /// redacted placeholders from `existing`. Entries are matched by `id` (and
    /// provider id) so reordered lists still round-trip.
    pub fn restore_secrets(&mut self, existing: &HarnessConfig) {
        restore_value(&mut self.api_key, &existing.api_key);
        restore_value(&mut self.tinyfish_api_key, &existing.tinyfish_api_key);
        restore_value(&mut self.auth_token, &existing.auth_token);
        self.search.restore_secrets(&existing.search);

        for p in self.custom_providers.iter_mut() {
            match existing.custom_providers.iter().find(|o| o.id == p.id) {
                Some(old) => p.restore_secrets(old),
                // New or renamed provider: a placeholder is not a real key.
                None => p.restore_secrets(&CustomProvider {
                    api_key: String::new(),
                    ..p.clone()
                }),
            }
        }
        for m in self.mcp_servers.iter_mut() {
            match existing.mcp_servers.iter().find(|o| o.id == m.id) {
                Some(old) => m.restore_secrets(old),
                None => {
                    for v in m.env.values_mut() {
                        if v == SECRET_REDACTED {
                            v.clear();
                        }
                    }
                }
            }
        }
        for s in self.servers.iter_mut() {
            match existing.servers.iter().find(|o| o.id == s.id) {
                Some(old) => s.restore_secrets(old),
                None => {
                    if s.api_token == SECRET_REDACTED {
                        s.api_token.clear();
                    }
                }
            }
        }
    }

    /// True when at least one credential is configured (for `has_credentials` UI hints).
    pub fn has_any_credentials(&self) -> bool {
        !self.api_key.is_empty()
            || self.custom_providers.iter().any(|p| !p.api_key.is_empty())
            || !self.tinyfish_api_key.is_empty()
            || !self.search.tinyfish_api_key.is_empty()
            || !self.search.tavily_api_key.is_empty()
            || !self.search.brave_api_key.is_empty()
            || self.servers.iter().any(|s| !s.api_token.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_then_restore_round_trips_secrets() {
        let mut existing = HarnessConfig::default();
        existing.api_key = "sk-live-1".into();
        existing.tinyfish_api_key = "tf-1".into();
        existing.search.tavily_api_key = "tv-1".into();
        existing.custom_providers.push(CustomProvider {
            id: "acme".into(),
            name: "Acme".into(),
            api_base: "http://x/v1".into(),
            api_key: "acme-key".into(),
            default_model: "m".into(),
            is_active: false,
            enabled: true,
        });
        existing.servers.push(ServerNodeConfig {
            id: "node-1".into(),
            name: "Node".into(),
            url: "http://n".into(),
            api_token: "tok-1".into(),
            enabled: true,
            is_default: false,
        });
        existing.mcp_servers.push(McpServerConfig {
            id: "srv".into(),
            name: "srv".into(),
            transport: "stdio".into(),
            command: "x".into(),
            args: vec![],
            env: [("GEMINI_API_KEY".to_string(), "gem-1".to_string())]
                .into_iter()
                .collect(),
            url: String::new(),
            enabled: true,
        });

        let mut out = existing.clone();
        out.redact_secrets();
        assert_eq!(out.api_key, SECRET_REDACTED);
        assert_eq!(out.search.tavily_api_key, SECRET_REDACTED);
        let acme = out.custom_providers.iter().find(|p| p.id == "acme").unwrap();
        assert_eq!(acme.api_key, SECRET_REDACTED);
        let node = out.servers.iter().find(|s| s.id == "node-1").unwrap();
        assert_eq!(node.api_token, SECRET_REDACTED);
        let srv = out.mcp_servers.iter().find(|m| m.id == "srv").unwrap();
        assert_eq!(srv.env["GEMINI_API_KEY"], SECRET_REDACTED);

        // Simulate the client re-POSTing the redacted snapshot unchanged.
        out.restore_secrets(&existing);
        assert_eq!(out.api_key, "sk-live-1");
        assert_eq!(out.tinyfish_api_key, "tf-1");
        assert_eq!(out.search.tavily_api_key, "tv-1");
        let acme = out.custom_providers.iter().find(|p| p.id == "acme").unwrap();
        assert_eq!(acme.api_key, "acme-key");
        let node = out.servers.iter().find(|s| s.id == "node-1").unwrap();
        assert_eq!(node.api_token, "tok-1");
        let srv = out.mcp_servers.iter().find(|m| m.id == "srv").unwrap();
        assert_eq!(srv.env["GEMINI_API_KEY"], "gem-1");
    }

    #[test]
    fn empty_and_explicit_clear_are_preserved() {
        let mut existing = HarnessConfig::default();
        existing.api_key = "sk-live-1".into();

        // Explicit clear with "" must clear, not restore.
        let mut incoming = existing.clone();
        incoming.api_key = String::new();
        incoming.restore_secrets(&existing);
        assert_eq!(incoming.api_key, "");

        // Empty stays empty (no credential configured).
        let mut none = HarnessConfig::default();
        none.redact_secrets();
        assert_eq!(none.api_key, "");
    }

    #[test]
    fn non_secret_mcp_env_is_not_redacted() {
        let mut m = McpServerConfig {
            id: "s".into(),
            name: "s".into(),
            transport: "stdio".into(),
            command: "x".into(),
            args: vec![],
            env: [
                ("GOOGLE_AI_MODEL".to_string(), "gemini-2.5-flash".to_string()),
                ("MY_TOKEN".to_string(), "secret".to_string()),
            ]
            .into_iter()
            .collect(),
            url: String::new(),
            enabled: true,
        };
        m.redact_secrets();
        assert_eq!(m.env["GOOGLE_AI_MODEL"], "gemini-2.5-flash");
        assert_eq!(m.env["MY_TOKEN"], SECRET_REDACTED);
    }
}

    #[test]
    fn placeholder_for_unknown_provider_is_cleared_not_persisted() {
        // A renamed/added provider echoes the redaction placeholder back; there is
        // no stored key for the new id, so it must be cleared rather than stored
        // and later sent upstream as a literal key.
        let existing = HarnessConfig::default();
        let mut incoming = HarnessConfig::default();
        incoming.custom_providers = vec![CustomProvider {
            id: "brand-new".into(),
            name: "Brand New".into(),
            api_base: "https://api.example.com/v1".into(),
            api_key: SECRET_REDACTED.into(),
            default_model: "m".into(),
            is_active: true,
            enabled: true,
        }];
        incoming.servers = vec![ServerNodeConfig {
            id: "new-node".into(),
            name: "New".into(),
            url: "https://n.example".into(),
            api_token: SECRET_REDACTED.into(),
            enabled: true,
            is_default: false,
        }];
        incoming.restore_secrets(&existing);
        assert_eq!(incoming.custom_providers[0].api_key, "");
        assert_eq!(incoming.servers[0].api_token, "");
    }

    #[test]
    fn no_providers_are_shipped_by_default() {
        let cfg = HarnessConfig::default();
        assert!(
            cfg.custom_providers.is_empty(),
            "the harness must ship with no configured provider"
        );
        assert_eq!(cfg.provider, "");
        assert_eq!(cfg.api_key, "");
        assert_eq!(cfg.api_base, "");
        assert!(!cfg.has_any_credentials());
    }