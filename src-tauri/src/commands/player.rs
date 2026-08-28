//! Commands for watching a release while it is still downloading.
//!
//! Two entry points: [`player_play`] for something already in the library, and
//! [`player_watch_topic`] for "just show me this", which streams into a scratch
//! folder and cleans up afterwards. Both hand mpv the whole season as a
//! playlist so episodes roll on by themselves.

use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, State};

use crate::db::models::{TorrentSource, WatchHistoryItem};
use crate::engine::{AddOptions, AddSource, TorrentFileEntry};
use crate::error::{AppError, AppResult};
use crate::player::{self, Playback, PlayerStatus};
use crate::state::{AppState, NowPlaying, TempWatch};

/// How long to wait for a freshly added torrent to become streamable.
const READY_TIMEOUT: Duration = Duration::from_secs(90);

#[tauri::command]
pub async fn player_status(state: State<'_, Arc<AppState>>) -> AppResult<PlayerStatus> {
    Ok(state.player.status())
}

/// Live position and playlist state, plus how fast the file behind it is
/// downloading — for a stream that matters as much as the timecode does.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackView {
    #[serde(flatten)]
    pub playback: Playback,
    /// Download speed of the torrent being streamed, bytes per second.
    pub download_speed_bps: Option<u64>,
    pub peers: Option<u32>,
    /// Bytes of the current episode already on disk.
    pub file_downloaded: Option<u64>,
    pub file_total: Option<u64>,
    /// The next episode is already fully downloaded.
    pub next_ready: bool,
}

#[tauri::command]
pub async fn player_playback(state: State<'_, Arc<AppState>>) -> AppResult<Option<PlaybackView>> {
    let Some(playback) = state.player.playback() else {
        return Ok(None);
    };

    let context = state.now_playing.lock().clone();
    let mut view = PlaybackView {
        download_speed_bps: None,
        peers: None,
        file_downloaded: None,
        file_total: None,
        next_ready: false,
        playback,
    };

    if let Some(context) = context {
        if let Ok(progress) = state.engine.progress_one(&context.info_hash) {
            view.download_speed_bps = Some(progress.download_speed_bps);
            view.peers = Some(progress.peers_live);
        }
        if let (Ok(details), Ok(done)) = (
            state.engine.details(&context.info_hash),
            state.engine.file_progress(&context.info_hash),
        ) {
            let length = |id: usize| {
                details
                    .files
                    .iter()
                    .find(|f| f.index == id)
                    .map(|f| f.length)
                    .unwrap_or(0)
            };
            let position = view.playback.playlist_pos.unwrap_or(0);

            if let Some(&file_id) = context.files.get(position) {
                view.file_downloaded = done.get(file_id).copied();
                view.file_total = Some(length(file_id));
            }
            if let Some(&next_id) = context.files.get(position + 1) {
                let total = length(next_id);
                view.next_ready =
                    total > 0 && done.get(next_id).copied().unwrap_or(0) >= total;
            }
        }
    }

    Ok(Some(view))
}

/// Video files inside a torrent, in natural episode order.
#[tauri::command]
pub async fn player_video_files(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<Vec<TorrentFileEntry>> {
    Ok(video_files(&state, &info_hash)?)
}

fn video_files(state: &AppState, info_hash: &str) -> AppResult<Vec<TorrentFileEntry>> {
    let details = state.engine.details(info_hash)?;
    let mut files: Vec<TorrentFileEntry> = details
        .files
        .into_iter()
        .filter(|f| player::is_video_file(&f.name))
        .collect();
    files.sort_by(|a, b| natural_key(&a.name).cmp(&natural_key(&b.name)));
    Ok(files)
}

/// Sort key that orders `Серия 2` before `Серия 10`.
///
/// Plain lexicographic order puts episode 10 before episode 2, which makes a
/// season play in the wrong order — the one thing a playlist must get right.
fn natural_key(name: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            let mut number = c.to_digit(10).unwrap() as u64;
            while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                number = number.saturating_mul(10).saturating_add(d as u64);
                chars.next();
            }
            parts.push(NaturalPart::Number(number));
        } else {
            let mut text = c.to_lowercase().to_string();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    break;
                }
                text.push_str(&next.to_lowercase().to_string());
                chars.next();
            }
            parts.push(NaturalPart::Text(text));
        }
    }
    parts
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum NaturalPart {
    // Numbers sort before text at the same position, which keeps names like
    // "01.mkv" and "extras.mkv" in a sensible order.
    Number(u64),
    Text(String),
}

/// Starts playback of a torrent already known to the app.
#[tauri::command]
pub async fn player_play(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
    file_id: usize,
) -> AppResult<()> {
    ensure_player_available(&state)?;

    // A torrent that is not yet live cannot be streamed from, and launching mpv
    // early leaves it sitting on its empty "drop files here" screen.
    state
        .engine
        .wait_until_streamable(&info_hash, READY_TIMEOUT)
        .await?;

    let files = video_files(&state, &info_hash)?;
    if files.is_empty() {
        return Err(AppError::msg("в этой раздаче нет видеофайлов"));
    }
    let start = files.iter().position(|f| f.index == file_id).unwrap_or(0);

    let urls: Vec<String> = files
        .iter()
        .map(|f| state.streams.url_for(&info_hash, f.index))
        .collect();
    let title = files[start].name.clone();

    let cfg = state.config.read().player.clone();
    state.player.play(&urls, start, &title, &cfg).await?;

    remember_and_prefetch(&state, &info_hash, files.iter().map(|f| f.index).collect());

    let record = state.db.get_torrent(&info_hash)?;
    let topic_id = record.as_ref().and_then(|r| r.topic_id);
    state.db.history_add(
        topic_id,
        Some(&info_hash),
        record.as_ref().map(|r| r.name.as_str()).unwrap_or(&title),
        Some(&title),
        topic_id.and_then(|id| cached_preview(&state, id)).as_deref(),
        false,
    )?;
    if let Some(id) = topic_id {
        ensure_history_artwork(&state, id);
    }
    Ok(())
}

/// Watches a tracker release without adding it to the library.
///
/// The torrent goes into a scratch folder and is narrowed to its video files,
/// so "just show me this" does not pull a whole 60 GB release. Once the player
/// has been closed for a few minutes a background task deletes it; the cache is
/// also wiped at startup.
#[tauri::command]
pub async fn player_watch_topic(
    state: State<'_, Arc<AppState>>,
    topic_id: i64,
    title: Option<String>,
) -> AppResult<()> {
    ensure_player_available(&state)?;

    // Whatever was being watched before is now stale.
    drop_previous_temp(&state).await;

    let bytes = state.rutracker.download_torrent(topic_id).await?;
    let cache = state.stream_cache_dir();
    std::fs::create_dir_all(&cache)?;

    let added = state
        .engine
        .add(
            AddSource::Bytes(bytes),
            AddOptions {
                output_folder: Some(cache.to_string_lossy().to_string()),
                ..Default::default()
            },
        )
        .await?;

    let videos: Vec<usize> = added
        .files
        .iter()
        .filter(|f| player::is_video_file(&f.name))
        .map(|f| f.index)
        .collect();
    if videos.is_empty() {
        let _ = state.engine.delete(&added.info_hash).await;
        return Err(AppError::msg("в раздаче нет видеофайлов"));
    }

    state.db.upsert_torrent(
        &added.info_hash,
        added.name.as_deref().unwrap_or("просмотр"),
        &added.output_folder,
        added.total_bytes as i64,
        TorrentSource::Rutracker,
        Some(topic_id),
        None,
    )?;

    *state.temp_watch.lock() = Some(TempWatch {
        info_hash: added.info_hash.clone(),
        last_active: std::time::Instant::now(),
        paused: false,
    });

    state
        .engine
        .wait_until_streamable(&added.info_hash, READY_TIMEOUT)
        .await?;

    // Only now: librqbit refuses to change the file selection while a torrent
    // is still initializing, which is what made this fail on the first click.
    // Everything but the video is dead weight for a one-off viewing.
    if videos.len() < added.files.len() {
        narrow_to_videos(&state, &added.info_hash, &videos).await;
    }

    let files = video_files(&state, &added.info_hash)?;
    let urls: Vec<String> = files
        .iter()
        .map(|f| state.streams.url_for(&added.info_hash, f.index))
        .collect();
    let display = title
        .filter(|t| !t.trim().is_empty())
        .or_else(|| added.name.clone())
        .unwrap_or_else(|| files[0].name.clone());

    let cfg = state.config.read().player.clone();
    state.player.play(&urls, 0, &display, &cfg).await?;

    remember_and_prefetch(
        &state,
        &added.info_hash,
        files.iter().map(|f| f.index).collect(),
    );

    state.db.history_add(
        Some(topic_id),
        Some(&added.info_hash),
        &display,
        files.first().map(|f| f.name.as_str()),
        cached_preview(&state, topic_id).as_deref(),
        true,
    )?;
    ensure_history_artwork(&state, topic_id);
    Ok(())
}

/// Restricts a temporary stream to its video files.
///
/// Best effort on purpose: if the engine will not narrow the selection, the
/// release downloads in full — wasteful, but the scratch copy is deleted a few
/// minutes after viewing anyway, and refusing to play would be worse.
async fn narrow_to_videos(state: &AppState, info_hash: &str, videos: &[usize]) {
    for attempt in 0..5 {
        match state.engine.set_only_files(info_hash, videos.to_vec()).await {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!("could not narrow {info_hash} to video files: {e}");
                if attempt < 4 {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                }
            }
        }
    }
}

/// Records what is playing and starts pulling the following episode.
///
/// Waiting for the viewer to reach episode two before downloading it means a
/// pause at every episode boundary; fetching it while episode one plays out
/// removes that wait.
fn remember_and_prefetch(state: &Arc<AppState>, info_hash: &str, files: Vec<usize>) {
    *state.now_playing.lock() = Some(NowPlaying {
        info_hash: info_hash.to_string(),
        files: files.clone(),
    });

    let state = state.clone();
    let info_hash = info_hash.to_string();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;

            // Stop as soon as playback moves on or ends.
            let still_current = state
                .now_playing
                .lock()
                .as_ref()
                .map(|c| c.info_hash == info_hash)
                .unwrap_or(false);
            if !still_current || !state.player.is_playing() {
                return;
            }

            let Some(playback) = state.player.playback() else {
                continue;
            };
            let position = playback.playlist_pos.unwrap_or(0);
            let (Some(&current), Some(&next)) = (files.get(position), files.get(position + 1))
            else {
                // Nothing after this one; the prefetcher has no work left.
                return;
            };

            let (Ok(details), Ok(done)) = (
                state.engine.details(&info_hash),
                state.engine.file_progress(&info_hash),
            ) else {
                continue;
            };
            let length = |id: usize| {
                details
                    .files
                    .iter()
                    .find(|f| f.index == id)
                    .map(|f| f.length)
                    .unwrap_or(0)
            };

            let current_len = length(current);
            let next_len = length(next);
            let current_done =
                current_len > 0 && done.get(current).copied().unwrap_or(0) >= current_len;
            let next_done = next_len > 0 && done.get(next).copied().unwrap_or(0) >= next_len;
            if !current_done || next_done {
                continue;
            }

            // Opening a stream on the next file is what raises its priority:
            // librqbit fetches the pieces just past an open stream position.
            tracing::info!("prefetching next episode (file {next}) of {info_hash}");
            if let Err(e) = drain_file(&state, &info_hash, next).await {
                tracing::warn!("prefetch of file {next} stopped: {e}");
            }
        }
    });
}

/// Reads a file through to the end, which downloads it in order.
async fn drain_file(state: &AppState, info_hash: &str, file_id: usize) -> AppResult<()> {
    use tokio::io::AsyncReadExt;

    let mut stream = state.engine.file_stream(info_hash, file_id).await?;
    let mut buffer = vec![0u8; 512 * 1024];
    loop {
        // Give up the moment the viewer closes the player.
        if !state.player.is_playing() {
            return Ok(());
        }
        match stream.reader.read(&mut buffer).await {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(AppError::msg(format!("чтение прервано: {e}"))),
        }
    }
}

/// Artwork already cached for a topic, if the search grid has seen it.
fn cached_preview(state: &AppState, topic_id: i64) -> Option<String> {
    // Two layers of Option: the outer says whether the topic was ever looked
    // at, the inner whether it turned out to have a picture.
    state.db.get_topic_preview(topic_id).ok().flatten().flatten()
}

/// Makes sure a history entry ends up with a picture.
///
/// Nothing here is worth failing playback over: if the topic page cannot be
/// read, the entry simply keeps its placeholder.
fn ensure_history_artwork(state: &Arc<AppState>, topic_id: i64) {
    if cached_preview(state, topic_id).is_some() {
        return;
    }
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        let image = match state.rutracker.topic(topic_id).await {
            Ok(topic) => topic.images.into_iter().next(),
            Err(e) => {
                tracing::debug!("no artwork for topic {topic_id}: {e}");
                return;
            }
        };
        // Remembered either way, so a topic without pictures is not re-fetched.
        let _ = state.db.set_topic_preview(topic_id, image.as_deref());
        if let Some(url) = image {
            let _ = state.db.history_set_image(topic_id, &url);
            let _ = state.app_handle.emit(crate::state::events::HISTORY_UPDATED, ());
        }
    });
}

fn ensure_player_available(state: &AppState) -> AppResult<()> {
    let status = state.player.status();
    if status.available {
        return Ok(());
    }
    Err(AppError::msg(
        status
            .problem
            .unwrap_or_else(|| "проигрыватель недоступен".to_string()),
    ))
}

async fn drop_previous_temp(state: &AppState) {
    let previous = state.temp_watch.lock().take();
    if let Some(watch) = previous {
        let _ = state.engine.delete(&watch.info_hash).await;
        let _ = state.db.delete_torrent(&watch.info_hash);
    }
}

#[tauri::command]
pub async fn player_stop(state: State<'_, Arc<AppState>>) -> AppResult<()> {
    state.player.stop();
    // Ends the reads the player was waiting on, so teardown is immediate.
    state.streams.abort_all();
    *state.now_playing.lock() = None;
    Ok(())
}

/// Sends a raw mpv command, e.g. `["cycle", "pause"]`.
#[tauri::command]
pub async fn player_command(state: State<'_, Arc<AppState>>, args: Vec<String>) -> AppResult<()> {
    if args.is_empty() {
        return Err(AppError::msg("пустая команда плеера"));
    }
    state.player.command(&args)
}

// ------------------------------------------------------------------ history

#[tauri::command]
pub async fn history_list(state: State<'_, Arc<AppState>>) -> AppResult<Vec<WatchHistoryItem>> {
    state.db.history_list(60)
}

#[tauri::command]
pub async fn history_remove(state: State<'_, Arc<AppState>>, id: i64) -> AppResult<()> {
    state.db.history_remove(id)
}

#[tauri::command]
pub async fn history_clear(state: State<'_, Arc<AppState>>) -> AppResult<()> {
    state.db.history_clear()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodes_sort_in_natural_order() {
        let mut names = vec!["s01e10.mkv", "s01e02.mkv", "s01e01.mkv"];
        names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
        assert_eq!(names, vec!["s01e01.mkv", "s01e02.mkv", "s01e10.mkv"]);
    }

    #[test]
    fn plain_sorting_would_get_this_wrong() {
        // The exact case natural ordering exists for.
        let mut lexicographic = vec!["Серия 10.mkv", "Серия 2.mkv"];
        lexicographic.sort();
        assert_eq!(lexicographic[0], "Серия 10.mkv");

        let mut natural = vec!["Серия 10.mkv", "Серия 2.mkv"];
        natural.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
        assert_eq!(natural[0], "Серия 2.mkv");
    }

    #[test]
    fn numbers_sort_before_text() {
        let mut names = vec!["extras.mkv", "01.mkv"];
        names.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
        assert_eq!(names[0], "01.mkv");
    }
}
