//! Built-in default skills shipped with the harness.
//!
//! These are discovered first by `HarnessConfig::discover_skills_in` with
//! `builtin://` paths. A workspace skill with the same name overrides a built-in,
//! and any built-in can be disabled by name in `HarnessConfig::disabled_skills`.

/// A built-in skill. Fields mirror the user-authored `SkillDefinition`.
pub struct BuiltinSkill {
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

/// Default skills available out of the box.
pub const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        name: "code",
        description: "Implement a change well: read context, make minimal correct edits, verify with tests.",
        content: r#"# Code skill

Use when implementing a feature, fixing a bug, or refactoring.

## Procedure
1. **Read before writing.** Inspect the relevant code and its callers; reuse existing patterns, naming, and style.
2. **Smallest correct change.** Make minimal edits that fix the root cause; do not rewrite working code or expand scope.
3. **Do not break the build.** Keep public signatures compatible unless a change is required; update call sites.
4. **Verify.** Run the narrowest check first, then the full test suite; read the result. Add/adjust tests for the change. Leave records, logs, and test data as they were.
5. **Safety.** Sandbox all file paths; keep edits inside the workspace; never silence an error to make a check pass.
6. **Honesty.** Report what you changed and what you verified; surface failures.

## Tools
Prefer `read_file`/`grep_search`/`list_dir`, `edit_file` for exact edits, `write_file` for new files, `bash` for builds/tests."#,
    },
    BuiltinSkill {
        name: "orchestrator",
        description: "Decompose a goal, coordinate subagents/specialists, integrate and verify results.",
        content: r#"# Orchestrator skill

Use when a task is large, parallelizable, or benefits from separate specialists.

## Procedure
1. **Decompose.** Break the goal into independent, well-scoped units with clear ownership and non-overlapping scope.
2. **Coordinate.** Dispatch subagents (`invoke_subagent`) or swarm specialists (`swarm_board`) with complete, self-contained instructions; prefer parallel independent work.
3. **Constrain.** Give each worker a fresh context, explicit inputs, allowed tools, and a precise deliverable. Avoid duplicating work.
4. **Integrate.** Collect results, reconcile conflicts, and assemble one coherent answer or change set.
5. **Verify.** Re-run tests/checks on the integrated result; do not trust a worker's self-report — confirm it actually did the work.
6. **Own outcomes.** If a sub-result is wrong or a worker stalls, fix it or take over; report what shipped and what did not.

## Tools
`invoke_subagent`, `swarm_board`, `create_specialist`, `create_artifact`, `create_skill`."#,
    },
    BuiltinSkill {
        name: "reviewer",
        description: "Review code/diffs for correctness, security, and style; report findings without editing.",
        content: r#"# Reviewer skill

Use when reviewing a change, diff, or file. **Do not edit; report.**

## Look for
- **Correctness:** logic errors, edge cases, off-by-one, null/None, error handling, concurrency, resource cleanup, API misuse.
- **Security:** injection, path traversal/sandbox escape, secret leakage, missing authz, unsafe deserialization, TOCTOU, DoS.
- **Safety/reversibility:** hard-to-undo actions, missing guards, lost data, persistence bugs.
- **Contracts:** breaking changes, inconsistent types, schema/migration issues, tests that can't fail.
- **Style/consistency:** project conventions, naming, dead code.

## Output
Ranked findings. For each: severity, exact `file:line`, evidence, why it matters, suggested fix. Separate verified facts from inference. Say what you could NOT verify. Do not invent issues.

## Tools
`read_file`, `grep_search`, `list_dir`; run tests read-only where possible. No `write_file`/`edit_file`."#,
    },
    BuiltinSkill {
        name: "knowledge-work",
        description: "Research, answer questions, summarize, and draft; be accurate, cite sources, flag uncertainty.",
        content: r#"# Knowledge work skill

Use for research, analysis, Q&A, summarization, drafting, and synthesis (not code editing).

## Procedure
1. **Ground in sources.** Prefer reading provided files / fetching pages over recalling; cite exact `file:line` or URLs.
2. **Be accurate.** Do not invent facts, numbers, or citations. Mark estimates and unknowns; say `I don't know` rather than guess. Distinguish evidence from inference.
3. **Structure.** Lead with the answer; then support; then caveats. Match the requested format and length.
4. **Summarize faithfully.** Capture key points without distortion; flag what was omitted.
5. **Draft cleanly.** Produce ready-to-use prose; no padding or meta-commentary.
6. **Calibrate confidence.** State confidence; note what would change the conclusion.

## Tools
`read_file`, `grep_search`, `list_dir`, `web_search`, `web_fetch`, `create_artifact`. No file edits unless asked."#,
    },
    BuiltinSkill {
        name: "modify-harness",
        description: "Change the harness itself: add skills, tools, providers, plugins, automations, or settings safely.",
        content: r#"# Modify harness skill

Use when the user asks you to extend or change the harness itself — add a skill, custom tool,
provider, plugin, automation, or change settings. You are editing the runtime the user is
currently using, so changes must be reversible, validated, and persisted.

## Principles
1. **Persist, don't just display.** A change made only in the UI's in-memory state is lost on
   restart. Every change must end with a successful API call that persists config.
2. **Never fabricate credentials.** If a provider needs a key, ask the user for it. Never invent
   or reuse a key from another provider.
3. **Validate before saving.** Check required fields, then confirm the result by re-reading it.
4. **Explain what you changed** and how to undo it.

## Adding a provider
- Prefer the UI flow (`Settings → Providers → Add Provider`). Programmatically: `POST /api/config`
  with a new `custom_providers` entry, then `POST /api/providers/active {"provider_id": "<id>"}`.
- The provider must be persisted **before** activation; activation returns 404 for unknown ids.
- Required: `id`, `name`, `api_base`, `api_key`, `default_model`.
- After saving, verify with `GET /api/config` that the provider exists and `provider` matches.

## Adding a custom tool
- Use the `manage_tools` tool (`action: create`) — it writes `.harness/tools/<name>.json` and
  hot-registers the tool for the next turn.
- Declare an accurate `permission_level`; a shell/HTTP runner requires at least `shell_or_network`.
- Test it with `POST /api/tools/<name>/validate` before relying on it.

## Adding a skill
- Use the `create_skill` tool with `name`, `description`, and full Markdown `content`.
- Or write `.harness/skills/<name>.md` with `name`/`description` frontmatter.
- Skills are discovered on the next turn and invoked as `/<name>`.

## Adding an automation
- Use `manage_automations` (`action: create`) with `name`, `prompt`, and a `trigger`:
  `manual`, `interval` (+ `every` like `30m`), `cron` (5 fields), or `always_on`.
- Bind it to a project with `project_id`, or omit for a standalone automation.
- Test immediately with `action: run` and confirm a session was created.

## Changing settings
- `POST /api/config` merges; project state (`projects`, `active_project_id`, `sandbox_root`) is
  server-owned and preserved. Secrets are redacted in responses — send `__harness_redacted__`
  (or omit) to keep a stored secret, or an explicit value to change it.
- Prefer narrow changes: modify only the fields you intend to change.

## Verify
- Re-read the affected resource (`GET /api/config`, `GET /api/tools`, `manage_automations list`).
- If you changed permissions or providers, say so explicitly — these affect safety and cost."#,
    },
];


#[cfg(test)]
mod tests {
    #[test]
    fn ships_default_skills() {
        let names: Vec<&str> = super::BUILTIN_SKILLS.iter().map(|s| s.name).collect();
        for required in ["code", "orchestrator", "reviewer", "knowledge-work"] {
            assert!(names.contains(&required), "missing built-in skill {required}");
        }
        for s in super::BUILTIN_SKILLS {
            assert!(!s.description.is_empty());
            assert!(!s.content.is_empty());
        }
    }
}
