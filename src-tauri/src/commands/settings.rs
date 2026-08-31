//! Settings read/write, plus the bits of state the settings screen shows.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::config::{AppConfig, RUTRACKER_MIRRORS};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdate {
    pub config: AppConfig,
    /// Engine-level options only take effect on a fresh session, so the UI
    /// tells the user when a restart is needed instead of silently ignoring
    /// what they changed.
    pub restart_required: bool,
}

#[tauri::command]
pub async fn settings_get(state: State<'_, Arc<AppState>>) -> AppResult<AppConfig> {
    Ok(state.config_snapshot())
}

#[tauri::command]
pub async fn settings_mirrors() -> AppResult<Vec<String>> {
    Ok(RUTRACKER_MIRRORS.iter().map(|s| s.to_string()).collect())
}

#[tauri::command]
pub async fn settings_set(
    state: State<'_, Arc<AppState>>,
    config: AppConfig,
) -> AppResult<SettingsUpdate> {
    let previous = state.config_snapshot();

    let restart_required = previous.network.listen_port != config.network.listen_port
        || previous.network.enable_dht != config.network.enable_dht
        || previous.network.enable_lsd != config.network.enable_lsd
        || previous.network.enable_upnp != config.network.enable_upnp
        || previous.network.download_limit_kbps != config.network.download_limit_kbps
        || previous.network.upload_limit_kbps != config.network.upload_limit_kbps
        || previous.network.max_peers_per_torrent != config.network.max_peers_per_torrent
        || previous.download_dir != config.download_dir;

    let tracker_changed = previous.rutracker.host != config.rutracker.host
        || previous.network.tracker_proxy != config.network.tracker_proxy;

    *state.config.write() = config.clone();
    state.save_config()?;

    if tracker_changed {
        // Rebuilds the HTTP clients; the session cookie is carried across.
        state
            .rutracker
            .reconfigure(&config.rutracker, config.network.tracker_proxy.as_deref())?;
    }

    // A film already on screen picks up volume and normalisation immediately.
    if let Err(e) = state.player.apply(&config.player) {
        tracing::warn!("could not apply player settings: {e}");
    }

    if !config.download_dir.as_os_str().is_empty() {
        let _ = std::fs::create_dir_all(&config.download_dir);
    }

    Ok(SettingsUpdate {
        config,
        restart_required,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub covers_dir: String,
    /// Folder holding the log files, for the "open log folder" button.
    pub logs_dir: String,
}

#[tauri::command]
pub async fn app_info(state: State<'_, Arc<AppState>>) -> AppResult<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.data_dir.to_string_lossy().to_string(),
        covers_dir: state.covers_dir().to_string_lossy().to_string(),
        logs_dir: state.data_dir.join("logs").to_string_lossy().to_string(),
    })
}

/// Opens the log folder in Explorer.
///
/// Reporting a fault is much easier with the file in front of you than with a
/// path to copy out by hand.
#[tauri::command]
pub async fn logs_open(app: AppHandle, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    let dir = state.data_dir.join("logs");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::msg(format!("не удалось создать папку журнала: {e}")))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::msg(format!("не удалось открыть папку журнала: {e}")))
}
