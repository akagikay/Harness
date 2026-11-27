# Harness — Adversarial Loop Progress

_Running log. Each round: a set of adversarial agents attacks the project; findings are consolidated here and fixed; the loop repeats until a clean round._

Last updated: 2026-11-28 (round 4 prep)

---

## Current state

- **Version**: `1.1.4` (commit `e1d4020`, release built, not yet published)
- **Tests**: 211 unit + 66 integration, all green
- **Cluster topology** (for live testing): 1 client + 3 execution servers, each
  server in its own sandbox with a `ONLY-<name>.txt` marker file. 4 projects at
  4 scopes (3 single-folder + 1 multi-folder spanning all three).
- **Model**: NVIDIA NIM `nvidia/nemotron-3.5-lightning-30b-a3b` (the previously
  used `nemotron-3-nano-30b-a3b` is end-of-life on NVIDIA's API). Key is already
  configured in the client DB.

### Important operational notes
- **Do not start a cluster inside the repo** — it leaves `cluster/`, `harness.db`
  and `harness.key` behind. Use a temp dir. `harness.db`/`harness.key` are
  git-ignored on purpose.
- `--mode server` serves only the execution API (no UI). Default mode serves the UI.
- Release asset names must be exactly `harness.exe`, `harness-client.exe`,
  `harness-server.exe`, `SHA256SUMS`; the SHA256SUMS format is `<64-hex>  <name>`
  with LF endings (a reversed file shipped once and broke every install).
- **Never edit source with PowerShell `Set-Content`/`-replace`/`Out-File`** — it
  corrupts UTF-8 (emoji/codepoints) and can eat Rust delimiters like `>>`.
  Use the `edit`/`write` tools.

---

## Round 1 — live cluster QA (1 client + 3 servers)

**Method**: API-driven (no browser tool available to subagents) + PowerShell
filesystem verification + real model turns.

### Verified working
- Per-scope execution: each project's agent saw only its own folder.
- Scope isolation, 3/3 negative tests blocked with the exact errors:
  - alpha → `ONLY-beta.txt`: `read_file [not_found]: Failed to read file '...\srv-alpha\ONLY-beta.txt'`
  - beta → `ONLY-alpha.txt`: same shape
  - gamma → `list_dir "C:\Users"`: `Path escapes the workspace sandbox: absolute path outside project folders`
- Writes always landed in the correct sandbox, never in the others.
- Remote routing is enforced server-side (the server overrides `sandbox_root`).
- Automation: create → run (real model, 4s, 9348 tokens) → PATCH edit → Log → delete.
- File browser navigation, rename/mkdir/delete, protected-root guards.

### Bugs found → all FIXED in `e1d4020`
| Bug | Fix |
|---|---|
| Concurrent session creates shared a `ses_<ms>` id → conversations silently destroyed | `unique_session_id()` retries until unused |
| Multi-folder projects lost extra folders on remote servers | project folders now travel in the dispatch config as approved access roots |
| `/api/onboarding` said "no provider" on a working setup | `has_usable_provider()` covers all config paths |
| `mkdir` returned 200 for an existing folder | returns `409` |
| Folder picker listed `harness.key` / `harness.db` / `.env` | those names are never listed |
| Denied permission burned 120s per model retry | 3 unanswered requests abort the turn |

**Live verification after fixes**: 12 concurrent creates → 12 unique ids and 12
listed sessions; `p-cross` on server `alpha` can list all 3 folders;
`provider_configured:true`; duplicate `mkdir` → `409`; client dir lists only
`err.txt`, `log.txt`.

---

## Round 2 — hostile UI/UX critique (source-only review)

### P0-1 — the primary CTA created the wrong chat, and Ctrl+N did the opposite
`app.js:1205` bound the sidebar button to `startNewConversation(true)` (knowledge
mode, no file/shell tools) while `app.js:5556` bound Ctrl+N to
`startNewConversation()` (project mode). Both advertised "New Conversation ⌃N".
The placeholder never names the mode either.
**FIXED**: the button now creates a project chat like Ctrl+N and is labelled
**"New Project Chat"** with a tooltip. *Remaining: the composer still does not
name the active mode/project visibly.*

### P0-2 — a permission prompt could be invisible, stalling the agent forever
The card was appended to the stream with no `role="alert"`, no aria-live, and no
scroll-into-view; with auto-scroll off the agent hung with no signal.
**FIXED**: `role="alert"` + `aria-live="assertive"`, forced `scrollIntoView`,
a pulsing top-bar badge (`#permission-badge`, "N awaiting approval") that
scrolls to and focuses the prompt, `body.has-pending-permission` styling, and
the prompt now says **"Waiting for your approval — the agent is paused"**.
Also added **"Approve & always allow"** with a real session allowlist
(`state.session_tool_allowlist`) so a chatty `bash` is not re-prompted; the
allowlist is cleared when the session is deleted.

### P0-3 — first run ended in a raw, unactionable error
`throw new Error('Failed to create session')` with no hint; onboarding dropped a
typed API key when the base was empty; the wizard had no connection test; Escape
could not dismiss it and it never took focus.
**FIXED (partial)**: the session-create failure now names the cause and the fix
("No API key or model configured. Settings → AI Providers …"). *Remaining: the
wizard still has no "Test key" step, no focus management, and no Escape handling.*

### P1-5 — a settings section that never hid
`#settings-skill-router` was a sibling `settings-subsection`, so
`switchSettingsTab` (which only toggles `#settings-page-*`) left it dangling
under every tab.
**FIXED**: it now lives inside `#settings-page-skills`; verified rendered only on
the Skills tab (`onSkills:true`, `onProviders:false`, `onMemory:false`, `onMcp:false`).

### Still OPEN from round 2
- P1-1: Queue vs Steer lives in *Memory & Runtime*; only a hover tooltip hints it. Enter silently flips meaning.
- P1-2: `stopActiveSession`/`stopSubagent` only `console.error` on failure; the server "health" dot is hardcoded green for `local`.
- P1-3: model capabilities, Knowledge mode and telemetry meaning are hover-only.
- P1-4: 12 `outline:none` vs 2 `:focus-visible`; no focus trap on any modal; `--text-dim` (#484b59) is ~2.2:1 contrast.
- P1-6: right-click is the only path for project menus and file-browser actions; rename/new-folder use native `window.prompt` despite a custom dialog.
- P2s: dead `welcomeProjectTitle`; running pill has no elapsed time/step; inconsistent naming ("Safe (Ask)" vs "Safe (Interactive)"); swarm Members starts at 1; "Browse Workspace Files" opens the inspector instead of a picker.

---

## Round 3+ — running adversarial loop (max 4 concurrent agents)

Roles in rotation:
1. **Red team** — auth, sandbox escape, secret leakage, injection, the updater's trust chain.
2. **QA / bug finder** — functional correctness across every endpoint + real model turns.
3. **UX / product critic** — hostile review of the UI after each fix round.
4. **Feature / architecture reviewer** — what is missing or structurally wrong.

Rule: findings land in this file; only ONE agent edits source at a time.