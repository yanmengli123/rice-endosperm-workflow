//! Optional, user-confirmed application updates.
//!
//! macOS downloads are verified by the Tauri updater before they are retained
//! in memory. Other platforms keep the existing GitHub Releases check/fallback.

use super::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Mutex as StdMutex;
use tauri::{ipc::Channel, AppHandle, State};

const RELEASES_URL: &str = "https://github.com/xuzhougeng/wisp-science/releases";

#[derive(Deserialize)]
pub(super) struct GithubRelease {
    pub(super) tag_name: String,
    pub(super) html_url: String,
    #[serde(default)]
    pub(super) body: String,
}

#[derive(Clone, Serialize)]
pub(super) struct UpdateCheck {
    pub(super) current_version: String,
    pub(super) latest_version: String,
    pub(super) update_available: bool,
    pub(super) release_url: String,
    /// Release notes / changelog markdown from the release manifest.
    pub(super) notes: String,
    /// Only macOS supports the signed in-app download/install flow for now.
    pub(super) install_supported: bool,
    /// A verified package is already waiting for the second confirmation.
    pub(super) downloaded: bool,
    /// Another window is currently downloading this package.
    pub(super) downloading: bool,
}

pub(super) fn update_check_from_release(
    current_version: &str,
    release: GithubRelease,
) -> Result<UpdateCheck, String> {
    let current = semver::Version::parse(current_version)
        .map_err(|error| format!("Invalid current version {current_version}: {error}"))?;
    let latest_text = release.tag_name.trim_start_matches(['v', 'V']);
    let latest = semver::Version::parse(latest_text).map_err(|error| {
        format!(
            "Invalid GitHub release version {}: {error}",
            release.tag_name
        )
    })?;

    Ok(UpdateCheck {
        current_version: current.to_string(),
        latest_version: latest.to_string(),
        update_available: latest > current,
        release_url: release.html_url,
        notes: release.body,
        install_supported: false,
        downloaded: false,
        downloading: false,
    })
}

struct PendingUpdate {
    update: tauri_plugin_updater::Update,
    bytes: Option<Vec<u8>>,
    downloading: bool,
}

#[derive(Default)]
pub(super) struct PendingAppUpdate(StdMutex<Option<PendingUpdate>>);

impl PendingUpdate {
    fn check(&self) -> UpdateCheck {
        UpdateCheck {
            current_version: self.update.current_version.clone(),
            latest_version: self.update.version.clone(),
            update_available: true,
            release_url: release_url(&self.update.version),
            notes: self.update.body.clone().unwrap_or_default(),
            install_supported: true,
            downloaded: self.bytes.is_some(),
            downloading: self.downloading,
        }
    }
}

fn release_url(version: &str) -> String {
    format!("{RELEASES_URL}/tag/v{version}")
}

fn install_is_blocked(
    running_turn: bool,
    awaiting_confirmation: bool,
    reviewing: bool,
    active_run: bool,
) -> bool {
    running_turn || awaiting_confirmation || reviewing || active_run
}

#[tauri::command]
pub(super) async fn check_for_updates(
    app: AppHandle,
    pending: State<'_, PendingAppUpdate>,
) -> Result<UpdateCheck, String> {
    if cfg!(target_os = "macos") {
        use tauri_plugin_updater::UpdaterExt;

        // Do not replace a package while another window downloads it, or after
        // its signature has been verified and it is awaiting installation.
        if let Some(check) = pending
            .0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|update| update.downloading || update.bytes.is_some())
            .map(PendingUpdate::check)
        {
            return Ok(check);
        }

        let update = app
            .updater()
            .map_err(|error| format!("Failed to configure the updater: {error}"))?
            .check()
            .await
            .map_err(|error| format!("Failed to check for a signed update: {error}"))?;

        if let Some(update) = update {
            let check = UpdateCheck {
                current_version: update.current_version.clone(),
                latest_version: update.version.clone(),
                update_available: true,
                release_url: release_url(&update.version),
                notes: update.body.clone().unwrap_or_default(),
                install_supported: true,
                downloaded: false,
                downloading: false,
            };
            *pending.0.lock().unwrap() = Some(PendingUpdate {
                update,
                bytes: None,
                downloading: false,
            });
            return Ok(check);
        } else {
            *pending.0.lock().unwrap() = None;
            let current_version = env!("CARGO_PKG_VERSION").to_string();
            return Ok(UpdateCheck {
                current_version: current_version.clone(),
                latest_version: current_version,
                update_available: false,
                release_url: RELEASES_URL.to_string(),
                notes: String::new(),
                install_supported: true,
                downloaded: false,
                downloading: false,
            });
        }
    }

    const LATEST_RELEASE_API: &str =
        "https://api.github.com/repos/xuzhougeng/wisp-science/releases/latest";

    let release = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|error| format!("Failed to create update client: {error}"))?
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::USER_AGENT, "wisp-science-update-check")
        .send()
        .await
        .map_err(|error| format!("Failed to check GitHub Releases: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub Releases returned an error: {error}"))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("Invalid response from GitHub Releases: {error}"))?;

    update_check_from_release(env!("CARGO_PKG_VERSION"), release)
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub(super) enum UpdateDownloadEvent {
    Started {
        content_length: Option<u64>,
    },
    Progress {
        chunk_length: u64,
    },
    /// Emitted only after the package signature has been verified.
    Verified,
}

#[tauri::command]
pub(super) async fn download_update(
    pending: State<'_, PendingAppUpdate>,
    on_event: Channel<UpdateDownloadEvent>,
) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("In-app update installation is currently available on macOS only.".into());
    }

    let update = {
        let mut pending = pending.0.lock().unwrap();
        let update = pending
            .as_mut()
            .ok_or_else(|| "Check for updates before downloading.".to_string())?;
        if update.bytes.is_some() {
            let _ = on_event.send(UpdateDownloadEvent::Verified);
            return Ok(());
        }
        if update.downloading {
            return Err("This update is already downloading in another window.".into());
        }
        update.downloading = true;
        update.update.clone()
    };

    let progress_events = on_event.clone();
    let mut started = false;
    let result = update
        .download(
            move |chunk_length, content_length| {
                if !started {
                    started = true;
                    let _ = progress_events.send(UpdateDownloadEvent::Started { content_length });
                }
                let _ = progress_events.send(UpdateDownloadEvent::Progress {
                    chunk_length: chunk_length as u64,
                });
            },
            || {},
        )
        .await;

    match result {
        Ok(bytes) => {
            if let Some(update) = pending.0.lock().unwrap().as_mut() {
                update.bytes = Some(bytes);
                update.downloading = false;
            }
            let _ = on_event.send(UpdateDownloadEvent::Verified);
            Ok(())
        }
        Err(error) => {
            if let Some(update) = pending.0.lock().unwrap().as_mut() {
                update.downloading = false;
            }
            Err(format!(
                "Update download or signature verification failed: {error}"
            ))
        }
    }
}

#[tauri::command]
pub(super) async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingAppUpdate>,
) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("In-app update installation is currently available on macOS only.".into());
    }

    let running_turn = !state.running_turns.lock().await.is_empty();
    let awaiting_confirmation = !state.awaiting_confirm.lock().unwrap().is_empty();
    let reviewing = !state.reviewing.lock().unwrap().is_empty();
    let active_run = !state
        .store
        .list_active_runs()
        .await
        .map_err(|error| format!("Failed to check active runs: {error}"))?
        .is_empty();
    let blocked = install_is_blocked(running_turn, awaiting_confirmation, reviewing, active_run);
    if blocked {
        return Err("Wait for every task and run to finish before installing the update.".into());
    }

    let mut ready = pending
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "Download and verify the update before installing.".to_string())?;
    let Some(bytes) = ready.bytes.take() else {
        *pending.0.lock().unwrap() = Some(ready);
        return Err("Download and verify the update before installing.".into());
    };

    if let Err(error) = ready.update.install(&bytes) {
        ready.bytes = Some(bytes);
        *pending.0.lock().unwrap() = Some(ready);
        return Err(format!("Failed to install the update: {error}"));
    }

    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_waits_for_every_kind_of_active_work() {
        assert!(!install_is_blocked(false, false, false, false));
        assert!(install_is_blocked(true, false, false, false));
        assert!(install_is_blocked(false, true, false, false));
        assert!(install_is_blocked(false, false, true, false));
        assert!(install_is_blocked(false, false, false, true));
    }

    #[test]
    fn download_events_keep_the_frontend_channel_shape() {
        assert_eq!(
            serde_json::to_value(UpdateDownloadEvent::Started {
                content_length: Some(42)
            })
            .unwrap(),
            serde_json::json!({
                "event": "started",
                "data": { "content_length": 42 }
            })
        );
    }
}
