//! Commands for the downloads view.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::models::{TorrentRecord, TorrentSource};
use crate::engine::{AddOptions, AddSource, AddedTorrent, TorrentDetails, TorrentProgress};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// What the downloads list renders: durable fields from the database joined
/// with live counters from the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentView {
    #[serde(flatten)]
    pub record: TorrentRecord,
    pub progress: Option<TorrentProgress>,
}

/// Merges database rows with engine stats. Torrents present in the engine but
/// not in the database (added out of band, e.g. restored from the session
/// file) are still surfaced so nothing goes invisible.
pub fn build_views(state: &AppState) -> AppResult<Vec<TorrentView>> {
    let records = state.db.list_torrents()?;
    let mut progress: std::collections::HashMap<String, TorrentProgress> = state
        .engine
        .progress_all()
        .into_iter()
        .map(|p| (p.info_hash.to_uppercase(), p))
        .collect();

    let mut views: Vec<TorrentView> = records
        .into_iter()
        .map(|record| {
            let progress = progress.remove(&record.info_hash.to_uppercase());
            TorrentView { record, progress }
        })
        .collect();

    for (hash, p) in progress {
        let name = p.name.clone().unwrap_or_else(|| fallback_name(&hash));
        views.push(TorrentView {
            record: TorrentRecord {
                info_hash: hash,
                name,
                output_folder: String::new(),
                total_bytes: p.total_bytes as i64,
                added_at: 0,
                completed_at: None,
                source: TorrentSource::File,
                topic_id: None,
            },
            progress: Some(p),
        });
    }

    Ok(views)
}

#[tauri::command]
pub async fn torrents_list(state: State<'_, Arc<AppState>>) -> AppResult<Vec<TorrentView>> {
    build_views(&state)
}

#[tauri::command]
pub async fn torrents_progress(
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<TorrentProgress>> {
    Ok(state.engine.progress_all())
}

#[tauri::command]
pub async fn torrent_details(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<TorrentDetails> {
    state.engine.details(&info_hash)
}

/// Adds a magnet link, an `http(s)` link to a `.torrent`, or a bare info hash.
#[tauri::command]
pub async fn torrent_add_url(
    state: State<'_, Arc<AppState>>,
    url: String,
    output_folder: Option<String>,
) -> AppResult<AddedTorrent> {
    add_url_with(&state, &url, output_folder).await
}

#[tauri::command]
pub async fn torrent_add_file(
    state: State<'_, Arc<AppState>>,
    path: String,
    output_folder: Option<String>,
) -> AppResult<AddedTorrent> {
    add_path_with(&state, &path, output_folder).await
}

/// The same as the command above, callable without Tauri's `State` — the file
/// association handler opens torrents before any window has asked for them.
pub async fn add_url_with(
    state: &AppState,
    url: &str,
    output_folder: Option<String>,
) -> AppResult<AddedTorrent> {
    let source = if url.starts_with("magnet:") {
        TorrentSource::Magnet
    } else {
        TorrentSource::Url
    };
    let added = state
        .engine
        .add(
            AddSource::Url(url.to_string()),
            AddOptions {
                output_folder,
                // Files already on disk are hashed and counted, the way any
                // torrent client behaves. Without this, adding a release that
                // is already downloaded started it again from nothing.
                overwrite: true,
                ..Default::default()
            },
        )
        .await?;
    register(state, &added, source, None, None)?;
    Ok(added)
}

/// Re-hashes a torrent's files against the piece list.
///
/// The manual counterpart to the check that happens when a torrent is added:
/// for when files were replaced, moved back, or repaired outside the
/// application and the figures no longer match what is on disk.
#[tauri::command]
pub async fn torrent_recheck(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<AddedTorrent> {
    let added = state.engine.recheck(&info_hash).await?;
    tracing::info!(%info_hash, "проверка файлов запущена");
    Ok(added)
}

pub async fn add_path_with(
    state: &AppState,
    path: &str,
    output_folder: Option<String>,
) -> AppResult<AddedTorrent> {
    let bytes = std::fs::read(path)?;
    let added = state
        .engine
        .add(
            AddSource::Bytes(bytes.clone()),
            AddOptions {
                output_folder,
                // Files already on disk are hashed and counted, the way any
                // torrent client behaves. Without this, adding a release that
                // is already downloaded started it again from nothing.
                overwrite: true,
                ..Default::default()
            },
        )
        .await?;
    register(state, &added, TorrentSource::File, None, Some(&bytes))?;
    Ok(added)
}

/// Opens a magnet link handed to the app from outside the UI.
pub async fn add_url(state: &AppState, url: &str) -> AppResult<String> {
    let added = add_url_with(state, url, None).await?;
    let name = added
        .name
        .clone()
        .unwrap_or_else(|| fallback_name(&added.info_hash));
    // Everything the app downloads gets a library card, however it arrived.
    let _ = state
        .db
        .upsert_library_item(&added.info_hash, &crate::library::clean_title(&name), None, "game");
    Ok(name)
}

/// Opens a `.torrent` file handed to the app from outside the UI.
pub async fn add_path(state: &AppState, path: &str) -> AppResult<String> {
    let added = add_path_with(state, path, None).await?;
    let name = added
        .name
        .clone()
        .unwrap_or_else(|| fallback_name(&added.info_hash));
    let install = install_dir(&added);
    let _ = state.db.upsert_library_item(
        &added.info_hash,
        &crate::library::clean_title(&name),
        Some(&install),
        "game",
    );
    Ok(name)
}

#[tauri::command]
pub async fn torrent_pause(state: State<'_, Arc<AppState>>, info_hash: String) -> AppResult<()> {
    state.engine.pause(&info_hash).await
}

#[tauri::command]
pub async fn torrent_resume(state: State<'_, Arc<AppState>>, info_hash: String) -> AppResult<()> {
    state.engine.resume(&info_hash).await
}

/// Removes a torrent. `delete_files` also wipes the downloaded content, and in
/// that case the library card goes away with it.
#[tauri::command]
pub async fn torrent_remove(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    delete_files: bool,
) -> AppResult<()> {
    // The engine may not have this torrent any more — a temporary stream that
    // was already reaped, or a leftover row from a previous run. That must not
    // stop the entry from being removed, which is what made the delete button
    // appear to do nothing.
    let engine_result = if delete_files {
        state.engine.delete(&info_hash).await
    } else {
        state.engine.forget(&info_hash).await
    };
    if let Err(e) = engine_result {
        tracing::warn!("engine could not remove {info_hash}: {e}");
    }

    if let Some(record) = state.db.get_torrent(&info_hash)? {
        if let Some(topic_id) = record.topic_id {
            state.db.delete_tracked_topic(topic_id)?;
        }
    }
    // Cascades to the library card.
    state.db.delete_torrent(&info_hash)?;
    Ok(())
}

#[tauri::command]
pub async fn torrent_set_files(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    files: Vec<usize>,
) -> AppResult<()> {
    state.engine.set_only_files(&info_hash, files).await
}

#[tauri::command]
pub async fn torrent_open_folder(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<()> {
    let record = state
        .db
        .get_torrent(&info_hash)?
        .ok_or(AppError::TorrentNotFound)?;
    open_in_explorer(&record.output_folder)
}

/// Opens a path in Explorer. Used by both the downloads and library views.
pub fn open_in_explorer(path: &str) -> AppResult<()> {
    std::process::Command::new("explorer")
        .arg(path)
        // Explorer returns a non-zero exit code even on success, so the status
        // is deliberately not checked.
        .spawn()
        .map_err(|e| AppError::msg(format!("не удалось открыть проводник: {e}")))?;
    Ok(())
}

/// Writes a freshly added torrent into the database.
pub fn register(
    state: &AppState,
    added: &AddedTorrent,
    source: TorrentSource,
    topic_id: Option<i64>,
    torrent_file: Option<&[u8]>,
) -> AppResult<()> {
    let name = added
        .name
        .clone()
        .unwrap_or_else(|| fallback_name(&added.info_hash));
    state.db.upsert_torrent(
        &added.info_hash,
        &name,
        &added.output_folder,
        added.total_bytes as i64,
        source,
        topic_id,
        torrent_file,
    )
}

/// Placeholder shown until a magnet resolves its metadata. Better than a dash:
/// the user can still tell two unnamed torrents apart.
pub fn fallback_name(info_hash: &str) -> String {
    format!("Торрент {}", &info_hash[..info_hash.len().min(8)])
}

/// Where the content of a torrent actually lands.
///
/// librqbit puts a multi-file torrent into a subfolder named after it, and a
/// single-file torrent straight into the output folder.
pub fn install_dir(added: &AddedTorrent) -> String {
    match (&added.name, added.files.len()) {
        (Some(name), n) if n > 1 => std::path::Path::new(&added.output_folder)
            .join(name)
            .to_string_lossy()
            .to_string(),
        _ => added.output_folder.clone(),
    }
}
