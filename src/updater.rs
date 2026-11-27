//! GitHub-based auto-update.
//!
//! Flow:
//! 1. [`check`] asks the GitHub Releases API for the latest release and compares
//!    its tag with the running `CARGO_PKG_VERSION`.
//! 2. [`download`] fetches the platform binary asset into a temp file and, when
//!    the release publishes `SHA256SUMS`, verifies the digest.
//! 3. [`apply_and_restart`] swaps the running executable and relaunches.
//!
//! Updates are never applied implicitly: the caller must opt in
//! (`auto_update_apply` config, `--update`, or `POST /api/update/apply`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

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
    let resp = client()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("No published releases found for {repo}"));
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
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
fn digest_for(sums: &str, asset_name: &str) -> Option<String> {
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Formats: "<hex>  <name>" (sha256sum) or "<hex> *<name>" (binary mode).
        let mut it = line.split_whitespace();
        let hex = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        if name.eq_ignore_ascii_case(asset_name) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

/// Download the release binary to `dest_dir`, verifying its checksum when the
/// release publishes `SHA256SUMS`. Returns the written path.
pub async fn download(info: &UpdateInfo, dest_dir: &Path) -> Result<PathBuf, String> {
    let asset = info
        .asset
        .as_ref()
        .ok_or_else(|| format!("Release {} has no asset named {}", info.latest_version, current_exe_name()))?;
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let bytes = client()?
        .get(&asset.url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    // Guard against a truncated or swapped payload.
    if asset.size > 0 && bytes.len() as u64 != asset.size {
        return Err(format!(
            "Downloaded size {} does not match published size {}",
            bytes.len(),
            asset.size
        ));
    }

    if let Some(sums_asset) = &info.checksums {
        let sums = client()?
            .get(&sums_asset.url)
            .send()
            .await
            .map_err(|e| format!("Checksum download failed: {e}"))?
            .text()
            .await
            .map_err(|e| format!("Checksum download failed: {e}"))?;
        match digest_for(&sums, &asset.name) {
            Some(expected) => {
                let actual = crate::crypto::sha256_hex(&bytes);
                if actual != expected {
                    return Err(format!(
                        "Checksum mismatch for {}: expected {expected}, got {actual}",
                        asset.name
                    ));
                }
            }
            None => return Err(format!("{} missing from SHA256SUMS", asset.name)),
        }
    }

    let dest = dest_dir.join(&asset.name);
    std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dest)
}

/// Replace the running executable with `new_exe` and relaunch.
///
/// On Windows a running executable cannot be overwritten but *can* be renamed,
/// so the current binary is moved aside first. The caller should exit right
/// after this returns `Ok`.
pub fn apply_and_restart(new_exe: &Path) -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| format!("Cannot locate current exe: {e}"))?;
    if !new_exe.is_file() {
        return Err(format!("Update file not found: {}", new_exe.display()));
    }

    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let backup = current.with_extension(format!("old-{stamp}"));
    // Move the running binary aside (allowed on Windows).
    std::fs::rename(&current, &backup)
        .map_err(|e| format!("Failed to move current executable aside: {e}"))?;

    if let Err(e) = std::fs::copy(new_exe, &current) {
        // Roll back so the user is not left without a binary.
        let _ = std::fs::rename(&backup, &current);
        return Err(format!("Failed to install update: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o755));
    }

    // Relaunch the updated binary.
    std::process::Command::new(&current)
        .spawn()
        .map_err(|e| format!("Update installed but restart failed: {e}"))?;

    Ok(backup)
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
                    d3b07384d113edec49eaa6238ad5ff00  *harness-server.exe\n";
        assert_eq!(
            digest_for(sums, "harness.exe").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest_for(sums, "harness-server.exe").unwrap(),
            "d3b07384d113edec49eaa6238ad5ff00"
        );
        assert!(digest_for(sums, "missing.exe").is_none());
    }

    #[test]
    fn exe_name_is_non_empty() {
        assert!(!current_exe_name().is_empty());
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