//! `create_skill` meta-tool: lets the agent author a new workspace skill.
//!
//! Skills are plain Markdown files discovered under `<sandbox>/.harness/skills/`
//! (see `crate::config::HarnessConfig::discover_skills_in`). This tool writes a
//! flat `<name>.md` file with optional `name`/`description` frontmatter so the
//! skill is picked up on the next turn and can be invoked as `/<name>`.
//!
//! A common use is "condense this session into a skill": the model distills the
//! workflow it just performed into reusable instructions and persists it.

use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::contract::{OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission};
use super::file_ops::resolve_sandboxed_path;
use super::ToolPlugin;

pub struct CreateSkillPlugin;

/// Slugify a skill name into a safe file stem. Mirrors `slugify_skill_name`
/// used by the `/api/skills` handler so tool- and UI-created skills match.
fn slugify(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut slug = String::with_capacity(trimmed.len());
    for ch in trimmed.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            slug.push(ch);
        } else {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() || slug == "." || slug == ".." {
        "skill".to_string()
    } else {
        slug
    }
}

#[async_trait]
impl ToolPlugin for CreateSkillPlugin {
    fn name(&self) -> &str {
        "create_skill"
    }

    fn description(&self) -> &str {
        "Create a reusable workspace skill as a Markdown file in .harness/skills/. \
         The skill is discovered on the next turn and can be invoked with /<name>. \
         Use this to condense a workflow from the current session into a repeatable skill."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: "create_skill".to_string(),
            description: self.description().to_string(),
            category: "meta".to_string(),
            permission_level: ToolPermission::Meta,
            fields: vec![
                ToolFieldSpec::required_string(
                    "name",
                    "Skill name (slug; e.g. 'review-changes'). Becomes the /<name> slash command.",
                    "Use a short ASCII name like 'review-changes' or 'orchestrate'.",
                )
                .non_empty(),
                ToolFieldSpec::required_string(
                    "description",
                    "One-line description shown in the skill index so the model knows when to use it.",
                    "Provide a concise sentence describing the skill's purpose.",
                )
                .non_empty(),
                ToolFieldSpec::required_string(
                    "content",
                    "Full Markdown instructions for the skill (the body after frontmatter). \
                     Distill the workflow, steps, constraints, and preferred tools.",
                    "Write complete, reusable instructions, not a summary of this conversation.",
                )
                .non_empty(),
            ],
            expected_output: OutputContract::new(
                "json",
                "JSON object confirming the skill name and written workspace-relative path.",
                r#"{"status":"created","name":"review-changes","path":".harness/skills/review-changes.md"}"#,
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_field",
                    "name, description, or content is empty",
                    "Missing required parameter",
                    "Provide non-empty name, description, and content.",
                ),
                ToolFailureMode::new(
                    "write_failed",
                    "Could not create the skills directory or write the file",
                    "Failed to write skill file",
                    "Check workspace permissions; ensure the name is not reserved.",
                ),
            ],
            timeout_secs: 30,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Missing required parameter 'name'.".to_string())?;
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Missing required parameter 'description'.".to_string())?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.replace("\r\n", "\n"))
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "Missing required parameter 'content'.".to_string())?;

        let slug = slugify(name);
        let skills_dir: PathBuf = sandbox.join(".harness").join("skills");
        tokio::fs::create_dir_all(&skills_dir)
            .await
            .map_err(|e| format!("Failed to create .harness/skills directory: {e}"))?;

        let rel = format!(".harness/skills/{slug}.md");
        let target = resolve_sandboxed_path(sandbox, &rel)?;

        // Build the file. Only prepend frontmatter if the body doesn't already
        // carry its own `---` block, so callers can supply full SKILL.md content.
        let trimmed_content = content.trim_end();
        let body = if trimmed_content.starts_with("---\n") || trimmed_content == "---" {
            trimmed_content.to_string()
        } else {
            format!(
                "---\nname: {name}\ndescription: {description}\n---\n\n{trimmed_content}\n",
                name = name,
                description = description,
                trimmed_content = trimmed_content
            )
        };

        tokio::fs::write(&target, body)
            .await
            .map_err(|e| format!("Failed to write skill file {rel}: {e}"))?;

        Ok(serde_json::json!({
            "status": "created",
            "name": slug,
            "path": rel,
            "invoke": format!("/{slug}"),
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes() {
        assert_eq!(slugify("Review Changes!"), "review-changes");
        assert_eq!(slugify("  condense-session  "), "condense-session");
        assert_eq!(slugify(".."), "skill");
        assert_eq!(slugify(""), "skill");
    }
}


#[cfg(test)]
mod exec_tests {
    use super::*;

    #[tokio::test]
    async fn create_skill_writes_sandboxed_markdown() {
        let root = std::env::temp_dir().join(format!("harness-create-skill-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let plugin = CreateSkillPlugin;
        let out = plugin
            .execute(
                serde_json::json!({
                    "name": "Condense Session",
                    "description": "Condense a session into a reusable workflow.",
                    "content": "## Steps\n1. Review history.\n2. Save skill."
                }),
                &root,
            )
            .await
            .expect("create_skill should succeed");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "created");
        assert_eq!(v["name"], "condense-session");
        assert_eq!(v["path"], ".harness/skills/condense-session.md");
        assert_eq!(v["invoke"], "/condense-session");
        let written = std::fs::read_to_string(root.join(".harness/skills/condense-session.md")).unwrap();
        assert!(written.starts_with("---\nname: Condense Session\n"));
        assert!(written.contains("description: Condense a session into a reusable workflow."));
        assert!(written.contains("## Steps"));

        // The written skill is then discovered on the next turn.
        let cfg = crate::config::HarnessConfig::default();
        let found = cfg.discover_skills_in(&root);
        assert!(found.iter().any(|s| s.path.ends_with("condense-session.md") && !s.path.starts_with("builtin://")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn create_skill_rejects_missing_fields() {
        let root = std::env::temp_dir().join(format!("harness-create-skill-bad-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let plugin = CreateSkillPlugin;
        assert!(plugin.execute(serde_json::json!({"name":"x","description":"y"}), &root).await.is_err());
        assert!(plugin.execute(serde_json::json!({"name":"x","content":"c"}), &root).await.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
