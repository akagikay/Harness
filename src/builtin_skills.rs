//! Built-in default skills shipped with the harness.
//!
//! These are discovered first by `HarnessConfig::discover_skills_in` with
//! `builtin://` paths. A workspace skill with the same name overrides a built-in,
//! and any built-in can be disabled by name in `HarnessConfig::disabled_skills`.
//!
//! Every entry also carries `triggers`: the high-signal keywords the deterministic
//! router in `crate::skill_router` matches a user message against (word-stemmed,
//! case-insensitive) so a relevant skill can be injected without the user typing
//! `/skill-name`. Keep them concrete verbs/nouns of the skill's subject, and keep
//! each body tight (150-600 words) — bodies are injected verbatim into context.

/// A built-in skill. Fields mirror the user-authored `SkillDefinition`, plus the
/// router's `triggers` keywords.
pub struct BuiltinSkill {
    pub name: &'static str,
    pub description: &'static str,
    /// High-signal keywords for automatic routing (see `crate::skill_router`).
    pub triggers: &'static [&'static str],
    pub content: &'static str,
}

/// Default skills available out of the box.
pub const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        name: "code",
        description: "Implement or fix code well: read context, make minimal correct edits, verify with tests.",
        triggers: &[
            "implement", "code", "coding", "fix", "bug", "refactor", "build", "compile", "feature",
            "edit", "patch", "function", "tests", "cargo", "rust",
        ],
        content: r#"# Code skill

Use when implementing a feature, fixing a bug, or changing existing code — the
execution half of a change you already understand. Use `codebase-analysis` first
when the change is still unclear.

## Procedure
1. **Read before writing.** `read_file{path, start_line, end_line}`,
   `grep_search{pattern, include, path}` and `list_dir{path}` for the affected
   code and its callers. Reuse existing patterns, naming, and style.
2. **Smallest correct change.** Fix the root cause with minimal edits; do not
   rewrite working code or expand scope. Keep public signatures compatible unless
   a change is required, and update call sites.
3. **Edit precisely.** `edit_file{path, target, replacement}` for exact edits to
   existing files; `write_file{path, content}` only for new files. Paths are
   sandboxed to the workspace.
4. **Verify.** Run the narrowest check first, then the full suite:
   `bash{command:"cargo test", timeout_secs}` (only `timeout_secs` set when the
   run needs longer than the default). Read the result; never silence an error to
   make a check pass. Add or adjust tests for the change.
5. **Leave traces alone.** Do not modify records, logs, fixtures, or test data to
   make something pass.
6. **Honesty.** Report what you changed, what you verified, and what you could not
   verify. Surface failures instead of hiding them.

## Tools
`read_file`, `grep_search`, `list_dir`, `edit_file`, `write_file`, `bash`. For
parallel or independent work see `/orchestrator`; for independent verification
see `/swarm-review`."#,
    },
    BuiltinSkill {
        name: "codebase-analysis",
        description: "Understand a codebase or plan a refactor: map ownership and behaviour, propose a testable plan before editing.",
        triggers: &[
            "analyze", "analysis", "understand", "trace", "explore", "architecture", "plan",
            "mapping", "explain", "survey", "onboarding", "how does", "refactor plan", "impact",
            "dependencies",
        ],
        content: r#"# Codebase analysis skill

Use when you must understand code before changing it: onboarding, "how does X
work?", impact analysis, or a refactor plan. Read-only: it ends in a plan, not in
edits. Hand the plan to `/code` for execution.

## Procedure
1. **Find the entry points.** `list_dir` the workspace root; `grep_search` for
   `fn main`, `pub mod`, `pub trait`, `pub struct`, and route/handler definitions
   to locate the real boundaries (`src/lib.rs`, `src/main.rs`, `src/plugins/mod.rs`).
2. **Read the seams, not everything.** Read module roots and contracts first
   (`src/plugins/contract.rs` for tool shapes, `src/config.rs` for settings), then
   follow callers with `grep_search`.
3. **Map ownership.** For each area note: which file owns it, who calls it, what
   it persists, and what would break. Record `file:line` evidence for every claim.
4. **Plan with tests.** `create_artifact{title, kind:"plan", content}` with:
   affected files, new/changed public interfaces, data or schema impact, edge
   cases, the tests to add, and the verification command (`cargo test`,
   `cargo build`). Sequence the steps so each one leaves the tree compiling.
5. **State risks honestly.** Mark what you could not verify and what needs a
   decision from the user. Do not start editing.

## Tools
`list_dir`, `grep_search`, `read_file`, `create_artifact`. No `write_file`,
`edit_file`, or `bash` unless the user explicitly asks you to implement."#,
    },
    BuiltinSkill {
        name: "reviewer",
        description: "Review a diff or file read-only for correctness, security, and style; report ranked findings without editing.",
        triggers: &[
            "review", "feedback", "critique", "check", "inspect", "diff", "pull request", "pr",
            "findings", "bug hunt", "audit code", "code quality", "regression",
        ],
        content: r#"# Reviewer skill

Use for reviewing a change, diff, PR, or file. **Read-only: do not edit; report.**
For fan-out verification across several agents use `/swarm-review` instead.

## Look for
- **Correctness:** logic errors, edge cases, off-by-one, `Option`/`Result`
  mishandling, concurrency and cancellation, resource cleanup, API misuse.
- **Security:** injection, path traversal or sandbox escape, secret leakage,
  missing authorization, unsafe deserialization, TOCTOU, unbounded input.
- **Reversibility:** hard-to-undo actions, missing guards, lost data, persistence
  bugs, tests that cannot fail.
- **Contracts:** breaking changes, schema/migration issues, type drift.
- **Style/consistency:** project conventions, naming, dead code.

## Output
Ranked findings. For each: severity, exact `file:line`, the evidence, why it
matters, and a suggested fix. Separate verified facts from inference, say what you
could NOT verify, and never invent issues.

## Tools
`read_file`, `grep_search`, `list_dir`; run tests read-only where possible. Never
`write_file`, `edit_file`, or `create_artifact`."#,
    },
    BuiltinSkill {
        name: "web-research",
        description: "Grounded research and Q&A: search and fetch sources, answer with citations, flag uncertainty.",
        triggers: &[
            "research", "cite", "citation", "source", "reference", "web", "search", "look up",
            "latest", "news", "compare", "paper", "evidence", "online", "documentation",
        ],
        content: r#"# Web research skill

Use for research, Q&A, summarization, and synthesis that needs current or
external facts. Prefer local files (`read_file`, `grep_search` with `file:line`)
when the answer is already in the workspace; go to the web for everything else.

## Procedure
1. **Search first.** `web_search{query, max_results}` (default 5, max 50).
   `mcp_google_ai_search_search` is available when the Google AI Search MCP server
   is enabled. Run several query variants before concluding.
2. **Fetch the primary source.** `web_fetch{url, purpose}` the top results and
   read them; do not rely on snippets.
3. **Cite every claim.** Give the exact URL for each sourced fact and
   `file:line` for local ones. Never fabricate a citation, number, or quote —
   say `I don't know` instead.
4. **Calibrate.** Lead with the answer, then the evidence, then caveats. Mark
   estimates as estimates, distinguish evidence from inference, and note what
   would change the conclusion.
5. **Summarize faithfully.** Capture key points without distortion and flag what
   you omitted. Draft ready-to-use prose with no padding.

## Tools
`web_search`, `web_fetch`, `read_file`, `grep_search`, `list_dir`,
`create_artifact`. No file edits unless asked."#,
    },
    BuiltinSkill {
        name: "browser-ui-audit",
        description: "Inspect a running UI with the headless browser: snapshots, console/network errors, interactions, screenshots.",
        triggers: &[
            "browser", "browsing", "browser_ui", "ui", "ux", "page", "screenshot", "frontend",
            "layout", "css", "html", "render", "click", "form", "visual", "console", "dom",
            "responsive",
        ],
        content: r#"# Browser UI audit skill

Use when a UI must be inspected or verified in a real browser: layout, console
errors, failed requests, storage, or interaction flows. Report only what tool
output proves.

## Procedure
1. **Cheapest check first.** `browser_ui{action:"http_check", url, assert_status,
   assert_text}` verifies status and content without a browser engine.
2. **Orient.** `browser_ui{action:"navigate", url}`, then
   `browser_ui{action:"snapshot", url}` to get indexed elements (`ref_id`, tag,
   role, text, selector, visibility) — prefer `ref_id` over hand-written CSS.
3. **Diagnose.** `browser_ui{action:"debug_audit", url}` for console errors,
   failed requests, and horizontal overflow; `inspect_dom` for specific markup;
   `storage` (with `storage_type`: `all|local|session|cookie`, `storage_key`) to
   read/clear tokens and flags.
4. **Exercise it.** `browser_ui{action:"interact", url, steps:[{interaction:"click",
   ref_id}, {interaction:"fill", ref_id, text}, {interaction:"press_key", key:"Enter"},
   {interaction:"assert_text", selector, text}]}`; also `select_option`, `hover`,
   `wait_for_selector`, `scroll_to`, `eval`. Re-snapshot after navigation.
5. **Show it.** `browser_ui{action:"screenshot", url, screenshot_name, full_page}`
   writes an image into the workspace (`screenshot_name` must be a plain filename).
6. **Verify the fix.** Re-run the same action after a change and compare output.
   Raise `timeout_secs` (max 90) for slow pages instead of retrying blindly.

## Tools
`browser_ui`, `read_file`, `write_file`, `grep_search`, `bash` (to serve or build
the app). If no headless Chromium/Edge is installed, fall back to `http_check` and
say so."#,
    },
    BuiltinSkill {
        name: "swarm-review",
        description: "Fan out independent read-only reviewers across background subagents, then reconcile their findings.",
        triggers: &[
            "swarm", "parallel review", "multi-agent", "fan out", "second opinion", "cross-check",
            "consensus", "verify independently", "multiple perspectives", "reviewers", "board",
        ],
        content: r#"# Swarm review skill

Use when verification needs independent perspectives: a risky change, a security
review, or a disagreement worth settling. Delegating *work* is `/orchestrator`;
this skill is for independent, read-only scrutiny.

## Procedure
1. **Frame it.** `swarm_board{action:"set_title", title:"..."}`, then
   `create_job` (`title`, `description`, `status:"open"`) and `create_specialist`
   (`title`, `specialty`, `read_only:true`) tags so work is attributable.
2. **Dispatch.** `invoke_subagent` with `tasks:[{role, prompt, read_only:true}]`
   (max 8 per batch) — distinct lenses such as correctness, security, contracts,
   tests, and docs. Give each reviewer the exact files, the diff, and an output
   contract. Default `wait:false` runs them in the background.
3. **Tag them.** Pass `job_tag` and `specialist` so
   `swarm_board{action:"list_jobs"}` / `list_specialists` show who owns what.
4. **Wait cheaply.** Sleep zero-CPU with
   `swarm_board{action:"wait", since:<seq>, wait_all:true, timeout_secs}` — never
   busy-poll `status`/`read`.
5. **Reconcile.** Merge findings by `file:line`, drop unverified claims, and
   resolve conflicts by re-reading the code yourself. Never accept a worker's
   self-report as proof: confirm the finding against the file.
6. **Report.** One ranked list with severity, evidence, and suggested fix, plus
   the dissent you could not resolve.

## Tools
`invoke_subagent`, `swarm_board`, `read_file`, `grep_search`, `list_dir`."#,
    },
    BuiltinSkill {
        name: "orchestrator",
        description: "Decompose a large or parallelizable goal into units, delegate to subagents, integrate and verify the result.",
        triggers: &[
            "orchestrate", "decompose", "delegate", "parallelize", "split", "coordinate", "subagent",
            "break down", "assign", "specialists", "big task",
        ],
        content: r#"# Orchestrator skill

Use when a task is large, parallelizable, or benefits from separate specialists.
Independent *verification* is `/swarm-review`.

## Procedure
1. **Decompose.** Split the goal into independent units with clear ownership and
   no overlapping files. Each unit gets an explicit deliverable and acceptance
   criteria.
2. **Dispatch self-contained work.** `invoke_subagent{role, prompt, files,
   job_tag, specialist, read_only, model, thinking_effort, server_id}` or batch
   several via `tasks` (max 8). A worker starts with a fresh context, so its prompt
   must repeat every input it needs. Prefer `wait:false` background execution.
3. **Coordinate.** Tag jobs and specialists with `swarm_board` so you can see
   progress: `swarm_board{action:"wait", since:<seq>, wait_all:true}` sleeps
   zero-CPU instead of busy-polling.
4. **Constrain.** Set `read_only:true` for investigation-only workers; give write
   access only to the owner of those files to prevent conflicting edits.
5. **Integrate.** Collect results, reconcile conflicts, and assemble one coherent
   change set; re-run `cargo test` / `cargo build` on the integrated result.
6. **Own outcomes.** If a worker is wrong or stalls, fix it or take over. Do not
   trust a self-report: confirm the work exists.

## Tools
`invoke_subagent`, `swarm_board`, `create_artifact`, `read_file`, `bash`."#,
    },
    BuiltinSkill {
        name: "workspace-audit",
        description: "Audit this workspace and host: skills, tools, automations, permissions, and resource health with evidence.",
        triggers: &[
            "audit", "health", "status", "diagnose", "diagnostics", "system", "workspace",
            "environment", "inventory", "setup", "resources", "disk", "memory", "cpu", "permission",
        ],
        content: r#"# Workspace audit skill

Use for "what is going on with this workspace/host?" — a read-only systems audit
that ends in a findings report, not in changes.

## Procedure
1. **Host health.** `sys_info` for OS, CPU, and harness memory/process usage.
2. **Workspace inventory.** `list_dir{path:"."}` the root, then `grep_search`
   for `.harness/`, `harness.db`, build artifacts, and stray files. Note the
   project folders and the active sandbox.
3. **Capabilities.** `manage_tools{action:"list"}` for custom tools and their
   permission tiers, `manage_automations{action:"list"}` for automations and
   their triggers. Read the skill index for anything stale or duplicated.
4. **Permissions.** Report the effective permission mode and which tools are
   denied or ask-gated (`bash`, `write_file`, `edit_file`, `web_*`); flag
   over-broad rules and missing ones.
5. **Report.** `create_artifact{title, kind:"report", content}` with a findings
   table: area, observation, `file:line`/tool evidence, severity, suggested fix.
   Distinguish verified facts from inference.
6. **Do not change anything** unless the user asks; propose the fix and let them
   decide.

## Tools
`sys_info`, `list_dir`, `read_file`, `grep_search`, `manage_tools`,
`manage_automations`, `create_artifact`."#,
    },
    BuiltinSkill {
        name: "automation-authoring",
        description: "Author and test first-class automations: scheduled agent runs with manual, interval, cron, or always-on triggers.",
        triggers: &[
            "automation", "automate", "schedule", "scheduled", "recurring", "cron", "interval",
            "periodic", "nightly", "daily", "watchdog", "reminder", "background job",
        ],
        content: r#"# Automation authoring skill

Use when work must recur without a human prompt: "check the build every night",
"watch this repo weekly". Automations are persisted config, so author carefully
and test before trusting them.

## Procedure
1. **Check what exists.** `manage_automations{action:"list"}` — extend an
   existing automation rather than adding a near-duplicate.
2. **Write a self-contained prompt.** Each run is a fresh session with no
   conversation history: state the goal, the tools to use, the inputs, and the
   acceptance criteria inside `prompt` itself.
3. **Create it.** `manage_automations{action:"create", name, prompt, description,
   trigger}` where `trigger` is `manual`, `interval` (+ `every` like `"30m"`/`"2h"`
   or `every_secs`, minimum 60), `cron` (5 fields: minute hour day month weekday,
   e.g. `"*/10 * * * *"`), or `always_on`. Bind it to a project with `project_id`,
   or omit for a standalone automation.
4. **Test immediately.** `manage_automations{action:"run", id}` and confirm a
   session was actually created and produced the expected result. A cron or
   interval trigger never fires on demand, so `run` is the only honest test.
5. **Verify persistence.** Re-read with `manage_automations{action:"list"}`
   (config is persisted server-side; a UI-only change is lost on restart).
6. **Lifecycle.** `enable` / `disable` / `remove` by `id`. Disable rather than
   remove when you may re-enable it, and tell the user what changed and why.

## Tools
`manage_automations`, `create_artifact` (to write the runbook),
`read_file`, `grep_search`."#,
    },
    BuiltinSkill {
        name: "skill-authoring",
        description: "Write a reusable skill: clear trigger description, actionable body, correct tool usage, and real verification.",
        triggers: &[
            "skill", "author a skill", "create a skill", "new skill", "template", "playbook",
            "runbook", "sop", "codify", "distill", "instructions", "condense", "reusable",
        ],
        content: r#"# Skill authoring skill

Use when the user wants a reusable skill, or when a session should be condensed
into one (`/condense` does exactly this).

## Procedure
1. **Check for a duplicate.** Read the skill index first; extend an existing
   skill instead of shipping a near-identical one. Never reuse the name of a
   built-in unless you intend to override it.
2. **Name it for routing.** A short ASCII slug (`review-changes`,
   `browser-smoke`). It becomes the `/<name>` slash command and appears in the
   skill index.
3. **Write a trigger-shaped description.** One sentence naming *when* to use the
   skill, using the words a user would actually type — the deterministic router
   matches on these plus the body keywords.
4. **Write an actionable body** (150-600 words, no filler):
   - Goal and when *not* to use it.
   - Ordered steps with real tool calls, e.g. `read_file{path, start_line,
     end_line}`, `edit_file{path, target, replacement}`,
     `bash{command:"cargo test", timeout_secs}`,
     `browser_ui{action:"snapshot", url}`,
     `swarm_board{action:"wait", since:<seq>, wait_all:true}`.
   - Constraints: read-only vs write, sandbox rules, what must never change.
   - Verification: the exact command or check that proves success.
   - Output format the model should produce.
   Verify every tool name and argument against the contract before writing it.
5. **Persist it.** Either `write_file` to `.harness/skills/<slug>.md` with
   `---\nname: <slug>\ndescription: ...\n---` frontmatter, or call
   `create_skill{name, description, content}`. Skills are discovered on the
   next turn.
6. **Test it.** Invoke `/<slug>` in a fresh session and confirm the injected
   directive reads correctly and the steps actually work. Fix and re-save if not.

## Tools
`read_file`, `grep_search`, `list_dir`, `write_file`, `create_skill`."#,
    },
    BuiltinSkill {
        name: "modify-harness",
        description: "Change the harness itself: add providers, tools, skills, or automations, and change settings safely.",
        triggers: &[
            "provider", "api key", "settings", "config", "configure", "custom tool", "add a tool",
            "extend the harness", "self-modify", "endpoint", "api base", "model setup",
        ],
        content: r#"# Modify harness skill

Use when the user asks you to extend or change the harness itself — a provider,
custom tool, skill, automation, or setting. You are editing the runtime the user
is using right now, so changes must be persisted, validated, and reversible.
For a *new skill* use `/skill-authoring`; for *automations* use
`/automation-authoring`.

## Principles
1. **Persist, don't just display.** A change left in the UI's in-memory state is
   lost on restart: every change must end with a successful API call.
2. **Never fabricate credentials.** If a provider needs a key, ask for it. Never
   invent or reuse a key from another provider.
3. **Validate before saving.** Check required fields, then confirm by re-reading
   the resource.
4. **Explain what changed** and how to undo it, especially permission or
   provider changes (they affect safety and cost).

## Providers
Prefer `Settings -> Providers -> Add Provider`. Programmatically:
`POST /api/config` with a new `custom_providers` entry, then
`POST /api/providers/active {"provider_id":"<id>"}`. The provider must be
persisted before activation (activation 404s on unknown ids). Required fields:
`id`, `name`, `api_base`, `api_key`, `default_model`. Verify with `GET /api/config`.

## Custom tools
`manage_tools{action:"create"}` writes `.harness/tools/<name>.json` and
hot-registers the tool for the next turn. Declare an honest `permission_level`
(`read_only`, `workspace_write`, `shell_or_network`, `high_risk`, `meta`) — a
shell or HTTP runner needs at least `shell_or_network`. Use
`action:"inspect"`/`"list"` to confirm, and remove what you cannot verify.

## Settings
`POST /api/config` merges a full config snapshot. Project state (`projects`,
`active_project_id`, `sandbox_root`) is server-owned and preserved. Secrets are
redacted in responses: send `__harness_redacted__` (or omit) to keep a stored
secret, or an explicit value to change it. Prefer narrow changes.

## Verify
Re-read the affected resource (`GET /api/config`, `manage_tools{action:"list"}`,
`manage_automations{action:"list"}`) before reporting success."#,
    },
];


#[cfg(test)]
mod tests {
    #[test]
    fn ships_default_skills() {
        let names: Vec<&str> = super::BUILTIN_SKILLS.iter().map(|s| s.name).collect();
        for required in [
            "code",
            "reviewer",
            "web-research",
            "browser-ui-audit",
            "swarm-review",
            "skill-authoring",
        ] {
            assert!(names.contains(&required), "missing built-in skill {required}");
        }
        for s in super::BUILTIN_SKILLS {
            assert!(!s.description.is_empty());
            assert!(!s.content.is_empty());
            // Router keywords must exist and be lowercase-safe.
            assert!(!s.triggers.is_empty(), "{} needs triggers", s.name);
            for t in s.triggers {
                assert_eq!(*t, t.to_ascii_lowercase(), "{}: trigger '{t}' must be lowercase", s.name);
            }
        }
    }

    #[test]
    fn skill_names_are_unique_and_slug_shaped() {
        let mut seen = std::collections::HashSet::new();
        for s in super::BUILTIN_SKILLS {
            assert!(seen.insert(s.name), "duplicate built-in skill {}", s.name);
            assert!(
                s.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{} must be a lowercase slug",
                s.name
            );
        }
    }

    #[test]
    fn bodies_stay_tight_and_actionable() {
        for s in super::BUILTIN_SKILLS {
            let words = s.content.split_whitespace().count();
            assert!(
                (100..=700).contains(&words),
                "{} body is {} words; keep it in the 100-700 range",
                s.name,
                words
            );
        }
    }

    #[test]
    fn no_builtin_duplicates_another_skill_scope() {
        // Scopes must stay distinct: delegating work vs verifying it vs planning.
        let by_name = |n: &str| {
            super::BUILTIN_SKILLS
                .iter()
                .find(|s| s.name == n)
                .expect("skill present")
        };
        // Each skill points at the neighbour that owns the adjacent scope.
        assert!(by_name("orchestrator").content.contains("Independent *verification*"));
        assert!(by_name("swarm-review").content.contains("Delegating *work*"));
        assert!(by_name("codebase-analysis").content.contains("Read-only"));
        assert!(by_name("reviewer").content.contains("Read-only"));
        assert!(by_name("skill-authoring").content.contains("/condense"));
        assert!(by_name("modify-harness").content.contains("/skill-authoring"));
    }

    #[test]
    fn every_skill_references_at_least_one_real_tool() {
        for s in super::BUILTIN_SKILLS {
            let any = s
                .content
                .split('`')
                .any(|chunk| chunk.starts_with("read_file")
                    || chunk.starts_with("write_file")
                    || chunk.starts_with("edit_file")
                    || chunk.starts_with("bash")
                    || chunk.starts_with("grep_search")
                    || chunk.starts_with("list_dir")
                    || chunk.starts_with("web_search")
                    || chunk.starts_with("web_fetch")
                    || chunk.starts_with("browser_ui")
                    || chunk.starts_with("invoke_subagent")
                    || chunk.starts_with("swarm_board")
                    || chunk.starts_with("create_artifact")
                    || chunk.starts_with("manage_tools")
                    || chunk.starts_with("manage_automations")
                    || chunk.starts_with("create_skill")
                    || chunk.starts_with("sys_info")
                    || chunk.starts_with("POST")
                    || chunk.starts_with("GET"));
            assert!(any, "{} must cite at least one real tool or API call", s.name);
        }
    }
}