## Fully automatic updates

Harness now updates itself with no user action:

- **On launch** it checks GitHub in the background, downloads the new binary
  while streaming **live progress** (bytes + a real progress bar), verifies the
  SHA-256 digest against `SHA256SUMS`, waits for any running
  conversation/automation/subagent to finish, then **replaces its own
  executable and relaunches** with the same arguments (`--port`, `--db`,
  `--sandbox` are preserved instead of being reset to defaults).
- The UI shows an update drawer with progress, and a full-screen
  "Restarting&hellip;" overlay that reloads the tab once the new binary answers.
- New settings toggles in **Settings &rarr; Memory & Runtime**:
  *Automatic Update Checks* and *Auto-Install Updates & Self-Restart* (both on by
  default), plus a "Check for updates" button and the running version.
- A relaunched process now **retries binding its port** for up to 30s, so the
  handover between the old and new process no longer looks like a crash.

## Also fixed

- **v1.1.1 was uninstallable-by-update**: its published `SHA256SUMS` had the
  name/hash order reversed (and CRLF endings), so every checksum verification
  failed with `harness.exe missing from SHA256SUMS`. The parser is now liberal
  about both orders and CRLF, and the asset on the v1.1.1 release has been
  corrected.
- The old auto-update path spawned the new binary but never exited the current
  process, leaving two instances and one unbound port. That path is replaced.
- `tools/release.ps1` now generates `SHA256SUMS` (hash-first, LF-only) and
  verifies every digest before publishing, with regression tests covering both
  formats.

## Assets

Verify with `SHA256SUMS`:

```powershell
(Get-FileHash harness.exe -Algorithm SHA256).Hash.ToLower()
```