use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use super::contract::{OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission};
use super::ToolPlugin;

fn default_true() -> bool {
    true
}

fn default_http_method() -> String {
    "GET".to_string()
}

/// Execution backend for an agent-authored or user-authored custom tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CustomToolRunner {
    Shell {
        command_template: String,
    },
    Http {
        #[serde(default = "default_http_method")]
        method: String,
        url_template: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        body_template: Option<String>,
    },
}

/// Serializable specification for a modular custom tool persisted under `.harness/tools/<name>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolSpec {
    pub contract: ToolContract,
    pub runner: CustomToolRunner,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Deserialize)]
struct FlatCustomToolInput {
    name: String,
    description: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    permission_level: Option<ToolPermission>,
    #[serde(default)]
    fields: Vec<ToolFieldSpec>,
    #[serde(default)]
    expected_output: OutputContract,
    #[serde(default)]
    failure_modes: Vec<ToolFailureMode>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    runner: CustomToolRunner,
    #[serde(default = "default_true")]
    enabled: bool,
}

impl CustomToolSpec {
    /// Parses either a canonical `CustomToolSpec` (`{ contract, runner, enabled }`)
    /// or a flat payload (`{ name, description, ..., runner }`).
    pub fn from_json_value(value: Value) -> Result<Self, String> {
        let mut spec = if value.get("contract").is_some() {
            serde_json::from_value::<CustomToolSpec>(value)
                .map_err(|e| format!("Invalid custom tool spec: {}", e))?
        } else {
            let flat = serde_json::from_value::<FlatCustomToolInput>(value)
                .map_err(|e| format!("Invalid custom tool payload: {}", e))?;
            let category = flat
                .category
                .filter(|c| !c.trim().is_empty())
                .unwrap_or_else(|| "custom".to_string());
            let permission_level = flat.permission_level.unwrap_or_else(|| match &flat.runner {
                CustomToolRunner::Shell { .. } | CustomToolRunner::Http { .. } => {
                    ToolPermission::ShellOrNetwork
                }
            });
            let contract = ToolContract {
                name: flat.name,
                description: flat.description,
                category,
                permission_level,
                fields: flat.fields,
                expected_output: flat.expected_output,
                failure_modes: flat.failure_modes,
                timeout_secs: flat.timeout_secs.unwrap_or(30),
                is_custom: true,
            };
            CustomToolSpec {
                contract,
                runner: flat.runner,
                enabled: flat.enabled,
            }
        };

        spec.normalize();
        spec.validate_spec()?;
        Ok(spec)
    }

    pub fn normalize(&mut self) {
        self.contract.name = self.contract.name.trim().to_string();
        self.contract.description = self.contract.description.trim().to_string();
        if self.contract.category.trim().is_empty() {
            self.contract.category = "custom".to_string();
        }
        if self.contract.timeout_secs == 0 {
            self.contract.timeout_secs = 30;
        }
        self.contract.is_custom = true;
    }

    pub fn validate_spec(&self) -> Result<(), String> {
        validate_tool_name(&self.contract.name)?;

        if self.contract.description.trim().is_empty() {
            return Err("Tool contract 'description' must be non-empty.".to_string());
        }

        let mut seen_fields = HashSet::new();
        let mut seen_env_vars: HashMap<String, String> = HashMap::new();
        for field in &self.contract.fields {
            let field_name = field.name.trim();
            if !is_valid_field_name(field_name) {
                return Err(format!(
                    "Tool field name '{}' must start with an ASCII letter and may only contain ASCII letters, digits, '_' or '-'.",
                    field.name
                ));
            }
            if !seen_fields.insert(field_name.to_string()) {
                return Err(format!("Duplicate field name '{}' in contract.", field_name));
            }
            let env_name = to_env_var_name(field_name);
            if let Some(existing) = seen_env_vars.insert(env_name.clone(), field_name.to_string()) {
                return Err(format!(
                    "Field names '{}' and '{}' both normalize to environment variable '{}'; rename one of them.",
                    existing, field_name, env_name
                ));
            }
        }

        match &self.runner {
            CustomToolRunner::Shell { command_template } => {
                if command_template.trim().is_empty() {
                    return Err("Shell runner 'command_template' must be non-empty.".to_string());
                }
            }
            CustomToolRunner::Http {
                method,
                url_template,
                ..
            } => {
                if url_template.trim().is_empty() {
                    return Err("HTTP runner 'url_template' must be non-empty.".to_string());
                }
                let upper = method.trim().to_ascii_uppercase();
                if !matches!(
                    upper.as_str(),
                    "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
                ) {
                    return Err(format!(
                        "Unsupported HTTP method '{}'. Expected one of GET, POST, PUT, PATCH, DELETE, HEAD.",
                        method
                    ));
                }
            }
        }

        // The permission gate (config.rs) and the knowledge-mode gate
        // (plugins/mod.rs) trust the declared `permission_level`/`category` and
        // never inspect the runner, so a declaration must never understate what
        // the runner can actually do.
        let (runner_label, runner_action) = match &self.runner {
            CustomToolRunner::Shell { .. } => ("Shell", "execute arbitrary shell commands"),
            CustomToolRunner::Http { .. } => ("HTTP", "perform outbound network requests"),
        };
        if permission_capability_rank(self.contract.permission_level) < RUNNER_MIN_CAPABILITY_RANK {
            return Err(format!(
                "Custom tool '{}' declares permission_level '{}', which is too low: its {} runner can {}. Declare 'shell_or_network' (or 'high_risk') instead.",
                self.contract.name,
                self.contract.permission_level.as_str(),
                runner_label,
                runner_action
            ));
        }

        Ok(())
    }
}

/// Minimum permission capability tier every custom tool runner must be declared
/// with: both the Shell and the HTTP runner can escape the workspace sandbox.
const RUNNER_MIN_CAPABILITY_RANK: u8 = 2;

/// Capability tier implied by a declared `ToolPermission`. `ReadOnly`/`Meta`
/// declare no execution capability, `WorkspaceWrite` only local filesystem
/// writes, while `ShellOrNetwork`/`HighRisk` cover shell and network access.
fn permission_capability_rank(level: ToolPermission) -> u8 {
    match level {
        ToolPermission::ReadOnly | ToolPermission::Meta => 0,
        ToolPermission::WorkspaceWrite => 1,
        ToolPermission::ShellOrNetwork | ToolPermission::HighRisk => RUNNER_MIN_CAPABILITY_RANK,
    }
}

/// Contract field names must be usable as template placeholders, environment
/// variable suffixes, and LLM-facing parameter identifiers.
fn is_valid_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Validates a tool identifier for safe filesystem storage and LLM tool calling.
pub fn validate_tool_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return Err("Tool name must be between 1 and 64 characters.".to_string());
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err("Tool name cannot be empty.".to_string());
    };
    if !first.is_ascii_alphabetic() {
        return Err(format!(
            "Tool name '{}' must start with an ASCII letter.",
            trimmed
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "Tool name '{}' may only contain ASCII letters, digits, '_' or '-'.",
            trimmed
        ));
    }
    Ok(())
}

/// Runtime `ToolPlugin` wrapper backed by a `CustomToolSpec`.
pub struct DynamicCustomTool {
    pub spec: CustomToolSpec,
}

impl DynamicCustomTool {
    pub fn new(mut spec: CustomToolSpec) -> Self {
        spec.normalize();
        Self { spec }
    }
}

#[async_trait]
impl ToolPlugin for DynamicCustomTool {
    fn name(&self) -> &str {
        &self.spec.contract.name
    }

    fn description(&self) -> &str {
        &self.spec.contract.description
    }

    fn contract(&self) -> ToolContract {
        let mut c = self.spec.contract.clone();
        c.is_custom = true;
        c
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        let contract = self.contract();
        contract.validate_args(&args)?;

        let resolved_map = build_resolved_arg_map(&contract, &args);
        let timeout_secs = contract.timeout_secs.clamp(1, 600);
        let unused = declared_unused_field_names(&contract.fields, &resolved_map);
        let render_opts = RenderOptions {
            declared_unused: &unused,
            shell_quote: false,
        };

        match &self.spec.runner {
            CustomToolRunner::Shell { command_template } => {
                execute_shell_runner(
                    command_template,
                    &resolved_map,
                    &args,
                    &unused,
                    sandbox,
                    timeout_secs,
                )
                .await
            }
            CustomToolRunner::Http {
                method,
                url_template,
                headers,
                body_template,
            } => {
                execute_http_runner(
                    method,
                    url_template,
                    headers,
                    body_template.as_deref(),
                    &resolved_map,
                    render_opts,
                    timeout_secs,
                )
                .await
            }
        }
    }
}

fn build_resolved_arg_map(contract: &ToolContract, args: &Value) -> Map<String, Value> {
    let mut resolved = args.as_object().cloned().unwrap_or_default();
    for field in &contract.fields {
        let missing = resolved
            .get(&field.name)
            .map(|v| v.is_null())
            .unwrap_or(true);
        if missing {
            if let Some(ref def) = field.default {
                resolved.insert(field.name.clone(), def.clone());
            }
        }
    }
    resolved
}

fn value_to_template_string(val: &Value) -> String {
    let raw = match val {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    };
    raw.replace('\0', "")
}

/// Rendering options shared by every custom tool runner.
///
/// `render_template` stays pure: the caller resolves which declared fields are
/// unused and whether the output will be handed to a shell.
#[derive(Debug, Clone, Copy, Default)]
struct RenderOptions<'a> {
    /// Declared contract fields that received no value or default. Only these
    /// names may have their placeholders stripped; placeholders that arrived
    /// inside a substituted value are preserved verbatim.
    declared_unused: &'a [String],
    /// Shell-quote every substituted value (used only by the shell runner).
    shell_quote: bool,
}

/// Contract field names without a resolved value or default in `resolved`.
fn declared_unused_field_names<'a>(
    fields: &'a [ToolFieldSpec],
    resolved: &Map<String, Value>,
) -> Vec<String> {
    fields
        .iter()
        .filter(|field| !resolved.contains_key(field.name.trim()))
        .map(|field| field.name.trim().to_string())
        .collect()
}

/// Shell-quotes a substituted value so it cannot escape its quoting context and
/// inject commands (`; rm -rf / #`) into the rendered command line.
fn shell_quote(value: &str, is_windows: bool) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if is_windows {
        // PowerShell single-quoted strings are literal apart from `''`.
        format!("'{}'", value.replace('\'', "''"))
    } else {
        // POSIX single-quoted strings are literal apart from the `'` escape
        // sequence `'\''`.
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn render_template(template: &str, args: &Map<String, Value>, opts: RenderOptions<'_>) -> String {
    let mut rendered = template.to_string();
    for (k, v) in args {
        let placeholder = format!("{{{{{}}}}}", k);
        if rendered.contains(&placeholder) {
            let mut value = value_to_template_string(v);
            if opts.shell_quote {
                value = shell_quote(&value, cfg!(target_os = "windows"));
            }
            rendered = rendered.replace(&placeholder, &value);
        }
    }

    // Clear only declared fields that had no value/default. Placeholders that
    // came from a substituted value are left untouched, and the result no
    // longer depends on HashMap iteration order.
    let mut out = String::with_capacity(rendered.len());
    let mut rest = rendered.as_str();
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            let key = &after_start[..end];
            if !key.is_empty()
                && opts
                    .declared_unused
                    .iter()
                    .any(|declared| declared.as_str() == key)
            {
                rest = &after_start[end + 2..];
                continue;
            }
        }
        out.push_str("{{");
        rest = after_start;
    }
    out.push_str(rest);
    out
}

fn to_env_var_name(field_name: &str) -> String {
    let suffix: String = field_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("HARNESS_ARG_{}", suffix)
}

async fn execute_shell_runner(
    command_template: &str,
    resolved_args: &Map<String, Value>,
    raw_args: &Value,
    unused_fields: &[String],
    sandbox: &Path,
    timeout_secs: u64,
) -> Result<String, String> {
    // Substituted values are shell-quoted so an argument can never break out of
    // the template and inject additional commands into `sh -c` / PowerShell.
    let command_str = render_template(
        command_template,
        resolved_args,
        RenderOptions {
            declared_unused: unused_fields,
            shell_quote: true,
        },
    );

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("powershell");
        c.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(&command_str);
        c
    };

    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&command_str);
        c
    };

    cmd.current_dir(sandbox);
    cmd.env("HARNESS_ARGS_JSON", raw_args.to_string());
    for (k, v) in resolved_args {
        cmd.env(to_env_var_name(k), value_to_template_string(v));
    }

    match timeout(Duration::from_secs(timeout_secs), cmd.output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);

            if output.status.success() {
                let mut res = String::new();
                if !stdout.is_empty() {
                    res.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !res.is_empty() {
                        res.push_str("\n--- stderr ---\n");
                    }
                    res.push_str(&stderr);
                }
                if res.is_empty() {
                    res.push_str("(Custom shell tool completed with exit code 0 and no output)");
                }
                Ok(res)
            } else {
                Err(format!(
                    "Custom shell tool failed with exit code {}\nStdout:\n{}\nStderr:\n{}",
                    code, stdout, stderr
                ))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to spawn custom shell command: {}", e)),
        Err(_) => Err(format!(
            "Custom shell tool timed out after {} seconds.",
            timeout_secs
        )),
    }
}

async fn execute_http_runner(
    method: &str,
    url_template: &str,
    headers_map: &HashMap<String, String>,
    body_template: Option<&str>,
    resolved_args: &Map<String, Value>,
    render_opts: RenderOptions<'_>,
    timeout_secs: u64,
) -> Result<String, String> {
    let upper_method = method.trim().to_ascii_uppercase();
    let req_method = reqwest::Method::from_bytes(upper_method.as_bytes())
        .map_err(|e| format!("Invalid HTTP method '{}': {}", method, e))?;

    let url = render_template(url_template, resolved_args, render_opts);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let mut header_map = HeaderMap::new();
    for (k, v) in headers_map {
        let rendered_val = render_template(v, resolved_args, render_opts);
        let name = HeaderName::from_bytes(k.trim().as_bytes())
            .map_err(|e| format!("Invalid header name '{}': {}", k, e))?;
        let val = HeaderValue::from_str(&rendered_val)
            .map_err(|e| format!("Invalid header value for '{}': {}", k, e))?;
        header_map.insert(name, val);
    }

    let mut req = client.request(req_method, &url).headers(header_map.clone());
    if let Some(body_tmpl) = body_template {
        let rendered_body = render_template(body_tmpl, resolved_args, render_opts);
        if !header_map.contains_key(CONTENT_TYPE) {
            let trimmed = rendered_body.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                req = req.header(CONTENT_TYPE, "application/json");
            }
        }
        req = req.body(rendered_body);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("HTTP request to '{}' failed: {}", url, e))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read HTTP response body from '{}': {}", url, e))?;

    if status.is_success() {
        Ok(format!("HTTP {}\n{}", status.as_u16(), body))
    } else {
        Err(format!(
            "HTTP request to '{}' returned status {}: {}",
            url, status, body
        ))
    }
}

/// Directory storing project-local custom tool specs: `<sandbox>/.harness/tools`.
pub fn custom_tools_dir(sandbox: &Path) -> PathBuf {
    sandbox.join(".harness").join("tools")
}

/// Loads all valid `CustomToolSpec` JSON files from `<sandbox>/.harness/tools`.
pub fn load_custom_tools_in(sandbox: &Path) -> Vec<CustomToolSpec> {
    let dir = custom_tools_dir(sandbox);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut specs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<Value>(&raw).and_then(|v| {
            CustomToolSpec::from_json_value(v).map_err(serde::de::Error::custom)
        }) {
            Ok(spec) => specs.push(spec),
            Err(e) => {
                eprintln!(
                    "Warning: skipping invalid custom tool file '{}': {}",
                    path.display(),
                    e
                );
            }
        }
    }

    specs.sort_by(|a, b| a.contract.name.cmp(&b.contract.name));
    specs
}

/// Validates and persists a `CustomToolSpec` into `<sandbox>/.harness/tools/<name>.json`.
pub fn save_custom_tool(sandbox: &Path, spec: &CustomToolSpec) -> Result<PathBuf, String> {
    let mut normalized = spec.clone();
    normalized.normalize();
    normalized.validate_spec()?;

    let dir = custom_tools_dir(sandbox);
    std::fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Failed to create custom tools directory '{}': {}",
            dir.display(),
            e
        )
    })?;

    let path = dir.join(format!("{}.json", normalized.contract.name));
    let json_text = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("Failed to serialize custom tool spec: {}", e))?;
    std::fs::write(&path, json_text)
        .map_err(|e| format!("Failed to write '{}': {}", path.display(), e))?;

    Ok(path)
}

/// Deletes `<sandbox>/.harness/tools/<name>.json`.
pub fn delete_custom_tool(sandbox: &Path, name: &str) -> Result<(), String> {
    validate_tool_name(name)?;
    let path = custom_tools_dir(sandbox).join(format!("{}.json", name.trim()));
    if !path.exists() {
        return Err(format!(
            "Custom tool '{}' not found at '{}'",
            name,
            path.display()
        ));
    }
    std::fs::remove_file(&path)
        .map_err(|e| format!("Failed to delete '{}': {}", path.display(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_contract(name: &str, permission_level: ToolPermission) -> ToolContract {
        ToolContract {
            name: name.to_string(),
            description: "Test custom tool".to_string(),
            category: "custom".to_string(),
            permission_level,
            fields: vec![ToolFieldSpec::required_string(
                "target",
                "Target label",
                "Provide a non-empty target string.",
            )],
            expected_output: OutputContract::new("text", "Echo output", "hello"),
            failure_modes: vec![ToolFailureMode::new(
                "missing_target",
                "target missing",
                "Missing required field 'target'",
                "Pass target string.",
            )],
            timeout_secs: 15,
            is_custom: true,
        }
    }

    fn shell_spec(name: &str, permission_level: ToolPermission) -> CustomToolSpec {
        CustomToolSpec {
            contract: test_contract(name, permission_level),
            runner: CustomToolRunner::Shell {
                command_template: "echo hello_{{target}}".to_string(),
            },
            enabled: true,
        }
    }

    fn http_spec(name: &str, permission_level: ToolPermission, fields: Vec<ToolFieldSpec>) -> CustomToolSpec {
        let mut contract = test_contract(name, permission_level);
        contract.fields = fields;
        CustomToolSpec {
            contract,
            runner: CustomToolRunner::Http {
                method: "GET".to_string(),
                url_template: "https://example.com/{{target}}".to_string(),
                headers: HashMap::new(),
                body_template: None,
            },
            enabled: true,
        }
    }

    #[test]
    fn shell_runner_requires_shell_or_network_or_higher() {
        for too_low in [
            ToolPermission::ReadOnly,
            ToolPermission::Meta,
            ToolPermission::WorkspaceWrite,
        ] {
            let mut spec = shell_spec("shell_tier", too_low);
            spec.normalize();
            let err = spec
                .validate_spec()
                .expect_err("a Shell runner declared below shell_or_network must be rejected");
            assert!(
                err.contains("Shell runner") && err.contains(too_low.as_str()),
                "error must name the runner and the declared permission level, got: {}",
                err
            );
        }

        for ok in [ToolPermission::ShellOrNetwork, ToolPermission::HighRisk] {
            let mut spec = shell_spec("shell_tier", ok);
            spec.normalize();
            assert!(
                spec.validate_spec().is_ok(),
                "Shell runner declared {:?} must validate",
                ok
            );
        }
    }

    #[test]
    fn http_runner_requires_shell_or_network_or_higher() {
        for too_low in [
            ToolPermission::ReadOnly,
            ToolPermission::Meta,
            ToolPermission::WorkspaceWrite,
        ] {
            let mut spec = http_spec("http_tier", too_low, vec![]);
            spec.normalize();
            let err = spec
                .validate_spec()
                .expect_err("an HTTP runner declared below shell_or_network must be rejected");
            assert!(
                err.contains("HTTP runner") && err.contains(too_low.as_str()),
                "error must name the runner and the declared permission level, got: {}",
                err
            );
        }

        for ok in [ToolPermission::ShellOrNetwork, ToolPermission::HighRisk] {
            let mut spec = http_spec("http_tier", ok, vec![]);
            spec.normalize();
            assert!(
                spec.validate_spec().is_ok(),
                "HTTP runner declared {:?} must validate",
                ok
            );
        }
    }

    #[test]
    fn undeclared_flat_permission_defaults_to_runner_capability() {
        let spec = CustomToolSpec::from_json_value(json!({
            "name": "flat_default",
            "description": "Flat payload default",
            "runner": { "type": "shell", "command_template": "echo hi" }
        }))
        .expect("flat payload with no permission_level must still validate");
        assert_eq!(
            spec.contract.permission_level,
            ToolPermission::ShellOrNetwork
        );
    }

    #[test]
    fn field_names_must_be_identifiers_and_map_to_unique_env_vars() {
        for name in ["1field", "my field", "fie.ld", "", "  ", "_hidden"] {
            let mut spec = shell_spec("fields", ToolPermission::ShellOrNetwork);
            spec.contract.fields = vec![ToolFieldSpec::required_string(name, "d", "hint")];
            let err = spec
                .validate_spec()
                .expect_err("invalid field name must be rejected");
            assert!(
                err.contains("Tool field name"),
                "unexpected error for '{}': {}",
                name,
                err
            );
        }

        for name in ["field", "api-key", "api_key", "fieldName2"] {
            let mut spec = shell_spec("fields_ok", ToolPermission::ShellOrNetwork);
            spec.contract.fields = vec![ToolFieldSpec::required_string(name, "d", "hint")];
            assert!(
                spec.validate_spec().is_ok(),
                "field name '{}' must be accepted",
                name
            );
        }

        // Distinct field names that collapse onto one HARNESS_ARG_* env var.
        let mut colliding = shell_spec("fields_collide", ToolPermission::ShellOrNetwork);
        colliding.contract.fields = vec![
            ToolFieldSpec::required_string("api-key", "d", "hint"),
            ToolFieldSpec::required_string("api_key", "d", "hint"),
        ];
        let err = colliding
            .validate_spec()
            .expect_err("colliding env var names must be rejected");
        assert!(
            err.contains("HARNESS_ARG_API_KEY"),
            "error must name the colliding env var, got: {}",
            err
        );
    }

    #[test]
    fn declared_unused_field_names_only_reports_missing_declarations() {
        let fields = vec![
            ToolFieldSpec::required_string("a", "d", "hint"),
            ToolFieldSpec::optional_string("b", "d"),
            ToolFieldSpec::optional_string("c", "d"),
        ];
        let resolved = json!({ "a": "1", "b": null })
            .as_object()
            .cloned()
            .unwrap();

        let unused = declared_unused_field_names(&fields, &resolved);
        assert_eq!(unused, vec!["c".to_string()]);
        assert_eq!(to_env_var_name("api-key"), "HARNESS_ARG_API_KEY");
    }

    #[test]
    fn shell_quote_neutralizes_metacharacters_and_quotes() {
        // POSIX: `;` and `#` stay inside one single-quoted word.
        assert_eq!(shell_quote("; rm -rf / #", false), "'; rm -rf / #'");
        assert_eq!(shell_quote("it's a trap", false), "'it'\\''s a trap'");
        // PowerShell: a single quote is escaped by doubling it.
        assert_eq!(shell_quote("'; Remove-Item -Recurse #", true), "'''; Remove-Item -Recurse #'");
        assert_eq!(shell_quote("plain", false), "'plain'");
        assert_eq!(shell_quote("", false), "''");
    }

    #[test]
    fn render_template_quotes_values_for_shell_runner() {
        let args = json!({
            "target": "; rm -rf / #",
            "note": "it's fine"
        })
        .as_object()
        .cloned()
        .unwrap();
        let opts = RenderOptions {
            declared_unused: &[],
            shell_quote: true,
        };

        // The metacharacters stay inside the quotes of the substituted word.
        let rendered = render_template("echo {{target}} # {{missing}}", &args, opts);
        assert_eq!(rendered, "echo '; rm -rf / #' # {{missing}}");

        // A value that itself contains a placeholder must survive untouched.
        let nested_args = json!({ "target": "deploy {{env}}" })
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(
            render_template("echo {{target}}", &nested_args, opts),
            "echo 'deploy {{env}}'"
        );

        // The HTTP runner keeps raw values (no shell quoting on that path).
        let raw_opts = RenderOptions {
            declared_unused: &[],
            shell_quote: false,
        };
        assert_eq!(
            render_template("/v1/{{target}}", &args, raw_opts),
            "/v1/; rm -rf / #"
        );
    }

    #[test]
    fn render_template_only_strips_declared_unused_placeholders() {
        let declared_unused = vec!["missing".to_string()];
        let opts = RenderOptions {
            declared_unused: &declared_unused,
            shell_quote: false,
        };

        // The declared-unused field is stripped, but placeholders arriving from a
        // substituted value or from an undeclared key must be preserved.
        let value_args = json!({ "known": "deploy {{env}}" })
            .as_object()
            .cloned()
            .unwrap();
        let rendered = render_template(
            "run {{missing}} | {{known}} | {{env}} | {{unknown}}",
            &value_args,
            opts,
        );
        assert_eq!(rendered, "run  | deploy {{env}} | {{env}} | {{unknown}}");

        // Trailing declared-unused placeholders collapse without disturbing the rest.
        let extra_args = json!({ "known2": "x" }).as_object().cloned().unwrap();
        let shuffled = render_template("{{known2}}{{missing}}", &extra_args, opts);
        assert_eq!(shuffled, "x");
    }
}
