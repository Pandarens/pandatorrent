//! Updating the application itself from GitHub releases.
//!
//! The updater reads `latest.json` published alongside each release and only
//! accepts artifacts signed with the project's private key, so an update cannot
//! be swapped out in transit. The release workflow signs them; the matching
//! public key lives in `tauri.conf.json`.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Progress of a running update, pushed to the UI.
pub const EVENT_PROGRESS: &str = "app-update:progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    pub available: bool,
    pub current_version: String,
    /// Version offered by the release, when there is one.
    pub version: Option<String>,
    /// Release notes, as written on the GitHub release.
    pub notes: Option<String>,
    pub published_at: Option<String>,
}

/// Asks GitHub whether a newer release exists.
#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> AppResult<AppUpdate> {
    let current = app.package_info().version.to_string();

    let updater = app
        .updater()
        .map_err(|e| AppError::msg(format!("обновления недоступны: {e}")))?;

    match updater.check().await {
        Ok(Some(update)) => Ok(AppUpdate {
            available: true,
            current_version: current,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            published_at: update.date.map(|d| d.to_string()),
        }),
        Ok(None) => Ok(AppUpdate {
            available: false,
            current_version: current,
            version: None,
            notes: None,
            published_at: None,
        }),
        Err(e) => Err(AppError::msg(format!(
            "не удалось проверить обновления: {e}"
        ))),
    }
}

/// Downloads and installs the update, then restarts the app.
///
/// Torrents are stopped first: the installer replaces the executable, and
/// leaving the engine mid-write through that is asking for a corrupt file.
#[tauri::command]
pub async fn app_update_install(app: AppHandle, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    let updater = app
        .updater()
        .map_err(|e| AppError::msg(format!("обновления недоступны: {e}")))?;

    let update = updater
        .check()
        .await
        .map_err(|e| AppError::msg(format!("не удалось проверить обновления: {e}")))?
        .ok_or_else(|| AppError::msg("обновлений нет"))?;

    let mut downloaded = 0usize;
    let progress_app = app.clone();

    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk;
                let percent = total
                    .map(|total| (downloaded as f64 / total as f64 * 100.0).round() as u32)
                    .unwrap_or(0);
                let _ = progress_app.emit(EVENT_PROGRESS, percent);
            },
            || {},
        )
        .await
        .map_err(|e| AppError::msg(format!("не удалось установить обновление: {e}")))?;

    state.player.stop();
    state.engine.shutdown().await;

    app.restart();
}
