//! Splitting credentials out of `HarnessConfig` so they can be encrypted at rest.
//!
//! `app_config.config_json` stores the configuration with every credential
//! blanked; the credentials themselves live in `app_secrets` encrypted with the
//! master key (see [`crate::crypto`]). This keeps the readable config free of
//! secrets while leaving all call sites working with a fully-populated config.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::config::HarnessConfig;
use crate::secret_redaction::looks_secret_key;

/// All credential material from a `HarnessConfig`, keyed by stable ids so the
/// blob survives reordering of providers/servers.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SecretsBlob {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tinyfish_api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub search_tinyfish_api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub search_tavily_api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub search_brave_api_key: String,
    /// provider id -> api_key
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_keys: BTreeMap<String, String>,
    /// server id -> api_token
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub server_tokens: BTreeMap<String, String>,
    /// mcp server id -> (env key -> value), credential-looking keys only
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp_env: BTreeMap<String, BTreeMap<String, String>>,
}

/// True for endpoints that run on the local machine (no credential to protect).
fn is_local_endpoint(api_base: &str) -> bool {
    let b = api_base.trim().to_ascii_lowercase();
    b.contains("localhost") || b.contains("127.0.0.1") || b.contains("[::1]") || b.contains("0.0.0.0")
}

impl SecretsBlob {    pub fn is_empty(&self) -> bool {
        self.api_key.is_empty()
            && self.tinyfish_api_key.is_empty()
            && self.auth_token.is_empty()
            && self.search_tinyfish_api_key.is_empty()
            && self.search_tavily_api_key.is_empty()
            && self.search_brave_api_key.is_empty()
            && self.provider_keys.is_empty()
            && self.server_tokens.is_empty()
            && self.mcp_env.is_empty()
    }
}

impl HarnessConfig {
    /// Move every credential out of `self` (leaving empty strings) and return it.
    pub fn extract_secrets(&mut self) -> SecretsBlob {
        let mut blob = SecretsBlob {
            api_key: std::mem::take(&mut self.api_key),
            tinyfish_api_key: std::mem::take(&mut self.tinyfish_api_key),
            auth_token: std::mem::take(&mut self.auth_token),
            search_tinyfish_api_key: std::mem::take(&mut self.search.tinyfish_api_key),
            search_tavily_api_key: std::mem::take(&mut self.search.tavily_api_key),
            search_brave_api_key: std::mem::take(&mut self.search.brave_api_key),
            ..Default::default()
        };
        for p in self.custom_providers.iter_mut() {
            // Local providers (Ollama/vLLM) use placeholder keys; nothing to encrypt.
            if is_local_endpoint(&p.api_base) {
                continue;
            }
            let key = std::mem::take(&mut p.api_key);
            if !key.is_empty() {
                blob.provider_keys.insert(p.id.clone(), key);
            }
        }
        for s in self.servers.iter_mut() {
            let token = std::mem::take(&mut s.api_token);
            if !token.is_empty() {
                blob.server_tokens.insert(s.id.clone(), token);
            }
        }
        for m in self.mcp_servers.iter_mut() {
            let mut env = BTreeMap::new();
            let secret_keys: Vec<String> = m
                .env
                .keys()
                .filter(|k| looks_secret_key(k))
                .cloned()
                .collect();
            for k in secret_keys {
                if let Some(v) = m.env.remove(&k) {
                    if !v.is_empty() {
                        env.insert(k, v);
                    }
                }
            }
            if !env.is_empty() {
                blob.mcp_env.insert(m.id.clone(), env);
            }
        }
        blob
    }

    /// Put credentials back into `self`.
    pub fn apply_secrets(&mut self, blob: &SecretsBlob) {
        self.api_key = blob.api_key.clone();
        self.tinyfish_api_key = blob.tinyfish_api_key.clone();
        self.auth_token = blob.auth_token.clone();
        self.search.tinyfish_api_key = blob.search_tinyfish_api_key.clone();
        self.search.tavily_api_key = blob.search_tavily_api_key.clone();
        self.search.brave_api_key = blob.search_brave_api_key.clone();
        for p in self.custom_providers.iter_mut() {
            if let Some(v) = blob.provider_keys.get(&p.id) {
                p.api_key = v.clone();
            }
        }
        for s in self.servers.iter_mut() {
            if let Some(v) = blob.server_tokens.get(&s.id) {
                s.api_token = v.clone();
            }
        }
        for m in self.mcp_servers.iter_mut() {
            if let Some(env) = blob.mcp_env.get(&m.id) {
                for (k, v) in env {
                    m.env.insert(k.clone(), v.clone());
                }
            }
        }
    }

    /// True when the config currently holds any credential.
    pub fn has_secrets(&self) -> bool {
        !self.api_key.is_empty()
            || !self.tinyfish_api_key.is_empty()
            || !self.auth_token.is_empty()
            || !self.search.tinyfish_api_key.is_empty()
            || !self.search.tavily_api_key.is_empty()
            || !self.search.brave_api_key.is_empty()
            || self
                .custom_providers
                .iter()
                .any(|p| !is_local_endpoint(&p.api_base) && !p.api_key.is_empty())
            || self.servers.iter().any(|s| !s.api_token.is_empty())
            || self
                .mcp_servers
                .iter()
                .any(|m| m.env.iter().any(|(k, v)| looks_secret_key(k) && !v.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CustomProvider, McpServerConfig, ServerNodeConfig};

    fn sample() -> HarnessConfig {
        let mut cfg = HarnessConfig::default();
        cfg.api_key = "top-secret".into();
        cfg.tinyfish_api_key = "tf".into();
        cfg.auth_token = "auth".into();
        cfg.search.tavily_api_key = "tv".into();
        cfg.custom_providers.push(CustomProvider {
            id: "acme".into(),
            name: "Acme".into(),
            api_base: "http://x/v1".into(),
            api_key: "acme-key".into(),
            default_model: "m".into(),
            is_active: false,
            enabled: true,
        });
        cfg.servers.push(ServerNodeConfig {
            id: "n1".into(),
            name: "N1".into(),
            url: "http://n".into(),
            api_token: "tok".into(),
            enabled: true,
            is_default: false,
        });
        cfg.mcp_servers.push(McpServerConfig {
            id: "m1".into(),
            name: "m1".into(),
            transport: "stdio".into(),
            command: "x".into(),
            args: vec![],
            env: [
                ("GEMINI_API_KEY".to_string(), "gem".to_string()),
                ("GOOGLE_AI_MODEL".to_string(), "gemini-2.5-flash".to_string()),
            ]
            .into_iter()
            .collect(),
            url: String::new(),
            enabled: true,
        });
        cfg
    }

    #[test]
    fn extract_then_apply_round_trips_every_credential() {
        let mut cfg = sample();
        let blob = cfg.extract_secrets();

        // Nothing secret may remain in the config.
        assert!(cfg.api_key.is_empty());
        assert!(cfg.tinyfish_api_key.is_empty());
        assert!(cfg.auth_token.is_empty());
        assert!(cfg.search.tavily_api_key.is_empty());
        // Non-local providers are blanked; local ones (Ollama placeholder) are not secrets.
        assert!(cfg
            .custom_providers
            .iter()
            .filter(|p| !is_local_endpoint(&p.api_base))
            .all(|p| p.api_key.is_empty()));
        assert!(cfg.servers.iter().all(|s| s.api_token.is_empty()));
        assert!(cfg
            .mcp_servers
            .iter()
            .all(|m| !m.env.contains_key("GEMINI_API_KEY")));
        // Non-secret env survives in the plain config.
        assert_eq!(
            cfg.mcp_servers.iter().find(|m| m.id == "m1").unwrap().env["GOOGLE_AI_MODEL"],
            "gemini-2.5-flash"
        );
        assert!(!cfg.has_secrets());

        cfg.apply_secrets(&blob);
        assert_eq!(cfg.api_key, "top-secret");
        assert_eq!(cfg.tinyfish_api_key, "tf");
        assert_eq!(cfg.auth_token, "auth");
        assert_eq!(cfg.search.tavily_api_key, "tv");
        assert_eq!(
            cfg.custom_providers.iter().find(|p| p.id == "acme").unwrap().api_key,
            "acme-key"
        );
        assert_eq!(
            cfg.servers.iter().find(|s| s.id == "n1").unwrap().api_token,
            "tok"
        );
        assert_eq!(
            cfg.mcp_servers.iter().find(|m| m.id == "m1").unwrap().env["GEMINI_API_KEY"],
            "gem"
        );
    }

    #[test]
    fn serialized_config_contains_no_secret_values() {
        let mut cfg = sample();
        cfg.extract_secrets();
        let json = serde_json::to_string(&cfg).unwrap();
        for needle in ["top-secret", "acme-key", "\"tok\"", "\"gem\"", "\"tv\"", "\"tf\""] {
            assert!(!json.contains(needle), "config JSON leaked {needle}");
        }
    }

    #[test]
    fn blob_omits_empty_fields() {
        let mut cfg = HarnessConfig::default();
        let blob = cfg.extract_secrets();
        assert!(blob.is_empty());
        let json = serde_json::to_string(&blob).unwrap();
        assert_eq!(json, "{}", "empty blob should serialize to an empty object");
    }
}
