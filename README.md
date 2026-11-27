# Harness

Ultra-resource-efficient agentic workspace IDE and distributed execution runtime written in Rust.

## Release Binaries

Three release executables are built from this crate:

| Binary | Role | Description |
| :--- | :--- | :--- |
| `harness` | **Server + Client (Unified)** | Full standalone workspace with SQLite persistence, embedded Web UI, local tool runtime, and remote subagent dispatch. |
| `harness-client` | **Standalone Client** | Lightweight frontend & conversation/history SQLite database that connects to one or more remote `harness-server` nodes. |
| `harness-server` | **Standalone Server** | Headless tool execution & subagent runner exposing `/api/server/*` endpoints without a Web UI or history DB. |

## Features

- **Multi-Provider Model Management**: Simultaneous credentials across OpenAI-compatible, NVIDIA NIM, OpenRouter, DeepSeek, Anthropic, and Ollama providers with provider-grouped search, context-window filtering, modality badges, and cascading thinking-effort selection.
- **Orchestrator & Multi-Profile Subagents**: Dedicated Orchestrator mode with customizable subagent profiles (e.g. Research, Coding, Reviewer) and per-profile model, thinking level, and execution-server routing (`local`, `auto`, or specific server node).
- **Projects & Knowledge-Only Conversations**: Nested project workspaces (with per-project default settings and an in-browser directory browser) separated from standalone knowledge-work conversations (where file/shell execution tools are disabled).
- **Client / Server Split**: Start a conversation on one execution server and spawn subagents across remote execution servers with SSE streaming, live steer/cancel, and permission forwarding.
- **SQLite Persistence**: WAL-mode SQLite database (`harness.db`) storing projects, servers, conversations, messages, and replayable events.
- **Multi-folder Projects**: a project can span several folders; the agent can read and write in all of them, and can request access to further folders with explicit approval.
- **Automations**: scheduled agent runs with `manual`, `interval`, `cron`, or `always-on` triggers. Standalone or project-bound, with an optional model override. Managed from the sidebar, the settings page, or the `manage_automations` tool.
- **Skills**: Built-in `code`, `orchestrator`, `reviewer`, `knowledge-work`, and `modify-harness` skills plus workspace skills in `.harness/skills/`. The agent can author new skills with the `create_skill` tool, and `/condense` distills the current session into a reusable skill.

## Security

The API can read configuration, execute tools, and browse the filesystem, so it is protected by default:

- **Loopback-only bind by default.** Binding to a non-loopback address refuses to start unless `HARNESS_AUTH_TOKEN` is set.
- **Optional bearer auth.** Set `HARNESS_AUTH_TOKEN` (or `auth_token` in config) to require `Authorization: Bearer <token>` on every `/api/*` request. The UI prompts for the token and remembers it locally.
- **DNS-rebinding guard.** On loopback binds, requests with a non-loopback `Host` header are rejected (HTTP 403).
- **No permissive CORS.** The UI is same-origin; cross-origin reads are denied.
- **Secrets are redacted in API responses.** `GET /api/config`, `/api/servers`, and `/api/mcp` return `__harness_redacted__` in place of stored keys; re-POSTing the redacted snapshot preserves the stored values.
- **Credentials are never accepted in URLs** for model listing (use `POST /api/models/remote`).
- **Artifacts** that could run script (HTML/SVG/XML/JS) are served as downloads with a sandboxing CSP.
- **Credentials are encrypted at rest.** Provider keys, search keys, MCP secrets and server tokens are stored in a separate `app_secrets` row encrypted with AES-256-GCM, never in the readable config JSON. Existing databases migrate automatically on first load.
- **Security headers** (`X-Frame-Options`, `Referrer-Policy`, `nosniff`, a same-origin CSP) are applied to every response.
- **No credentials over plaintext HTTP.** Remote execution refuses to send configuration to a non-loopback `http://` server.

### Encryption key

The AES-256 master key comes from `HARNESS_MASTER_KEY` (64 hex chars or base64 of 32 bytes) or, if unset, from `harness.key` next to the database (created automatically, `0600` on Unix, git-ignored).

> **Back up `harness.key` with `harness.db`.** Losing the key makes stored credentials unrecoverable; delete `app_secrets` and re-enter them if that happens.

## Auto-update

Releases are published on GitHub (`akagikay/Harness` by default, override with `update_repo` or `HARNESS_UPDATE_REPO`).

- **Notify only (default):** the app checks for a newer release at startup and the UI shows an "Update available" banner with release notes.
- **Install from the UI:** the banner's *Install & restart* button downloads the matching binary, verifies its SHA-256 against the release's `SHA256SUMS`, swaps it in and relaunches.
- **CLI:** `harness --check-update` reports status; `harness --update` installs and restarts.
- **Unattended:** set `auto_update_apply = true` to install automatically. Installing requires this explicit opt-in (`POST /api/update/apply` returns 403 otherwise).

Checksums are verified before installation; a size or digest mismatch aborts the update and leaves the current binary untouched.


> Keys are stored unencrypted in `harness.db`. Keep the sandbox directory private and prefer environment variables for credentials.
## Quick Start

### Unified Server + Client
```bash
harness --host 127.0.0.1 --port 8080 --sandbox /path/to/workspace
```

### Distributed Client + Multiple Execution Servers
```bash
# Start execution server 1
harness-server --host 127.0.0.1 --port 8081 --id server-1 --sandbox /path/to/repo-a

# Start execution server 2
harness-server --host 127.0.0.1 --port 8082 --id server-2 --sandbox /path/to/repo-b

# Start client UI connected to SQLite history DB
harness-client --host 127.0.0.1 --port 8080 --db ./harness.db
```

## Building from Source

```bash
cargo test
cargo build --release
```

Size-optimized release binaries are written to `target/release/`:
- `target/release/harness` (or `harness.exe`)
- `target/release/harness-client` (or `harness-client.exe`)
- `target/release/harness-server` (or `harness-server.exe`)
