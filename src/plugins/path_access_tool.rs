//! `request_path_access` — let the agent ask the user for access to a folder
//! outside the project workspace.
//!
//! Tools are sandboxed to the session's project folders. When the agent needs a
//! file elsewhere, it calls this tool: the request is surfaced through the normal
//! permission flow (`ToolPermission::ShellOrNetwork` ⇒ `Ask` in Safe mode), and
//! only an explicit approval adds the folder to `extra_access_roots`.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::contract::{OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission};
use super::{SessionToolContext, ToolPlugin};

pub struct RequestPathAccessPlugin;

/// Resolve and validate a user-supplied folder path.
fn resolve_folder(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty.".to_string());
    }
    if trimmed.contains('\0') {
        return Err("Path contains a null byte.".to_string());
    }
    let p = PathBuf::from(trimmed);
    let absolute = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(p)
    };
    Ok(absolute.canonicalize().unwrap_or(absolute))
}

#[async_trait]
impl ToolPlugin for RequestPathAccessPlugin {
    fn name(&self) -> &str {
        "request_path_access"
    }

    fn description(&self) -> &str {
        "Ask the user for permission to read or write a folder outside the current project \
         workspace. On approval the folder is added to the allowed roots for future tool calls."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: "request_path_access".to_string(),
            description: self.description().to_string(),
            category: "meta".to_string(),
            // Requires user approval: never grant filesystem access silently.
            permission_level: ToolPermission::ShellOrNetwork,
            fields: vec![
                ToolFieldSpec::required_string(
                    "path",
                    "Absolute path of the folder you need access to.",
                    "Provide an absolute path such as C:\\data\\reference.",
                )
                .non_empty(),
                ToolFieldSpec::optional_string(
                    "reason",
                    "Why this folder is needed (shown to the user).",
                ),
            ],
            expected_output: OutputContract::new(
                "json",
                "JSON confirming the granted root.",
                r#"{"status":"granted","path":"C:\\data\\reference"}"#,
            ),
            failure_modes: vec![ToolFailureMode::new(
                "not_a_directory",
                "the path exists but is not a directory",
                "Path is not a directory",
                "Request a folder, not a file.",
            )],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, _sandbox: &Path) -> Result<String, String> {
        // Without application state the grant cannot be persisted.
        let raw = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'path'.".to_string())?;
        let folder = resolve_folder(raw)?;
        Err(format!(
            "Access to '{}' requires approval; call this tool with session context so the \
             user can be prompted.",
            folder.display()
        ))
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _sandbox: &Path,
        ctx: Option<&SessionToolContext>,
    ) -> Result<String, String> {
        let state = ctx
            .and_then(|c| c.state.clone())
            .ok_or_else(|| "request_path_access requires application state.".to_string())?;
        let raw = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'path'.".to_string())?;
        let folder = resolve_folder(raw)?;

        if folder.exists() && !folder.is_dir() {
            return Err(format!("'{}' is a file, not a directory.", folder.display()));
        }

        {
            let mut conf = state.config.write().await;
            if !conf.extra_access_roots.iter().any(|r| r == &folder) {
                conf.extra_access_roots.push(folder.clone());
            }
        }
        state.persist_config().await;

        Ok(json!({
            "status": "granted",
            "path": folder.display().to_string(),
            "note": "This folder is now readable/writable by file tools for this harness."
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_null() {
        assert!(resolve_folder("   ").is_err());
        assert!(resolve_folder("a\0b").is_err());
    }

    #[test]
    fn resolves_absolute_temp_dir() {
        let dir = std::env::temp_dir();
        let resolved = resolve_folder(&dir.display().to_string()).unwrap();
        assert!(resolved.is_absolute());
    }
}
