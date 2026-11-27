use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::fs;

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::ToolPlugin;

/// Lexically normalize a path without touching the filesystem: `.` components
/// are dropped and `..` components pop the previous component (an empty result
/// means the traversal climbed above the root).
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            // Push the component itself (not `as_os_str()`) so Windows drive
            // prefixes, UNC prefixes and root separators survive normalization.
            other => out.push(other),
        }
    }
    out
}

/// Returns `true` when `candidate` is `sandbox` itself or lives underneath it.
///
/// Lexical normalization is the primary gate. When both paths exist on disk the
/// canonicalized forms are compared too, so Windows `\\?\` prefixes, junctions
/// and symlinks cannot be used to step outside the sandbox root.
fn is_within_sandbox(sandbox: &Path, candidate: &Path) -> bool {
    if !lexically_normalize(candidate).starts_with(lexically_normalize(sandbox)) {
        return false;
    }
    if let (Ok(canon_sandbox), Ok(canon_candidate)) =
        (sandbox.canonicalize(), candidate.canonicalize())
    {
        return canon_candidate.starts_with(canon_sandbox);
    }
    true
}

/// Join a workspace-relative path onto the sandbox, rejecting anything that
/// could escape it: absolute inputs, `..` traversal, null bytes, and paths that
/// resolve (lexically or via symlinks) outside the sandbox root.
pub(crate) fn resolve_sandboxed_path(sandbox: &Path, rel: &str) -> Result<PathBuf, String> {
    const ESCAPE_ERR: &str = "Path escapes the workspace sandbox";

    if rel.contains('\0') {
        return Err(format!("{}: null bytes are not allowed", ESCAPE_ERR));
    }

    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(format!("{}: absolute paths are not allowed", ESCAPE_ERR));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("{}: '..' traversal is not allowed", ESCAPE_ERR));
    }

    let joined = sandbox.join(p);
    if !is_within_sandbox(sandbox, &joined) {
        return Err(ESCAPE_ERR.to_string());
    }
    Ok(lexically_normalize(&joined))
}

fn resolve_path(sandbox: &Path, rel: &str) -> Result<PathBuf, String> {
    resolve_sandboxed_path(sandbox, rel)
}

/// All roots a tool call may use: the tool sandbox plus the session's project
/// folders and user-approved extra roots.
fn effective_roots(sandbox: &Path, ctx: Option<&super::SessionToolContext>) -> Vec<PathBuf> {
    let mut roots = vec![sandbox.to_path_buf()];
    if let Some(c) = ctx {
        for r in &c.sandbox_roots {
            if !roots.iter().any(|x| x == r) {
                roots.push(r.clone());
            }
        }
    }
    roots
}

/// Resolve a workspace path against several allowed roots (multi-folder projects).
///
/// The first root that contains the path wins. Absolute paths are accepted only
/// when they fall inside one of the roots, so a project with extra folders can
/// address files in any of them without widening the sandbox.
pub(crate) fn resolve_sandboxed_path_multi(roots: &[PathBuf], rel: &str) -> Result<PathBuf, String> {
    if roots.is_empty() {
        return Err("Path escapes the workspace sandbox".to_string());
    }
    if rel.contains('\0') {
        return Err("Path escapes the workspace sandbox: null bytes are not allowed".to_string());
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        for root in roots {
            let joined = p.to_path_buf();
            if is_within_sandbox(root, &joined) {
                return Ok(lexically_normalize(&joined));
            }
        }
        return Err("Path escapes the workspace sandbox: absolute path outside project folders".to_string());
    }
    // Relative: try each root, preferring the first that contains it.
    for root in roots {
        if let Ok(resolved) = resolve_sandboxed_path(root, rel) {
            if resolved.exists() {
                return Ok(resolved);
            }
        }
    }
    // Nothing existed yet: use the primary root (creates files there).
    resolve_sandboxed_path(&roots[0], rel)
}

// ----------------------------------------------------------------------------
// ReadFilePlugin
// ----------------------------------------------------------------------------
pub struct ReadFilePlugin;

#[async_trait]
impl ToolPlugin for ReadFilePlugin {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read contents of a file within the workspace, optionally restricted by line numbers."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "filesystem".to_string(),
            permission_level: ToolPermission::ReadOnly,
            fields: vec![
                ToolFieldSpec::required_string(
                    "path",
                    "Relative or absolute path to the file.",
                    "Provide a non-empty file path relative to the workspace or an absolute path.",
                )
                .non_empty(),
                ToolFieldSpec::optional_int(
                    "start_line",
                    "1-based starting line number (inclusive).",
                )
                .with_min(1.0)
                .with_error_hint("Provide a 1-based positive integer >= 1 for start_line."),
                ToolFieldSpec::optional_int(
                    "end_line",
                    "1-based ending line number (inclusive).",
                )
                .with_min(1.0)
                .with_error_hint("Provide a 1-based positive integer >= 1 for end_line."),
            ],
            expected_output: OutputContract::new(
                "text",
                "Full file contents, or 1-based line-numbered slice when start_line/end_line are provided.",
                "   1 | fn main() {\n   2 |     println!(\"hello\");\n   3 | }",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "file_not_found",
                    "Target file path does not exist",
                    "Failed to read file '<path>': No such file or directory",
                    "Use list_dir or grep_search to verify the exact file path before retrying.",
                ),
                ToolFailureMode::new(
                    "unreadable_or_binary_file",
                    "File lacks read permissions, is a directory, or contains non-UTF-8 bytes",
                    "Failed to read file '<path>': <io error>",
                    "Verify the target is a readable UTF-8 text file rather than a directory or binary.",
                ),
            ],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        self.execute_with_context(args, sandbox, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&super::SessionToolContext>,
    ) -> Result<String, String> {
        let path_str = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'path'".to_string())?;

        let full_path = resolve_sandboxed_path_multi(&effective_roots(sandbox, ctx), path_str)?;
        let content = fs::read_to_string(&full_path).await
            .map_err(|e| format!("Failed to read file '{}': {}", full_path.display(), e))?;

        let start_line = args.get("start_line").and_then(|v| v.as_u64()).map(|n| n as usize);
        let end_line = args.get("end_line").and_then(|v| v.as_u64()).map(|n| n as usize);

        if start_line.is_none() && end_line.is_none() {
            return Ok(content);
        }

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        let start = start_line.unwrap_or(1).saturating_sub(1);
        let end = end_line.unwrap_or(total_lines).min(total_lines);

        if start >= total_lines {
            return Ok(format!("(Empty: start_line {} exceeds total file lines {})", start + 1, total_lines));
        }

        let mut output = String::with_capacity(content.len());
        for (idx, line) in lines[start..end].iter().enumerate() {
            output.push_str(&format!("{:4} | {}\n", start + idx + 1, line));
        }
        Ok(output)
    }
}

// ----------------------------------------------------------------------------
// WriteFilePlugin
// ----------------------------------------------------------------------------
pub struct WriteFilePlugin;

#[async_trait]
impl ToolPlugin for WriteFilePlugin {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Write full contents to a file. Overwrites existing files or creates parent directories."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "filesystem".to_string(),
            permission_level: ToolPermission::WorkspaceWrite,
            fields: vec![
                ToolFieldSpec::required_string(
                    "path",
                    "Relative or absolute path to the file.",
                    "Provide a non-empty target file path.",
                )
                .non_empty(),
                ToolFieldSpec::required_string(
                    "content",
                    "Full file content to write.",
                    "Provide the string content to write to the file (may be empty).",
                ),
            ],
            expected_output: OutputContract::new(
                "text",
                "Confirmation message reporting the number of bytes written and target file path.",
                "Successfully wrote 128 bytes to 'src/example.rs'",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "directory_create_failed",
                    "Parent directory hierarchy could not be created",
                    "Failed to create directories for '<path>': <io error>",
                    "Verify the target path is valid and its parent location is writable.",
                ),
                ToolFailureMode::new(
                    "write_failed",
                    "File could not be written (permissions, lock, or path is a directory)",
                    "Failed to write file '<path>': <io error>",
                    "Check file permissions and confirm the path does not point to an existing directory.",
                ),
            ],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        self.execute_with_context(args, sandbox, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&super::SessionToolContext>,
    ) -> Result<String, String> {
        let path_str = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'path'".to_string())?;

        let content = args.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'content'".to_string())?;

        let full_path = resolve_sandboxed_path_multi(&effective_roots(sandbox, ctx), path_str)?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await
                .map_err(|e| format!("Failed to create directories for '{}': {}", full_path.display(), e))?;
        }

        fs::write(&full_path, content.as_bytes()).await
            .map_err(|e| format!("Failed to write file '{}': {}", full_path.display(), e))?;

        Ok(format!("Successfully wrote {} bytes to '{}'", content.len(), full_path.display()))
    }
}

// ----------------------------------------------------------------------------
// EditFilePlugin
// ----------------------------------------------------------------------------
pub struct EditFilePlugin;

#[async_trait]
impl ToolPlugin for EditFilePlugin {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Replace an exact substring occurrence in an existing file with replacement text."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "filesystem".to_string(),
            permission_level: ToolPermission::WorkspaceWrite,
            fields: vec![
                ToolFieldSpec::required_string(
                    "path",
                    "Relative or absolute path to the file.",
                    "Provide a non-empty path to an existing file.",
                )
                .non_empty(),
                ToolFieldSpec::required_string(
                    "target",
                    "The exact character sequence to replace.",
                    "Provide the exact non-empty substring to replace, matching whitespace and indentation.",
                )
                .non_empty(),
                ToolFieldSpec::required_string(
                    "replacement",
                    "The text to replace target with.",
                    "Provide the replacement string (may be empty to delete target).",
                ),
            ],
            expected_output: OutputContract::new(
                "text",
                "Confirmation summary reporting the number of exact occurrences replaced.",
                "Successfully replaced 1 occurrence(s) in 'src/main.rs'",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "file_not_found",
                    "Target file does not exist or cannot be read",
                    "Failed to read file '<path>': <io error>",
                    "Verify the file path using list_dir or read_file before editing.",
                ),
                ToolFailureMode::new(
                    "target_not_found",
                    "Exact target substring does not appear in the file",
                    "Target string not found in '<path>'. Ensure exact whitespace matching.",
                    "Read the file lines with read_file and copy the exact target text including indentation.",
                ),
                ToolFailureMode::new(
                    "write_failed",
                    "Updated file contents could not be written back to disk",
                    "Failed to write updated file '<path>': <io error>",
                    "Check filesystem write permissions and ensure the file is not locked.",
                ),
            ],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        self.execute_with_context(args, sandbox, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&super::SessionToolContext>,
    ) -> Result<String, String> {
        let path_str = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'path'".to_string())?;

        let target = args.get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'target'".to_string())?;

        let replacement = args.get("replacement")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'replacement'".to_string())?;

        let full_path = resolve_sandboxed_path_multi(&effective_roots(sandbox, ctx), path_str)?;
        let content = fs::read_to_string(&full_path).await
            .map_err(|e| format!("Failed to read file '{}': {}", full_path.display(), e))?;

        let exact_count = content.matches(target).count();
        let (count, new_content) = if exact_count > 0 {
            (exact_count, content.replace(target, replacement))
        } else {
            // Fall back to line-ending normalized matching (handles Windows CRLF vs LF seamlessly)
            let uses_crlf = content.contains("\r\n");
            let norm_content = content.replace("\r\n", "\n");
            let norm_target = target.replace("\r\n", "\n");
            let norm_count = norm_content.matches(&norm_target).count();
            if norm_count == 0 {
                let mut candidate_needles: Vec<&str> = norm_target
                    .lines()
                    .map(str::trim)
                    .filter(|l| l.len() >= 4)
                    .collect();
                candidate_needles.sort_by_key(|l| std::cmp::Reverse(l.len()));
                let anchor_hint = candidate_needles
                    .into_iter()
                    .find_map(|needle| {
                        norm_content
                            .lines()
                            .enumerate()
                            .find(|(_, line)| line.contains(needle))
                            .map(|(idx, line)| {
                                format!(
                                    " Closest line match at line {}: `{}`",
                                    idx + 1,
                                    line.trim().chars().take(120).collect::<String>()
                                )
                            })
                    })
                    .unwrap_or_default();
                return Err(format!(
                    "Target string not found in '{}'. Ensure exact whitespace matching.{}",
                    full_path.display(),
                    anchor_hint
                ));
            }
            let norm_repl = replacement.replace("\r\n", "\n");
            let replaced = norm_content.replace(&norm_target, &norm_repl);
            let final_text = if uses_crlf {
                replaced.replace('\n', "\r\n")
            } else {
                replaced
            };
            (norm_count, final_text)
        };

        fs::write(&full_path, new_content.as_bytes()).await
            .map_err(|e| format!("Failed to write updated file '{}': {}", full_path.display(), e))?;

        Ok(format!("Successfully replaced {} occurrence(s) in '{}'", count, full_path.display()))
    }
}

// ----------------------------------------------------------------------------
// ListDirPlugin
// ----------------------------------------------------------------------------
pub struct ListDirPlugin;

#[async_trait]
impl ToolPlugin for ListDirPlugin {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List files and directories in a directory with file sizes."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "filesystem".to_string(),
            permission_level: ToolPermission::ReadOnly,
            fields: vec![
                ToolFieldSpec::optional_string(
                    "path",
                    "Path to directory (default: current directory).",
                )
                .with_default(json!("."))
                .with_error_hint(
                    "Provide a relative or absolute directory path (omit or use '.' for workspace root).",
                ),
            ],
            expected_output: OutputContract::new(
                "text",
                "Sorted newline-delimited list of [DIR] and [FILE] entries with byte sizes, or an empty directory notice.",
                "[DIR]  src/\n[FILE] Cargo.toml (1024 bytes)",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "dir_not_found",
                    "Target directory path does not exist",
                    "Failed to read directory '<path>': No such file or directory",
                    "Call list_dir with path='.' to inspect top-level workspace directories.",
                ),
                ToolFailureMode::new(
                    "not_a_directory",
                    "Target path is a regular file or lacks read permissions",
                    "Failed to read directory '<path>': <io error>",
                    "Use read_file if the target is a file, or verify directory permissions.",
                ),
            ],
            timeout_secs: 15,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        self.execute_with_context(args, sandbox, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        sandbox: &Path,
        ctx: Option<&super::SessionToolContext>,
    ) -> Result<String, String> {
        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let full_path = resolve_sandboxed_path_multi(&effective_roots(sandbox, ctx), path_str)?;

        let mut read_dir = fs::read_dir(&full_path).await
            .map_err(|e| format!("Failed to read directory '{}': {}", full_path.display(), e))?;

        let mut entries = Vec::new();
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let meta = entry.metadata().await.ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let name = entry.file_name().to_string_lossy().to_string();

            if is_dir {
                entries.push(format!("[DIR]  {}/", name));
            } else {
                entries.push(format!("[FILE] {} ({} bytes)", name, size));
            }
        }

        entries.sort();
        if entries.is_empty() {
            Ok(format!("Directory '{}' is empty.", full_path.display()))
        } else {
            Ok(entries.join("\n"))
        }
    }
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    fn unique_sandbox(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "harness_fops_{}_{}",
            tag,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rejects_absolute_input_paths() {
        let sandbox = unique_sandbox("absolute");

        #[cfg(windows)]
        let absolute_paths = [
            r"C:\Windows\System32\drivers\etc\hosts",
            r"\\server\share\file.txt",
        ];
        #[cfg(not(windows))]
        let absolute_paths = ["/etc/passwd", "/root/.ssh/authorized_keys"];

        for bad in absolute_paths {
            let err = resolve_sandboxed_path(&sandbox, bad)
                .expect_err("absolute paths must be rejected");
            assert!(
                err.contains("Path escapes the workspace sandbox"),
                "'{}' should be rejected, got: {}",
                bad,
                err
            );
        }

        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn rejects_parent_dir_traversal() {
        let sandbox = unique_sandbox("traversal");

        for bad in [
            r"..\..\x",
            "../x",
            "sub/../../escape.txt",
            r"a\..\..\b.txt",
            "..",
        ] {
            let err = resolve_sandboxed_path(&sandbox, bad)
                .expect_err("'..' components must be rejected");
            assert!(
                err.contains("Path escapes the workspace sandbox"),
                "'{}' should be rejected, got: {}",
                bad,
                err
            );
        }

        let _ = std::fs::remove_dir_all(&sandbox);
    }

    // Windows-only: rooted-without-prefix (`\Windows\...`) and drive-relative
    // (`C:Windows`) paths are not `is_absolute()`, yet they silently replace the
    // sandbox directory when joined, so containment must reject them too.
    #[cfg(windows)]
    #[test]
    fn rejects_rooted_paths_replacing_sandbox() {
        let sandbox = unique_sandbox("rooted");

        for bad in [
            r"\Windows\System32\drivers\etc\hosts",
            r"C:Windows\System32",
        ] {
            let err = resolve_sandboxed_path(&sandbox, bad)
                .expect_err("rooted / drive-relative paths must be rejected");
            assert!(
                err.contains("Path escapes the workspace sandbox"),
                "'{}' should be rejected, got: {}",
                bad,
                err
            );
        }

        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn rejects_null_bytes() {
        let sandbox = unique_sandbox("nul");
        let err = resolve_sandboxed_path(&sandbox, "evil\0.txt")
            .expect_err("null bytes must be rejected");
        assert!(err.contains("Path escapes the workspace sandbox"), "{}", err);
        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn resolves_relative_paths_under_sandbox() {
        let sandbox = unique_sandbox("relative");
        std::fs::create_dir_all(sandbox.join("sub")).unwrap();

        let resolved = resolve_sandboxed_path(&sandbox, "sub/file.txt")
            .expect("relative path should resolve inside the sandbox");
        assert_eq!(resolved, sandbox.join("sub").join("file.txt"));
        assert!(resolved.starts_with(&sandbox));

        // '.' segments are pure noise and must not change the target.
        let dotted = resolve_sandboxed_path(&sandbox, "./sub/./nested/file.txt").unwrap();
        assert_eq!(dotted, sandbox.join("sub").join("nested").join("file.txt"));

        // The sandbox root itself remains addressable (list_dir default).
        let root = resolve_sandboxed_path(&sandbox, ".").unwrap();
        assert_eq!(root, lexically_normalize(&sandbox));

        let _ = std::fs::remove_dir_all(&sandbox);
    }
}
