//! Fully automatic updates.
//!
//! [`spawn`] runs a background loop that:
//! 1. asks GitHub for the latest release,
//! 2. streams the new binary while publishing progress into
//!    [`AppState::update_status`] (so the UI can show a real progress bar),
//! 3. verifies the SHA-256 digest against `SHA256SUMS`,
//! 4. waits until no conversation, automation, subagent or remote run is busy,
//! 5. replaces the running executable and relaunches it with the *same*
//!    arguments.
//!
//! Any failure is recorded in the status (never panics, never blocks startup).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::state::AppState;
use crate::updater::{self, UpdateInfo, UpdatePhase};

/// How often to re-check once the first pass has finished.
const RECHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Give the server time to come up before the first network call.
const INITIAL_DELAY: Duration = Duration::from_secs(3);
/// Wait this long for running work to settle before installing.
const IDLE_GRACE: Duration = Duration::from_secs(30 * 60);

/// Kick off the background auto-update loop (standalone/UI mode only).
///
/// The repo/flags are read from the *live* config on every pass: `main` builds
/// `AppState::with_db(config)` from CLI/env defaults, and a stale local snapshot
/// would ignore whatever the user saved in Settings.
pub fn spawn(state: Arc<AppState>, current_version: &'static str) {
    tokio::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            let (repo, do_check, do_install) = {
                let conf = state.config.read().await;
                (
                    conf.update_repo.clone(),
                    conf.auto_update_check,
                    conf.auto_update_apply,
                )
            };
            if do_check {
                let _ = run_once(&state, &repo, current_version, do_install).await;
            }
            tokio::time::sleep(RECHECK_INTERVAL).await;
        }
    });
}

/// One check + (optionally) install pass. Returns `None` when no update exists.
async fn run_once(
    state: &AppState,
    repo: &str,
    current_version: &'static str,
    auto_install: bool,
) -> Option<UpdateInfo> {
    state
        .set_update_status(|s| {
            s.current_version = current_version.to_string();
            s.progress_phase(UpdatePhase::Checking, "Checking for updates...");
        })
        .await;

    let info = match updater::check(repo, current_version).await {
        Ok(info) => info,
        Err(e) => {
            state.set_update_status(|s| s.fail(e)).await;
            return None;
        }
    };

    if !info.available {
        // Running the newest release: forget any previous install marker so a
        // later release can be installed again.
        let stale = {
            let conf = state.config.read().await;
            !conf.last_auto_update_target.is_empty()
                && conf.last_auto_update_target != info.latest_version
        };
        if stale {
            let mut conf = state.config.write().await;
            conf.last_auto_update_target = String::new();
            drop(conf);
            state.persist_config().await;
        }
        state
            .set_update_status(|s| {
                s.phase = UpdatePhase::UpToDate;
                s.target_version = info.current_version.clone();
                s.message = "Up to date".into();
                s.error = None;
                s.percent = None;
                s.downloaded_bytes = 0;
                s.total_bytes = 0;
                s.touch();
            })
            .await;
        return None;
    }

    state
        .set_update_status(|s| {
            s.progress_phase(
                UpdatePhase::Available,
                format!("v{} available", info.latest_version),
            );
            s.target_version = info.latest_version.clone();
            s.html_url = info.html_url.clone();
            s.notes = info.notes.clone();
        })
        .await;

    if !auto_install {
        eprintln!(
            "[harness-update] v{} available at {} (set auto_update_apply = true to install automatically)",
            info.latest_version, info.html_url
        );
        return Some(info);
    }

    // Never re-install the same target forever: a release whose binary reports
    // an older internal version than its tag would otherwise loop endlessly.
    let already_installed = {
        let conf = state.config.read().await;
        conf.last_auto_update_target == info.latest_version
    };
    if already_installed {
        state
            .set_update_status(|s| {
                s.progress_phase(
                    UpdatePhase::Available,
                    format!("v{} available (awaiting a fixed release)", info.latest_version),
                );
                s.touch();
            })
            .await;
        return Some(info);
    }

    // Mark the target first, so even a crash mid-install cannot loop.
    {
        let mut conf = state.config.write().await;
        conf.last_auto_update_target = info.latest_version.clone();
        drop(conf);
        state.persist_config().await;
    }

    if let Err(e) = install(state, &info).await {
        state.set_update_status(|s| s.fail(e)).await;
    }
    Some(info)
}

/// Download (with live progress), verify, wait for idle, install, relaunch.
pub async fn install(state: &AppState, info: &UpdateInfo) -> Result<(), String> {
    let dest_dir = std::env::temp_dir().join("harness-update");

    state
        .set_update_status(|s| {
            s.progress_phase(
                UpdatePhase::Downloading,
                format!("Downloading v{}...", info.latest_version),
            );
            s.downloaded_bytes = 0;
            s.total_bytes = info.asset.as_ref().map(|a| a.size).unwrap_or(0);
            s.percent = None;
        })
        .await;

    // The download callback is sync and runs inside an async future, so it must
    // never await a tokio lock. Bytes are parked in a plain mutex and drained
    // by a short-lived poller task that stitches them into `update_status`.
    let tick = Arc::new(std::sync::Mutex::new((0u64, info.asset.as_ref().map(|a| a.size))));
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<(u64, Option<u64>)>();
    let tick_for_cb = Arc::clone(&tick);
    let shared_status = Arc::clone(&state.update_status);
    let download = updater::download_with_progress(info, Path::new(&dest_dir), move |done, total| {
        let mut guard = tick_for_cb.lock().unwrap_or_else(|e| e.into_inner());
        guard.0 = done;
        guard.1 = total.or(guard.1);
        let snapshot = *guard;
        let _ = ui_tx.send(snapshot);
    });
    let poller = tokio::spawn(async move {
        while let Some((done, total)) = ui_rx.recv().await {
            let mut guard = shared_status.write().await;
            guard.phase = UpdatePhase::Downloading;
            guard.downloaded_bytes = done;
            guard.total_bytes = total.unwrap_or(0);
            guard.percent = total
                .filter(|t| *t > 0)
                .map(|t| ((done * 100) / t).min(100) as u8);
            guard.touch();
        }
    });

    let staged = download.await;
    let _ = poller.await;
    let staged = match staged {
        Ok(p) => p,
        Err(e) => {
            state.set_update_status(|s| s.fail(e.clone())).await;
            return Err(e);
        }
    };

    state
        .set_update_status(|s| {
            s.progress_phase(UpdatePhase::Verifying, "Verifying checksum...");
        })
        .await;

    // Let running work finish so an install never truncates a live turn.
    wait_until_idle(state).await;
    state
        .set_update_status(|s| {
            s.progress_phase(UpdatePhase::Installing, "Installing update...");
        })
        .await;

    // Brief pause so the UI can paint the "installing" state before we exit.
    tokio::time::sleep(Duration::from_millis(600)).await;

    let backup = updater::apply_and_restart(&staged)?;
    eprintln!(
        "[harness-update] installed v{}; previous binary kept at {}",
        info.latest_version,
        backup.display()
    );
    state
        .set_update_status(|s| {
            s.phase = UpdatePhase::Restarting;
            s.message = format!("Updated to v{} - restarting...", info.latest_version);
            s.percent = Some(100);
            s.touch();
        })
        .await;

    // The staged copy is already installed; drop the temp file.
    let _ = std::fs::remove_file(&staged);

    // Hand over to the new process: this one MUST exit, otherwise it keeps
    // holding the listening port and the relaunched binary can never bind.
    // The short delay lets the UI poll the "Restarting" status once.
    tokio::time::sleep(Duration::from_millis(700)).await;
    std::process::exit(0);
}

/// Wait until nothing is running (capped by [`IDLE_GRACE`]).
async fn wait_until_idle(state: &AppState) {
    let deadline = std::time::Instant::now() + IDLE_GRACE;
    loop {
        if state.is_idle().await {
            return;
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("[harness-update] work still running after grace period; installing anyway");
            return;
        }
        state
            .set_update_status(|s| {
                s.phase = UpdatePhase::WaitingForIdle;
                s.message = "Waiting for running work to finish...".into();
                s.touch();
            })
            .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}