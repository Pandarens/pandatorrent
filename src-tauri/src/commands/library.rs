//! Commands backing the Steam-like library view.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::models::{LibraryItem, WishlistItem};
use crate::error::{AppError, AppResult};
use crate::library::{self, ExecutableCandidate};
use crate::state::AppState;

use super::torrents::open_in_explorer;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryFlag {
    Favorite,
    Hidden,
}

impl LibraryFlag {
    fn column(self) -> &'static str {
        match self {
            LibraryFlag::Favorite => "favorite",
            LibraryFlag::Hidden => "hidden",
        }
    }
}

#[tauri::command]
pub async fn library_list(
    state: State<'_, Arc<AppState>>,
    include_hidden: bool,
) -> AppResult<Vec<LibraryItem>> {
    state.db.list_library(include_hidden)
}

/// Creates a library card for a torrent that was not downloaded through the
/// tracker search — a dropped `.torrent` file or a magnet link.
#[tauri::command]
pub async fn library_add(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    title: Option<String>,
    category: Option<String>,
) -> AppResult<i64> {
    let record = state
        .db
        .get_torrent(&info_hash)?
        .ok_or(AppError::TorrentNotFound)?;

    let title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| library::clean_title(&record.name));

    // The engine knows whether the content sits in a subfolder.
    let install_dir = state
        .engine
        .details(&info_hash)
        .ok()
        .map(|d| {
            if d.files.len() > 1 {
                std::path::Path::new(&d.output_folder)
                    .join(d.name.unwrap_or_default())
                    .to_string_lossy()
                    .to_string()
            } else {
                d.output_folder
            }
        })
        .unwrap_or_else(|| record.output_folder.clone());

    state.db.upsert_library_item(
        &info_hash,
        &title,
        Some(&install_dir),
        category.as_deref().unwrap_or("game"),
    )
}

/// Finds launchable executables under the install folder, best guess first.
#[tauri::command]
pub async fn library_scan_executables(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<Vec<ExecutableCandidate>> {
    let item = find_item(&state, &info_hash)?;
    let dir = item
        .install_dir
        .clone()
        .ok_or_else(|| AppError::msg("папка установки неизвестна"))?;
    library::scan_executables(std::path::Path::new(&dir), &item.title)
}

#[tauri::command]
pub async fn library_set_exe(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    exe_path: Option<String>,
) -> AppResult<()> {
    state.db.set_library_exe(&info_hash, exe_path.as_deref())
}

#[tauri::command]
pub async fn library_set_title(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    title: String,
) -> AppResult<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AppError::msg("название не может быть пустым"));
    }
    state.db.set_library_title(&info_hash, title)
}

#[tauri::command]
pub async fn library_set_flag(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    flag: LibraryFlag,
    value: bool,
) -> AppResult<()> {
    state.db.set_library_flag(&info_hash, flag.column(), value)
}

#[tauri::command]
pub async fn library_launch(state: State<'_, Arc<AppState>>, info_hash: String) -> AppResult<()> {
    let item = find_item(&state, &info_hash)?;
    let exe = item
        .exe_path
        .ok_or_else(|| AppError::msg("исполняемый файл не выбран"))?;
    library::launch(&exe)?;
    // Playtime is not tracked yet; the timestamp alone drives "recently played".
    state.db.record_play(&info_hash, 0)
}

#[tauri::command]
pub async fn library_open_folder(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<()> {
    let item = find_item(&state, &info_hash)?;
    let dir = item
        .install_dir
        .ok_or_else(|| AppError::msg("папка установки неизвестна"))?;
    open_in_explorer(&dir)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverResult {
    /// Absolute path to the cached image; the UI turns it into an asset URL.
    pub path: String,
}

/// Downloads artwork for a card and caches it next to the database.
///
/// With no explicit URL, the first image of the tracker topic is used.
#[tauri::command]
pub async fn library_fetch_cover(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    image_url: Option<String>,
) -> AppResult<CoverResult> {
    let record = state
        .db
        .get_torrent(&info_hash)?
        .ok_or(AppError::TorrentNotFound)?;

    let url = match image_url {
        Some(u) => u,
        None => {
            let topic_id = record
                .topic_id
                .ok_or_else(|| AppError::msg("раздача не связана с трекером"))?;
            let topic = state.rutracker.topic(topic_id).await?;
            topic
                .images
                .into_iter()
                .next()
                .ok_or_else(|| AppError::msg("в теме не найдено изображений"))?
        }
    };

    let bytes = fetch_image(&url, state.proxy().as_deref()).await?;

    let dir = state.covers_dir();
    std::fs::create_dir_all(&dir)?;
    let ext = extension_for(&url, &bytes);
    let path = dir.join(format!("{}.{ext}", info_hash.to_uppercase()));
    std::fs::write(&path, &bytes)?;

    let path = path.to_string_lossy().to_string();
    state.db.set_library_cover(&info_hash, Some(&path))?;
    Ok(CoverResult { path })
}

async fn fetch_image(url: &str, proxy: Option<&str>) -> AppResult<Vec<u8>> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
    if let Some(p) = proxy.filter(|p| !p.trim().is_empty()) {
        builder = builder.proxy(
            reqwest::Proxy::all(p)
                .map_err(|e| AppError::msg(format!("Неверный адрес прокси: {e}")))?,
        );
    }
    let resp = builder.build()?.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::msg(format!(
            "не удалось скачать изображение: {}",
            resp.status()
        )));
    }
    let bytes = resp.bytes().await?.to_vec();
    if bytes.len() > 12 * 1024 * 1024 {
        return Err(AppError::msg("изображение слишком большое"));
    }
    Ok(bytes)
}

/// Picks a file extension from the magic bytes, falling back to the URL.
fn extension_for(url: &str, bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if url.to_lowercase().contains(".png") {
        "png"
    } else {
        "jpg"
    }
}

fn find_item(state: &AppState, info_hash: &str) -> AppResult<LibraryItem> {
    state
        .db
        .list_library(true)?
        .into_iter()
        .find(|i| i.info_hash.eq_ignore_ascii_case(info_hash))
        .ok_or_else(|| AppError::msg("запись библиотеки не найдена"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_image_type_from_magic_bytes() {
        assert_eq!(extension_for("x", &[0x89, b'P', b'N', b'G']), "png");
        assert_eq!(extension_for("x", &[0xFF, 0xD8, 0xFF, 0x00]), "jpg");
        assert_eq!(extension_for("cover.png", &[0, 0, 0, 0]), "png");
    }
}

// ---------------------------------------------------------------- wishlist
//
// The library is "what I have"; the wishlist is "what I mean to get". Keeping
// them in one screen but separate tables means a planned film needs no torrent,
// no folder and no engine entry until the user actually downloads it.

#[tauri::command]
pub async fn wishlist_list(state: State<'_, Arc<AppState>>) -> AppResult<Vec<WishlistItem>> {
    state.db.wishlist_list()
}

#[tauri::command]
pub async fn wishlist_add(
    state: State<'_, Arc<AppState>>,
    topic_id: i64,
    title: String,
    image_url: Option<String>,
    size_bytes: Option<i64>,
    category: Option<String>,
) -> AppResult<()> {
    if title.trim().is_empty() {
        return Err(AppError::msg("нужно название"));
    }
    state.db.wishlist_add(
        topic_id,
        title.trim(),
        image_url.as_deref(),
        size_bytes,
        category.as_deref().unwrap_or("movie"),
    )
}

#[tauri::command]
pub async fn wishlist_remove(state: State<'_, Arc<AppState>>, topic_id: i64) -> AppResult<()> {
    state.db.wishlist_remove(topic_id)
}
