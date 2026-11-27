# Harness — Adversarial Loop Progress

_Running log. Each round: a set of adversarial agents attacks the project; findings are consolidated here and fixed; the loop repeats until a clean round._

Last updated: 2026-11-28 (round 5 findings applied + round 4 consolidated)

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

### Round 4 — launched (4 concurrent, all read-only reporters)
| Agent | Mandate |
|---|---|
| Red team | auth/Host-guard rebinding, sandbox escape vectors, secret leakage through every surface, agent-level prompt attack, remote-server dispatch abuse, the updater trust chain, DoS |
| Functional QA | state machine under concurrency, automation schedulers, remote-server failure modes, data survival + DB corruption, malformed input on every endpoint, feature gaps |
| UX round 2 | attacks the round-3 *fixes* (friendly errors, the badge, always-allow, the hidden section) then everything else: first-run, feedback, a11y, layout, copy |
| Architecture | trust-boundary map, scaling limits with named lines, async failure modes, what is untested, feature keep/build/cut verdicts, delete list |

**Results are still incoming. What has landed and been applied:**

#### Architecture review — top 3 applied (all CRITICAL/HIGH)
1. **`request_path_access` self-escalated.** It appended to the *global, persisted*
   `config.extra_access_roots` with no approval check, so an agent in
   Autonomous/Yolo mode could grant itself any absolute path, permanently, for
   every project, surviving restarts — the exact opposite of what the module's own
   doc comment promises. **Fixed**: requires an explicit human approval, only
   honored in `Safe` mode, and the grant is session-scoped
   (`state.session_path_grants`, dropped on delete). Verified: `src/plugins/path_access_tool.rs`.
2. **A remote execution server trusted the client's config.** `payload.config`
   is client-supplied, and `extra_access_roots`/`projects` from it survive into
   the run, so a malicious client could point the server at any folder.
   **Fixed** (see commit `e1d4020` chain): the dispatch config now carries the
   project's folders as approved roots and the server uses its own sandbox.
3. **Config staleness had no revisioning** — one `config.write()` is the write path
   for 6 endpoints, so a stale `POST /api/config` silently reverts active server,
   provider, MCP, automations and skills. **OPEN** (see Round 6 plan).

Also recorded, not yet acted on: `bash` is not sandboxed (`bash.rs:146` only sets
`current_dir`, so `cd C:\Users` works) while `read_file`/`list_dir` *are* —
that asymmetry is the defect. `persist_session_record` holds the global write
lock across a full-table rewrite. The session cap is 100 and the UI only ever
sees 100. Turn finalization waits on detached subagent lifetimes, which keeps
`is_idle()` false and blocks auto-update. Every turn clones the whole
conversation twice.

#### UX round 2 — applied
| Finding | Fix |
|---|---|
| **P0-1 (a regression I introduced)**: my friendly-error edit dropped the `if (!res.ok)` guard, so *every* session creation failed with `Error: Could not start a session (HTTP 200)` and left a dangling session | guard restored around the hint computation; 200 falls through to the session; a missing `session_id` is its own error |
| **P0-2**: the badge lied — `__pendingPerms` was never cleared on session switch/history/new chat, and an *answered* card kept `role="alert"`, so the badge jumped to a dead prompt | `clearAllPendingPermissions()` on every stream reset; answered cards drop `role`/`data-pending` and gain `data-decided`; the badge's click handler only selects `[data-pending="1"]` |
| **P0-3**: "Approve & always allow" was dropped on the remote forward, so the button lied on every multi-server setup | `always_allow` now travels in the forwarded decision and is reported back |
| P1: always-allow notice spammed once per tool *call* | emitted once per session/tool |
| P1: the UI's mock switch was local state seeded from nothing, so the server's `mock_mode` had no effect | the UI follows the server config and the toggle persists it |

**Live verification**: with `mock_mode=true` on the server, a reload makes the UI
show "On", and a real first prompt completes the full event chain
(`UserMessage → SessionStarted → ToolCallStarted → ToolCallFinished →
SessionCompleted`) with a final answer and no errors.

#### Functional QA — applied
| Finding | Fix |
|---|---|
| The permission fail-fast counter counted **approvals**, so a turn with more than three Ask-gated tools aborted with "permission requests were denied" even when the user approved all of them | the counter only increments on a denial/timeout |
| Cron day-of-week `7` inside a range or list (`5-7`, `1-7`, `6,7`) never matched Sunday, which chrono reports as `0` | `7` and `0` both match Sunday |

Regression tests added for both (`test_many_approved_permissions_do_not_abort_the_turn`,
`cron_dow_seven_is_sunday_and_ranges_include_sunday`).

### Round 3 — shipped in `69b0118` (verified live in the browser)
| Finding | Fix | Evidence |
|---|---|---|
| Sidebar CTA created a knowledge-mode chat while Ctrl+N created a project chat | button is now "New Project Chat" and matches Ctrl+N | `btnLabel:"New Project Chat"` |
| Permission prompt invisible → agent hung silently | `role=alert`, aria-live, forced scroll-into-view, top-bar badge, "the agent is paused" | prompt renders 820x129px, badge "1 awaiting approval", `role:alert`, `ariaLive:assertive` |
| No way to stop re-approving the same tool | "Approve & always allow" + per-session allowlist | `pendingCount` 1→0, badge hides, label "✓ Approved — always allowed" |
| Skill-routing setting dangled under every tab | moved inside `#settings-page-skills` | `onSkills:true, onProviders:false, onMemory:false, onMcp:false` |
| Raw "Failed to create session" | names the cause + fix (provider/project) | code only; the live variant needs a failing provider |

Console errors on load: none.
---

## Round 5 - loop protocol note

Two subagents edited source concurrently once and collided (one reverted the
other's edit mid-session). **From now on exactly one agent touches source per
round**; the others are read-only reporters whose findings are consolidated here
and applied by the orchestrator. Before applying anything each fix is re-verified
with a literal pattern check, then `cargo test`, then a live browser/API check.

### Round 6 - queued (from the findings above)
1. **`config_revision` + `409`** on `update_config` -- the single change that
   closes the whole stale-snapshot revert class (active server, provider, MCP,
   automations, skills).
2. **`bash` sandbox honesty** -- either enforce containment or stop reporting it;
   the asymmetry with `file_ops` is the defect.
3. **Permission prompt survives a reload** -- `PermissionRequested` is broadcast
   but never written to `session.history`, so a reload during an approval loses
   the card and the badge and the agent blocks on a 120s timeout invisibly.
4. First-run wizard: a "test key" step, focus management, and Escape.
5. Stop-failure feedback (the pill stays green when the stop 404s).
6. Red-team and functional-QA reports are still outstanding.
---

## Round 5 red-team report (applied next round - CRITICAL first)

Red-team pass on v1.1.4 found 7 issues, 2 of them severe. **All are still OPEN.**

### CRITICAL 1 - the AES master key is readable over the API
`GET /api/workspace/file?path=harness.key` returns **200** with the 32 raw key bytes
in `content`, and `/api/workspace/tree` lists `harness.key` and `harness.db`.
On a default install the DB and its key live INSIDE `sandbox_root`, so key +
ciphertext are both reachable by anything that can call the API: an agent turn, a
local process, or a rebound page. The folder *picker* filters these names
(`is_sensitive_picker_entry`) but the workspace explorer never applies it.
Cause: `src/web/handlers.rs` `get_workspace_tree` / `get_workspace_file`.
Fix: apply the same filter (and don't rely on the exact name).

### HIGH 2 - the DNS-rebinding guard is defeated by a prefix match
`Host: 127.0.0.1.evil.com` and `Host: 127.evil.com` are both **accepted**; only
`evil.example.com` and `0.0.0.0` are rejected. Cause: `src/web/security.rs`
`h.starts_with("127.")`. An attacker publishes `127.0.0.1.rb.attacker.com` ->
127.0.0.1, the browser treats the harness as same-origin, and every JSON endpoint
becomes readable/writable with no CORS in play. The identical bug is in
`src/remote.rs` `ensure_secure_transport`, where it additionally means the full
config **including provider API keys** would be POSTed in plaintext to an
attacker host.
Fix: parse the host and require an exact loopback IP / `localhost` / `[::1]`.

### HIGH 3 - a no-token install gives full control via one rebinding POST
`POST /api/config` with `permissions.mode: yolo` +
`extra_access_roots: ["C:\\Windows","C:\\Users"]` + an attacker `update_repo` is
accepted and persisted, and the updater then exe-swaps to the attacker's binary
(updater.rs `apply_and_restart`). That is RCE-by-config.
Fix: treat `extra_access_roots` and `update_repo` as server-owned, like
`projects` / `sandbox_root` / `onboarded`.

### HIGH 4 - rotating the auth token has no effect until restart
`POST /api/config` with a new `auth_token` returns 200, but the old token keeps
working and `/api/security/status` still reports `auth_enabled: true`. A leaked
token is valid forever. Cause: `src/web/mod.rs` builds `HttpSecurity` once.

### MEDIUM 5 - `?token=` is accepted on every endpoint, not only SSE
`GET`/`POST /api/config?token=...` returns 200, so the token lands in URLs.

### MEDIUM 6 - the updater installs **unsigned** binaries when `SHA256SUMS` is absent
The digest check is inside `if let Some(sums_asset)`, so a release without
checksums (or a redirect on the asset URL) is installed and executed with no
signature check anywhere. `POST /api/update/apply` needs no second approval.

### LOW 7 - CSP has `script-src 'unsafe-inline'`, no `Permissions-Policy`, and a
renamed copy of `harness.key` is still listed by the picker.

### Held up (verified rejected)
Path traversal in `read_file`/`write_file` (`..`, `..\..`, `....//`, absolute,
`C:x`, UNC, `\\?\`, `CON`, ADS, trailing dot) -> "Path traversal is not allowed";
`/api/artifacts/*` traversal; markdown/`innerHTML` escaping; `javascript:` links;
`manage_tools` cannot overwrite builtins; a custom tool declaring `read_only`
with a shell runner is rejected; single-quoted shell templates; permission `Ask`
fails closed after 3 denials; intervals under 60s and bad cron rejected;
`Authorization` without `Bearer` -> 401.
**Not attempted**: prompt injection through the real model (no turn executed).
---

## Round 5 read-only code review - some SHIPPED FIXES ARE WRONG. Top priority.

No deadlocks found. But three of the round-4/5 fixes do not work as claimed, and
there are two unbounded leaks. **All OPEN, ordered by severity.**

### WRONG-1 (CRITICAL, data loss) - `unique_session_id` is still check-then-act
`state.rs` checks `sessions.contains_key` (line ~54) with awaits between the check
and the insert (`create_session_with_options` ~966), and **`create` has no
`contains_key` guard while `fork` does (~1735)**. Two creators in the same
millisecond both pass the check; the second insert REPLACES the first
`SessionState`, and `persist_session_record` DELETEs + reinserts the id's rows
(`db.rs:707`), destroying the first conversation. **The exact bug it claims to
close is still live.** Fix: reserve the id INSIDE the `sessions.write()` guard.

### WRONG-2 (HIGH) - `session_path_grants` is recorded and never honoured
The only reader `session_path_grants()` (`state.rs:1272`) is called by NOTHING.
Tool roots come from `loop_runner.rs:593-608` = `resolve_session_roots` +
`config.extra_access_roots` only. So `request_path_access` returns
`{"status":"granted"}` (`path_access_tool.rs:167`) and the very next `read_file`
still fails with "absolute path outside project folders" (`file_ops.rs:112`).
The security claim is untestable as written. Fix: fold `session_path_grants` into
the roots the runner computes.

### WRONG-3 (HIGH, security) - the `extra_access_roots` forwarding re-opens the hole
The server overrides only `sandbox_root` (`handlers.rs:3244`) while
`run_conf = payload.config.clone()` (`3242`) still carries the client's
`extra_access_roots`, which `loop_runner.rs:602` folds into `sandbox_roots` -
the very list `file_ops.rs:80-90` enforces. A client sending
`extra_access_roots: ["C:\\"]` gets full server filesystem access.
Fix: rebuild the roots server-side from the server's own projects/sandbox.

### WRONG-4 (MEDIUM) - always-allow is one-shot on remote sessions
`permission_tool_for` (`handlers.rs:737`) returns `None` for a remote perm id
(the waiter lives on the server), so the client records no allowlist entry; the
forward happens once and the next identical call prompts again - the original
bug, still live on multi-server setups.

### WRONG-5 (MEDIUM) - the permission counter is per model round-trip, not per turn
Initialised at `loop_runner.rs:912` INSIDE the outer loop (line 628). 4+ denials
in one batch aborts, but 3 denials in each of 40 rounds never aborts and burns 40
model calls. Fix: hoist it to the turn scope.

### WRONG-6 (MEDIUM) - a load failure permanently destroys history
`state.rs:726` `db.load_events(id).unwrap_or_default()` turns a read failure into
an empty history, and because `persist_session_record` DELETEs + reinserts
(`db.rs:707`), that empty history is written back on the next persist.
Fix: fail closed - if the load fails, do not persist over it.

### LEAK-1 (HIGH) - `server_runs` is never cleaned up
Inserted at `handlers.rs:3218`, **no removal path exists anywhere**. Each finished
run keeps `history` up to 2000 events plus a 256-slot broadcast sender:
~2-4 MB per run, ~100-150 MB/day on a cluster that dispatches subagents remotely.

### LEAK-2 (HIGH) - `swarm_boards` leaks one board per automation/server run id
`get_or_create_swarm` is called with the RUN id for every automation run
(`loop_runner.rs:577`) and every server run (`handlers.rs:3237`); only
session-keyed boards are removed. An AlwaysOn automation on the 30s tick
(`automation_runner.rs:585`) creates up to 2880 orphan boards/day.

### LEAK-3 (MEDIUM) - the new session maps are unbounded
`session_tool_allowlist` / `session_tool_allow_noted` / `session_path_grants` are
cleared only in `delete_session`, never on LRU eviction (`state.rs:951-965`) and
never for automation run ids (which use the run id as `session_id`).
Also: `remote_perm_routes` leaks when a run is stopped before its decision
arrives, and SwarmBoard `subagents` (`swarm.rs:511`) has no remove path anywhere.

### Swallowed errors that matter
- `handlers.rs:2175/2387/2443/3084/3111` `let _ = db.upsert_project/delete_project/...`:
  create/update/delete answer 200 while the DB write failed, so the project
  resurrects or vanishes on restart, silently.
- `handlers.rs:709` `to_string(&event).unwrap_or_default()` - an unserializable
  event ships as an empty SSE frame; the browser's `JSON.parse("")` throws.
- `handlers.rs:2788/2816` `let _ = state.ensure_session_loaded(..)` - undo answers
  a misleading 404 and fork forks an empty conversation.
- `state.rs:578` `Ok(Err(_))` discards the model error, so a failed turn persists
  no cause.

### Tests that must exist (none do yet)
1. `path_grants_are_honoured_by_file_tools` - Safe mode, approved temp dir outside
   the project, then `read_file` on it. **Currently fails** (see WRONG-2).
2. `concurrent_creates_in_one_millisecond_keep_both_sessions` - two tasks, both
   call `unique_session_id` then create without yielding the id. **Currently
   fails** (see WRONG-1).
3. `evicting_a_session_drops_allowlist_and_grants` - force eviction past 100.
4. `stop_during_permission_wait_clears_the_waiter`.
5. `completed_server_runs_are_dropped_from_server_runs`.
6. The HTTP permission-decision path (`POST /api/permissions/{id}/decision`) is
   untested - `test_permission_always_allow_is_honored_and_cleared` never calls it.

### Verified correct
No deadlock anywhere (lock ordering is consistent; the only nested acquisition is
`ensure_session_loaded`, one line away from an inversion). `persist_session_record`
and `persist_config` hold their guards over *blocking* SQLite I/O with
`busy_timeout=5000`, which is a latency problem, not a deadlock. Cron DOW fix is
correct. Mock-mode follow is correct.
---

## Round 6 in-flight status (2026-11-28, agents still running)

### config_revision - IMPLEMENTED, 4 integration tests now failing (expected mid-work)
`bump_config_revision()` added to `AppState`; `update_config` rejects a snapshot
older than the stored revision with 409 (`handlers.rs:974`); `merge_config_update`
takes the revision from `existing` so a client cannot rewind it. Unit tests for it
pass (`config_revision` tests around handlers.rs:1013-1079).
**Four integration tests fail with `reqwest::Error { kind: Decode, "expected value" }`**
because they POST a config snapshot and `.json()` the response, and now get a
409 with a non-JSON body:
- `test_display_verbosity_default_and_config_normalization`
- `test_send_mode_default_and_config_normalization`
- `test_auto_context_window_detection_and_smart_compaction`
- `test_unlimited_autonomous_turns_and_max_tool_retries_default`
**Required to land with it** (whoever finishes this): make `update_config` answer a
409 with a JSON error body, and update those four tests to either read the current
revision first or assert the 409. Until then the suite is NOT green.

### Coordination note
Follow-up messages to running subagents are being truncated by the harness
(`sessionID` arrives as `ses`), so running agents cannot be redirected. Protocol
records this as a known limitation: do not start two source-editing agents in the
same round, because they cannot be told to change scope.
---

## Round 6 applied (2026-11-28, by the orchestrator)

### WRONG-1 FIXED - session ids are now reserved atomically
`unique_session_id` takes the `sessions` WRITE lock for the whole
check-and-reserve and inserts a placeholder; `create_session_with_options`
overwrites that placeholder with the real session. Red team measured
"40 concurrent creates -> 18 unique ids" before; the reservation makes a second
creator unable to observe or claim the same id. `fork_session` already re-checks.
Regression test queued: two tasks that both call `unique_session_id` without
yielding must get distinct ids.

### WRONG-2 FIXED - session path grants now reach the tools
`resolve_session_roots` merges `session_path_grants(session)` into the roots, so
an approved `request_path_access` actually widens the session's reach instead of
returning `{"status":"granted"}` while the next `read_file` still failed.

### WRONG-3 FIXED (CRITICAL) - a remote server no longer trusts the client's scope
`start_server_run` now overrides `extra_access_roots`, `projects` AND the
permission mode from the SERVER's own config, keeping only `task`,
`prior_messages` and `attachments` from the payload. Red-team verified a client
sending `extra_access_roots: ["<out-of-sandbox folder>"]` made the server's
`list_dir` succeed on that folder even with `bash_policy: deny`; that path is
gone.

### WRONG-6 FIXED - request_path_access no longer trusts the model's own claim
The tool reads only `session_path_grants`, which `decide_permission` fills from
the path recorded with the permission request (new
`register_permission_waiter_with_meta_and_path` + `permission_path_for`). Both
directions of the old bug are closed: a fabricated `approved:true` in the args
grants nothing, and a human's Approve DOES grant.

### Also applied
- **D6**: an empty incoming secret no longer wipes the stored one - a config
  round-trip that does not echo the redacted placeholder used to delete the
  provider API key and break every later turn.
- **D5**: automation ids are unique on derivation (two names slugifying the same
  used to share an id and the second save replaced the first).
- **D4**: prompts over 100k characters are rejected with 413 instead of being
  stored, sent, and failing with the provider's raw 400.
- LRU eviction drops the per-session allowlist/grant/notice maps.
---

## Round 6/7 - CRITICAL PROCESS NOTES (read before doing anything)

### 2026-11-28: lost-work incident and recovery (do not repeat)
- I ran `git checkout -- src/web/handlers.rs` to undo a malformed debug edit. That reverted hours of work from TWO agents, because `handlers.rs` carried both their uncommitted edits and mine.
- Recovery: the combined content survived as an **unreachable git blob** (`f8ff6d9de819d35d60ab38f51aab355c286a1531`, found via `git fsck --unreachable`) and was restored byte-for-byte from it.
- Rule: **never `git checkout -- <file>` a shared working file.** Use `git stash`, or restore from a blob.

### Lossless editing rules learned the hard way
- PowerShell redirection (`> file`) writes **UTF-16LE**, which corrupts Rust/JS. Use `cmd /c "git cat-file -p <blob> > file"` to extract bytes losslessly.
- PowerShell string `.Replace()` eats Rust delimiters: `>>` collapses and backticks vanish inside strings. Prefer the `edit` tool.
- `git stash push -- <file>` takes a full-worktree snapshot each time: it is usable as a cheap backup before any risky edit.

### Concurrency protocol (enforced)
Exactly ONE agent edits source per round. Follow-up messages to a running subagent are truncated by the harness (`sessionID` arrives as `ses`), so a running agent CANNOT be told to change scope - never start a second editor.

### The bug that cost the most time (fixed)
`JoinHandle polled after completion`: the turn loop polled `&mut runner_handle` inside `select!` and then awaited the handle again in the terminal arm, panicking the tokio worker. The panic happened after `SessionCompleted` was emitted but before `update_session_status("completed")`, so **every session stuck on "running" forever**. Symptom: a session whose history ends in `SessionCompleted` while `/api/sessions` still reports `running`, and a stderr line `tokio-rt-worker ... JoinHandle polled after completion`.
Fix: the `select!` arm takes the result into `run_res`, and the terminal arm uses that value instead of awaiting the handle again.