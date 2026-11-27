//! GitHub-based auto-update.
//!
//! Flow:
//! 1. [`check`] asks the GitHub Releases API for the latest release and compares
//!    its tag with the running `CARGO_PKG_VERSION`.
//! 2. [`download`] / [`download_with_progress`] fetch the platform binary asset
//!    into a temp file and, when the release publishes `SHA256SUMS`, verify the
//!    digest (streamed so the UI can show progress).
//! 3. [`apply_and_restart`] swaps the running executable and relaunches it with
//!    the *same* command-line arguments, so a port/db override survives updates.
//!
//! Updates can be fully automatic: see [`super::auto_update`] and the
//! `auto_update_apply` config flag.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// Default repository that publishes harness releases.
pub const DEFAULT_REPO: &str = "akagikay/Harness";
const USER_AGENT: &str = "harness-updater";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub available: bool,
    pub html_url: String,
    pub notes: String,
    pub published_at: String,
    /// Asset matching the running executable, when the release has one.
    pub asset: Option<ReleaseAsset>,
    /// `SHA256SUMS` asset, when published.
    pub checksums: Option<ReleaseAsset>,
}

/// Progress of an in-flight update, mirrored for the UI (and logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    /// Nothing has happened yet.
    Idle,
    /// Asking GitHub whether a newer release exists.
    Checking,
    /// A newer release was found but not downloaded yet.
    Available,
    /// Streaming the new binary.
    Downloading,
    /// Comparing the downloaded digest with `SHA256SUMS`.
    Verifying,
    /// Downloaded and verified; waiting for running work to settle.
    WaitingForIdle,
    /// Swapping the executable on disk.
    Installing,
    /// The new binary has been launched; this process is on the way out.
    Restarting,
    /// Already running the newest release.
    UpToDate,
    /// The pipeline stopped with an error.
    Failed,
}

impl Default for UpdatePhase {
    fn default() -> Self {
        UpdatePhase::Idle
    }
}

/// Live, cloneable snapshot of the update pipeline (safe to expose to the UI).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub phase: UpdatePhase,
    pub current_version: String,
    pub target_version: String,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    /// 0-100 when the total size is known.
    #[serde(default)]
    pub percent: Option<u8>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub notes: String,
    /// True when the failure is temporary (network hiccup, GitHub 5xx) and the
    /// pipeline retries on its own. Lets the UI soften the wording.
    #[serde(default)]
    pub transient: bool,
    /// Wall-clock (unix seconds) of the last change.
    #[serde(default)]
    pub updated_at: u64,
}

impl UpdateStatus {
    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().timestamp().max(0) as u64;
    }

    pub fn progress_phase(&mut self, phase: UpdatePhase, message: impl Into<String>) {
        self.phase = phase;
        self.message = message.into();
        self.error = None;
        self.transient = false;
        self.touch();
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.phase = UpdatePhase::Failed;
        self.error = Some(error.into());
        self.message = self.error.clone().unwrap_or_default();
        self.transient = false;
        self.touch();
    }

    /// A failure the pipeline retries by itself (GitHub 5xx, rate limit,
    /// DNS/timeout). Rendered gently so a blip does not look like a crash.
    pub fn fail_transient(&mut self, error: impl Into<String>) {
        self.fail(error);
        self.transient = true;
    }
}

/// The file name of the running executable (`harness.exe` on Windows).
pub fn current_exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "harness.exe".to_string()
            } else {
                "harness".to_string()
            }
        })
}

/// Compare dotted versions; `true` when `candidate` is newer than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .trim_start_matches('V')
            // Ignore pre-release/build metadata.
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let a = parts(candidate);
    let b = parts(current);
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

/// How many times a transient failure is retried before giving up.
const MAX_ATTEMPTS: u32 = 4;
/// Retro of transient failures (doubles each attempt).
const FIRST_RETRY_DELAY: Duration = Duration::from_millis(900);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(20);

/// Statuses that are worth retrying: GitHub returns 502/503/504 and 429 under
/// load, and a single one must not abort an otherwise valid update.
fn is_retryable_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 511)
}

fn retryable_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect()
}

/// Describe a response status the way a human wants to read it.
fn status_phrase(status: reqwest::StatusCode) -> String {
    if status == reqwest::StatusCode::FORBIDDEN {
        return "403 Forbidden (GitHub rate limit or blocked)".to_string();
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return "429 Too Many Requests (GitHub rate limit)".to_string();
    }
    match status.canonical_reason() {
        Some(reason) => format!("{} {reason}", status.as_u16()),
        None => format!("{} (unexpected status)", status.as_u16()),
    }
}

/// GET with exponential backoff for transient network/server failures.
///
/// Returns the final response even when it is an error status, so callers can
/// decide what it means; only gives up after [`MAX_ATTEMPTS`] tries.
pub async fn get_with_retry(
    url: &str,
    accept: Option<&str>,
) -> Result<reqwest::Response, String> {
    let mut attempt = 0;
    let mut delay = FIRST_RETRY_DELAY;
    loop {
        attempt += 1;
        let mut req = client()?.get(url);
        if let Some(a) = accept {
            req = req.header("Accept", a);
        }
        let sent = req.send().await;
        let resp = match sent {
            Ok(resp) => resp,
            Err(e) if retryable_error(&e) && attempt < MAX_ATTEMPTS => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_RETRY_DELAY);
                continue;
            }
            Err(e) => return Err(format!("Request to GitHub failed: {e}")),
        };
        let status = resp.status();
        if is_retryable_status(status.as_u16()) && attempt < MAX_ATTEMPTS {
            let after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|s| Duration::from_secs(s.min(60)));
            tokio::time::sleep(after.unwrap_or(delay)).await;
            delay = (delay * 2).min(MAX_RETRY_DELAY);
            continue;
        }
        return Ok(resp);
    }
}

/// Query the latest GitHub release and compare it with the running version.
pub async fn check(repo: &str, current_version: &str) -> Result<UpdateInfo, String> {
    check_for_exe(repo, current_version, &current_exe_name()).await
}

/// Like [`check`] but targets an explicit asset file name. Falls back to
/// `harness.exe` when the running binary was renamed (e.g. a test harness).
pub async fn check_for_exe(
    repo: &str,
    current_version: &str,
    exe_name: &str,
) -> Result<UpdateInfo, String> {
    let repo = if repo.trim().is_empty() { DEFAULT_REPO } else { repo.trim() };
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = get_with_retry(&url, Some("application/vnd.github+json"))
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("No published releases found for {repo}"));
    }
    if !resp.status().is_success() {
        return Err(format!(
            "Update check failed: GitHub API returned {} after {MAX_ATTEMPTS} attempts",
            status_phrase(resp.status())
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid GitHub response: {e}"))?;

    let tag = body
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let latest = tag.trim_start_matches('v').to_string();
    let wanted = exe_name.to_ascii_lowercase();
    // Prefer the exact running binary; otherwise fall back to the unified one.
    let fallback = "harness.exe";

    let mut asset = None;
    let mut fallback_asset = None;
    let mut checksums = None;
    if let Some(items) = body.get("assets").and_then(|v| v.as_array()) {
        for item in items {
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let url = item
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            if url.is_empty() {
                continue;
            }
            let lowered = name.to_ascii_lowercase();
            if lowered == wanted {
                asset = Some(ReleaseAsset {
                    name: name.to_string(),
                    url: url.to_string(),
                    size,
                });
            } else if lowered == fallback {
                fallback_asset = Some(ReleaseAsset {
                    name: name.to_string(),
                    url: url.to_string(),
                    size,
                });
            } else if lowered == "sha256sums" || lowered == "sha256sums.txt" {
                checksums = Some(ReleaseAsset {
                    name: name.to_string(),
                    url: url.to_string(),
                    size,
                });
            }
        }
    }
    let asset = asset.or(fallback_asset);

    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        available: !latest.is_empty() && is_newer(&latest, current_version),
        latest_version: latest,
        html_url: body
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        notes: body
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect(),
        published_at: body
            .get("published_at")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        asset,
        checksums,
    })
}

/// Expected digest for `asset_name` from a `SHA256SUMS` file.
///
/// Deliberately liberal about format, because a mismatched release file must
/// never block an update: accepts `sha256sum` order (`<hex>  <name>`), binary
/// mode (`<hex> *<name>`), reversed order (`<name> <hex>`), and CRLF endings.
fn digest_for(sums: &str, asset_name: &str) -> Option<String> {
    fn is_hex(s: &str) -> bool {
        let s = s.trim_start_matches('*');
        s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
    }
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let first = parts.next()?;
        let second = parts.next()?;
        // Determine which token is the digest and which is the file name.
        let (hex, name) = if is_hex(first) {
            (first, second)
        } else if is_hex(second) {
            (second, first)
        } else {
            continue;
        };
        let name = name.trim_start_matches('*');
        if name.eq_ignore_ascii_case(asset_name) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

/// Download the release binary to `dest_dir`, verifying its checksum when the
/// release publishes `SHA256SUMS`. Returns the written path.
pub async fn download(info: &UpdateInfo, dest_dir: &Path) -> Result<PathBuf, String> {
    download_with_progress(info, dest_dir, |_, _| {}).await
}

/// Like [`download`], but streams the payload so `on_progress(done, total)`
/// can drive a progress bar. The digest is computed incrementally and verified
/// against `SHA256SUMS` before the file is swapped into place.
pub async fn download_with_progress<F>(
    info: &UpdateInfo,
    dest_dir: &Path,
    mut on_progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(u64, Option<u64>),
{
    let asset = info
        .asset
        .as_ref()
        .ok_or_else(|| format!("Release {} has no asset named {}", info.latest_version, current_exe_name()))?;
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let resp = get_with_retry(&asset.url, None)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    let status = resp.status();
    let resp = resp.error_for_status().map_err(|_| {
        format!(
            "Download failed: GitHub returned {} after {MAX_ATTEMPTS} attempts",
            status_phrase(status)
        )
    })?;

    let total = if asset.size > 0 {
        Some(asset.size)
    } else {
        resp.content_length()
    };

    let dest = dest_dir.join(&asset.name);
    let part = dest_dir.join(format!("{}.part", asset.name));
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| format!("Failed to create update file: {e}"))?;
    let mut hasher = Sha256::new();
    let mut done: u64 = 0;
    let mut stream = resp.bytes_stream();
    on_progress(0, total);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download failed: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write update: {e}"))?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        on_progress(done, total);
    }
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush update: {e}"))?;

    // Guard against a truncated or swapped payload.
    if asset.size > 0 && done != asset.size {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!(
            "Downloaded size {done} does not match published size {}",
            asset.size
        ));
    }

    if let Some(sums_asset) = &info.checksums {
        let sums = get_with_retry(&sums_asset.url, None)
            .await
            .map_err(|e| format!("Checksum download failed: {e}"))?
            .text()
            .await
            .map_err(|e| format!("Checksum download failed: {e}"))?;
        match digest_for(&sums, &asset.name) {
            Some(expected) => {
                let actual = hex::encode(hasher.finalize());
                if actual != expected {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(format!(
                        "Checksum mismatch for {}: expected {expected}, got {actual}",
                        asset.name
                    ));
                }
            }
            None => {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(format!("{} missing from SHA256SUMS", asset.name));
            }
        }
    }

    // Only now is the payload trusted; publish it under its final name.
    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| format!("Failed to stage update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dest)
}

/// The arguments this process was started with, minus the executable path.
///
/// Restarting without them would silently drop `--port`, `--db`, `--sandbox`,
/// `--mode`, ... and the updated process would come up somewhere else.
pub fn relaunch_args() -> Vec<String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Our own flags are one-shot: never replay them into the child process.
    args.retain(|a| a != "--update" && a != "--check-update");
    args
}

/// Spawn a fresh copy of `exe` (defaults to the current binary) with the given
/// arguments, detached from this process. Returns the child's PID.
pub fn spawn_detached(exe: Option<&Path>, args: &[String]) -> Result<u32, String> {
    let exe = match exe {
        Some(p) => p.to_path_buf(),
        None => std::env::current_exe().map_err(|e| format!("Cannot locate current exe: {e}"))?,
    };
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Don't flash a console window when a GUI-launched harness restarts.
        cmd.creation_flags(0x0800_0000);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch {}: {e}", exe.display()))?;
    Ok(child.id())
}

/// Remove stale `*.old-*` backups next to `current` (best effort, keeps at most
/// the newest one).
fn prune_old_backups(current: &Path) {
    let Some(dir) = current.parent() else { return };
    let stem = current.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let prefix = format!("{stem}.old-");
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut olds: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    if olds.len() <= 1 {
        return;
    }
    olds.sort();
    let keep = olds.pop();
    for old in olds {
        let _ = std::fs::remove_file(old);
    }
    drop(keep);
}

/// Replace the running executable with `new_exe` and relaunch.
///
/// On Windows a running executable cannot be overwritten but *can* be renamed,
/// so the current binary is moved aside first. The caller should exit right
/// after this returns `Ok`.
pub fn apply_and_restart(new_exe: &Path) -> Result<PathBuf, String> {
    let args = relaunch_args();
    apply_and_restart_with_args(new_exe, &args)
}

pub fn apply_and_restart_with_args(new_exe: &Path, args: &[String]) -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| format!("Cannot locate current exe: {e}"))?;
    if !new_exe.is_file() {
        return Err(format!("Update file not found: {}", new_exe.display()));
    }

    // Relaunch from a temp copy only if it equals the current binary (a caller
    // may hand us a differently named download).
    let install_target = current.clone();

    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let stem = current
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "harness.exe".to_string());
    let backup = current
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}.old-{stamp}"));
    // Move the running binary aside (allowed on Windows even while executing).
    std::fs::rename(&install_target, &backup)
        .map_err(|e| format!("Failed to move current executable aside: {e}"))?;

    if let Err(e) = std::fs::copy(new_exe, &install_target) {
        // Roll back so the user is not left without a binary.
        let _ = std::fs::rename(&backup, &install_target);
        return Err(format!("Failed to install update: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&install_target, std::fs::Permissions::from_mode(0o755));
    }
    prune_old_backups(&install_target);

    // Relaunch the updated binary with the *same* arguments (port, db, sandbox).
    spawn_detached(Some(&install_target), args)
        .map_err(|e| format!("Update installed but restart failed: {e}"))?;

    Ok(backup)
}

/// Bind the UI listener, retrying for a while. The old process may still be
/// shutting down (or the freshly relaunched one may start before its parent
/// released the port), which used to make an update look like a crash.
pub async fn bind_with_retry(addr: std::net::SocketAddr, timeout: Duration) -> std::io::Result<tokio::net::TcpListener> {
    let deadline = std::time::Instant::now() + timeout;
    let mut attempt = 0;
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                if std::time::Instant::now() >= deadline || !is_addr_in_use(&e) {
                    return Err(e);
                }
                attempt += 1;
                if attempt == 1 {
                    println!("Waiting for port {addr} to be released...");
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

fn is_addr_in_use(err: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        err.raw_os_error() == Some(10048) // WSAEADDRINUSE
    }
    #[cfg(not(windows))]
    {
        err.kind() == std::io::ErrorKind::AddrInUse
    }
}

/// Convenience: check, download to `dest_dir`, and install.
pub async fn update_now(repo: &str, current_version: &str, dest_dir: &Path) -> Result<UpdateInfo, String> {
    let info = check(repo, current_version).await?;
    if !info.available {
        return Ok(info);
    }
    let path = download(&info, dest_dir).await?;
    apply_and_restart(&path)?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("v0.4.0", "0.3.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("garbage", "0.3.0"));
        // pre-release suffixes compare on the numeric core
        assert!(!is_newer("0.3.0-beta", "0.3.0"));
    }

    #[test]
    fn parses_sha256sums_formats() {
        let sums = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  harness.exe\n\
                    e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 *harness-server.exe\n";
        assert_eq!(
            digest_for(sums, "harness.exe").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest_for(sums, "harness-server.exe").unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(digest_for(sums, "missing.exe").is_none());
    }

    /// A release file written as "<name> <hex>" (as v1.1.1 accidentally shipped)
    /// must still resolve - otherwise no update could ever install.
    #[test]
    fn parses_name_then_hash_order() {
        let sums = "harness.exe DD991C8E1B19DF15BE0039FEF4941A7882082CC1BCDC5925028F9F57653750B3\n\
                    harness-server.exe E8C73A077FEBBC4A3E68B5BE5B1383223B1381363608D389553EF1F912A68A37\n";
        assert_eq!(
            digest_for(sums, "harness.exe").unwrap(),
            "dd991c8e1b19df15be0039fef4941a7882082cc1bcdc5925028f9f57653750b3"
        );
        assert_eq!(
            digest_for(sums, "harness-server.exe").unwrap(),
            "e8c73a077febbc4a3e68b5be5b1383223b1381363608d389553ef1f912a68a37"
        );
    }

    /// CRLF endings (what `sha256sum | Out-File` produces on Windows) must work.
    #[test]
    fn parses_crlf_and_uppercase_digests() {
        let sums = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD  harness.exe\r\n";
        assert_eq!(
            digest_for(sums, "harness.exe").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// A file that is not a digest list must not be mistaken for one.
    #[test]
    fn rejects_non_digest_lines() {
        assert!(digest_for("not a digest at all\n", "harness.exe").is_none());
        // too short / wrong character set
        assert!(digest_for("abc123  harness.exe\n", "harness.exe").is_none());
    }

    #[test]
    fn exe_name_is_non_empty() {
        assert!(!current_exe_name().is_empty());
    }

    #[test]
    fn relaunch_args_drop_one_shot_flags() {
        // `cargo test` passes its own args, so just assert the filter behaves.
        let args: Vec<String> = vec!["--port".into(), "8099".into(), "--db".into(), "x.db".into()];
        let mut kept: Vec<String> = args
            .iter()
            .filter(|a| *a != "--update" && *a != "--check-update")
            .cloned()
            .collect();
        assert_eq!(kept, vec!["--port", "8099", "--db", "x.db"]);
        kept.retain(|a| a != "--update" && a != "--check-update");
        assert_eq!(kept.len(), 4);
    }

    #[test]
    fn status_tracks_progress() {
        let mut st = UpdateStatus::default();
        assert_eq!(st.phase, UpdatePhase::Idle);
        st.progress_phase(UpdatePhase::Downloading, "fetching harness.exe");
        assert_eq!(st.message, "fetching harness.exe");
        assert!(st.error.is_none());
        assert!(st.updated_at > 0);
        st.fail("network");
        assert_eq!(st.phase, UpdatePhase::Failed);
        assert_eq!(st.error.as_deref(), Some("network"));
        // Failed stays sticky until an explicit progress phase clears it.
        st.progress_phase(UpdatePhase::Checking, "retry");
        assert!(st.error.is_none());
    }

    #[tokio::test]
    async fn bind_with_retry_accepts_a_free_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let bound = bind_with_retry(addr, Duration::from_secs(3)).await;
        assert!(bound.is_ok(), "a free port must bind immediately");
    }

    #[tokio::test]
    async fn bind_with_retry_gives_up_on_a_busy_port() {
        // Hold a port, then assert the retry loop times out instead of hanging.
        let deadline_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = deadline_listener.local_addr().unwrap().port();
        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let started = std::time::Instant::now();
        let res = bind_with_retry(addr, Duration::from_millis(400)).await;
        assert!(res.is_err(), "an in-use port must fail after the timeout");
        assert!(is_addr_in_use(res.as_ref().err().unwrap()));
        assert!(started.elapsed() >= Duration::from_millis(350));
        drop(deadline_listener);
    }

    #[test]
    fn relaunch_args_keep_runtime_flags_and_get_them_back() {
        let args = vec![
            "--port".to_string(),
            "8099".to_string(),
            "--db".to_string(),
            "harness.db".to_string(),
            "--update".to_string(),
        ];
        let kept: Vec<String> = args
            .iter()
            .filter(|a| *a != "--update" && *a != "--check-update")
            .cloned()
            .collect();
        assert_eq!(kept, vec!["--port", "8099", "--db", "harness.db"]);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Hits the real GitHub API. Run with:
    /// `cargo test --lib live_tests -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "requires network access to api.github.com"]
    async fn live_release_is_discoverable_with_asset_and_checksums() {
        let info = check_for_exe(DEFAULT_REPO, "0.0.1", "harness.exe")
            .await
            .expect("check should succeed");
        assert!(info.available, "0.0.1 should be older than the latest release");
        assert!(!info.latest_version.is_empty());
        let asset = info.asset.as_ref().expect("release must publish the platform binary");
        assert_eq!(asset.name, "harness.exe");
        assert!(asset.size > 0);
        let sums = info.checksums.as_ref().expect("release must publish SHA256SUMS");
        assert!(sums.url.contains("SHA256SUMS"));
    }
}