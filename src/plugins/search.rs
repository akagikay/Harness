use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use tokio::fs;

use super::contract::{
    OutputContract, ToolContract, ToolFailureMode, ToolFieldSpec, ToolPermission,
};
use super::file_ops::resolve_sandboxed_path;
use super::ToolPlugin;

fn glob_match_pattern(pattern: &str, candidate: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let s: Vec<char> = candidate.to_ascii_lowercase().chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_si = 0usize;

    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            star_si = si;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn matches_include_filters(rel_display: &str, file_name: &str, filters: &[&str]) -> bool {
    if filters.is_empty() {
        return true;
    }
    let norm_rel = rel_display.replace('\\', "/");
    filters.iter().any(|raw| {
        let f = raw.trim();
        if f.is_empty() {
            return false;
        }
        if f.contains('*') || f.contains('?') {
            glob_match_pattern(f, file_name) || glob_match_pattern(f, &norm_rel)
        } else {
            file_name.to_ascii_lowercase().contains(&f.to_ascii_lowercase())
                || norm_rel.to_ascii_lowercase().contains(&f.to_ascii_lowercase())
        }
    })
}

pub struct GrepSearchPlugin;

#[async_trait]
impl ToolPlugin for GrepSearchPlugin {
    fn name(&self) -> &'static str {
        "grep_search"
    }

    fn description(&self) -> &'static str {
        "Search for a text pattern recursively in files, returning matching file paths and line numbers. Supports case-insensitive search and include glob filters (e.g. '*.rs,*.ts')."
    }

    fn contract(&self) -> ToolContract {
        ToolContract {
            name: self.name().to_string(),
            description: self.description().to_string(),
            category: "search".to_string(),
            permission_level: ToolPermission::ReadOnly,
            fields: vec![
                ToolFieldSpec::required_string(
                    "pattern",
                    "Text substring to search for.",
                    "Provide a non-empty search pattern string.",
                )
                .non_empty(),
                ToolFieldSpec::optional_string("path", "Starting directory or file (default: '.').")
                    .with_default(json!("."))
                    .with_error_hint("Provide a relative path inside the workspace (or '.' for root)."),
                ToolFieldSpec::optional_string(
                    "include",
                    "Optional comma-separated filename/glob filter (e.g. '*.rs,*.toml' or 'src/engine').",
                )
                .with_error_hint("Provide comma-separated glob or filename filters like '*.rs'."),
                ToolFieldSpec::optional_bool(
                    "case_insensitive",
                    "Perform case-insensitive substring matching (default: false).",
                )
                .with_default(json!(false))
                .with_error_hint("Pass true or false for case_insensitive."),
                ToolFieldSpec::optional_int(
                    "max_results",
                    "Maximum number of matched lines to return (default: 50).",
                )
                .with_min(1.0)
                .with_default(json!(50))
                .with_error_hint("Provide a positive integer >= 1 for max_results."),
            ],
            expected_output: OutputContract::new(
                "text",
                "Newline-separated match lines formatted as '<relative_path>:<line_number>: <trimmed_line>', or a not-found message.",
                "src/main.rs:15: fn main() {",
            ),
            failure_modes: vec![
                ToolFailureMode::new(
                    "missing_pattern",
                    "pattern is omitted, null, or empty",
                    "Missing required parameter 'pattern'",
                    "Provide a non-empty string for 'pattern'.",
                ),
                ToolFailureMode::new(
                    "invalid_path_or_unreadable",
                    "Starting path does not exist or contains unreadable files",
                    "Pattern '<pattern>' not found in scanned files.",
                    "Verify the starting 'path' with list_dir and retry with a valid directory.",
                ),
            ],
            timeout_secs: 30,
            is_custom: false,
        }
    }

    async fn execute(&self, args: Value, sandbox: &Path) -> Result<String, String> {
        let pattern = args.get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required parameter 'pattern'".to_string())?;

        let rel_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let include_raw = args.get("include").and_then(|v| v.as_str()).unwrap_or("");
        let include_filters: Vec<&str> = include_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_results = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(50).clamp(1, 500) as usize;
        let pattern_lower = if case_insensitive {
            pattern.to_ascii_lowercase()
        } else {
            String::new()
        };

        // Route the starting path through the hardened sandbox containment check:
        // absolute paths, '..' traversal and symlink escapes are rejected so a
        // caller cannot disclose host files outside the workspace.
        let start_dir = resolve_sandboxed_path(sandbox, rel_path)?;
        let mut results = Vec::new();
        let mut stack = vec![start_dir.clone()];
        let mut files_scanned = 0usize;
        let allow_explicit_hidden = rel_path
            .split(['/', '\\'])
            .any(|seg| seg.starts_with('.') && seg != "." && seg != "..");

        if let Ok(meta) = fs::metadata(&start_dir).await {
            if meta.is_file() {
                stack.clear();
                files_scanned = 1;
                if let Ok(content) = fs::read_to_string(&start_dir).await {
                    let rel_display = start_dir.strip_prefix(sandbox).unwrap_or(&start_dir).display().to_string();
                    for (line_no, line) in content.lines().enumerate() {
                        let matched = if case_insensitive {
                            line.to_ascii_lowercase().contains(&pattern_lower)
                        } else {
                            line.contains(pattern)
                        };
                        if matched {
                            results.push(format!("{}:{}: {}", rel_display, line_no + 1, line.trim()));
                            if results.len() >= max_results {
                                break;
                            }
                        }
                    }
                }
            }
        }

        while let Some(current) = stack.pop() {
            if results.len() >= max_results || files_scanned >= 2000 {
                break;
            }

            let mut dir_reader = match fs::read_dir(&current).await {
                Ok(r) => r,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = dir_reader.next_entry().await {
                let path = entry.path();
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                // Skip noisy/heavy directories
                if (!allow_explicit_hidden && file_name.starts_with('.'))
                    || file_name == "target"
                    || file_name == "node_modules"
                    || file_name == "dist"
                {
                    continue;
                }

                if let Ok(meta) = entry.metadata().await {
                    if meta.is_dir() {
                        stack.push(path);
                    } else if meta.is_file() && meta.len() < 1_000_000 {
                        let rel_display = path
                            .strip_prefix(sandbox)
                            .unwrap_or(&path)
                            .display()
                            .to_string();
                        if !matches_include_filters(&rel_display, file_name, &include_filters) {
                            continue;
                        }
                        files_scanned += 1;
                        if let Ok(content) = fs::read_to_string(&path).await {
                            for (line_no, line) in content.lines().enumerate() {
                                let matched = if case_insensitive {
                                    line.to_ascii_lowercase().contains(&pattern_lower)
                                } else {
                                    line.contains(pattern)
                                };
                                if matched {
                                    results.push(format!("{}:{}: {}", rel_display, line_no + 1, line.trim()));
                                    if results.len() >= max_results {
                                        break;
                                    }
                                }
                            }
                        }
                        if results.len() >= max_results || files_scanned >= 2000 {
                            break;
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            Ok(format!(
                "Pattern '{}' not found across {} scanned file(s).",
                pattern, files_scanned
            ))
        } else {
            let truncated_note = if results.len() >= max_results {
                format!(" (showing first {}, increase max_results if needed)", max_results)
            } else {
                String::new()
            };
            Ok(format!(
                "{}\n\n[grep_search: {} match(es) across {} scanned file(s){}]",
                results.join("\n"),
                results.len(),
                files_scanned,
                truncated_note
            ))
        }
    }
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    #[tokio::test]
    async fn grep_search_rejects_paths_escaping_sandbox() {
        let sandbox = std::env::temp_dir().join(format!(
            "harness_grep_sandbox_{}_{}",
            std::process::id(),
            line!()
        ));
        tokio::fs::create_dir_all(sandbox.join("sub")).await.unwrap();
        tokio::fs::write(sandbox.join("inside.txt"), "needle inside workspace\n")
            .await
            .unwrap();

        let plugin = GrepSearchPlugin;
        let mut bad_paths: Vec<&str> = vec!["..", "../outside", r"..\..\x", "sub/../../outside"];

        #[cfg(windows)]
        bad_paths.extend([r"C:\Windows\System32\drivers\etc\hosts"]);
        #[cfg(not(windows))]
        bad_paths.extend(["/etc/passwd"]);

        for bad in bad_paths {
            let err = plugin
                .execute(json!({ "pattern": "needle", "path": bad }), &sandbox)
                .await
                .expect_err("path must not escape the workspace sandbox");
            assert!(
                err.contains("Path escapes the workspace sandbox"),
                "'{}' should be rejected, got: {}",
                bad,
                err
            );
        }

        let _ = tokio::fs::remove_dir_all(&sandbox).await;
    }

    #[tokio::test]
    async fn grep_search_still_scans_workspace_relative_paths() {
        let sandbox = std::env::temp_dir().join(format!(
            "harness_grep_ok_{}_{}",
            std::process::id(),
            line!()
        ));
        tokio::fs::create_dir_all(sandbox.join("sub")).await.unwrap();
        tokio::fs::write(sandbox.join("sub").join("notes.txt"), "needle in sub\n")
            .await
            .unwrap();

        let plugin = GrepSearchPlugin;

        let default_dir = plugin
            .execute(json!({ "pattern": "needle" }), &sandbox)
            .await
            .expect("default '.' start path should scan the workspace");
        assert!(default_dir.contains("notes.txt:1:"), "unexpected output: {}", default_dir);

        let nested = plugin
            .execute(json!({ "pattern": "needle", "path": "sub" }), &sandbox)
            .await
            .expect("relative subdirectory should scan");
        // Paths are reported relative to the sandbox; normalize separators so
        // the assertion holds on Windows (`\`) and Unix (`/`) alike.
        let nested_norm = nested.replace('\\', "/");
        assert!(nested_norm.contains("sub/notes.txt:1:"), "unexpected output: {}", nested);

        let _ = tokio::fs::remove_dir_all(&sandbox).await;
    }
}
