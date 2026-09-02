//! Unfinished "watch online" downloads left behind by a previous run.
//!
//! A viewing normally cleans itself up: paused when the player closes, deleted
//! a few minutes later, and released outright when the application exits. What
//! survives all three is a crash or a reboot — and throwing that away without
//! asking is wrong, because a part-downloaded film is exactly what somebody
//! came back to finish.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::engine::{AddOptions, AddSource};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, TempWatch};

/// A viewing found on disk that nobody has decided about yet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Leftover {
    pub info_hash: String,
    pub title: String,
    pub topic_id: Option<i64>,
    /// How much of it is already downloaded.
    pub bytes_on_disk: u64,
    pub total_bytes: u64,
}

/// Viewings whose files are still on disk from an earlier run.
///
/// The one being watched right now is not among them: it is not a leftover
/// until the application has been restarted without it being cleaned up.
#[tauri::command]
pub async fn leftovers_list(state: State<'_, Arc<AppState>>) -> AppResult<Vec<Leftover>> {
    let cache = state.stream_cache_dir();
    let current = state.temp_watch.lock().as_ref().map(|w| w.info_hash.clone());

    let progress = state.engine.progress_all();
    let mut out = Vec::new();

    for row in state.db.list_torrents()? {
        if !Path::new(&row.output_folder).starts_with(&cache) {
            continue;
        }
        if current.as_deref() == Some(row.info_hash.as_str()) {
            continue;
        }
        let done = progress
            .iter()
            .find(|p| p.info_hash.eq_ignore_ascii_case(&row.info_hash))
            .map(|p| p.progress_bytes)
            .unwrap_or(0);

        out.push(Leftover {
            info_hash: row.info_hash,
            title: row.name,
            topic_id: row.topic_id,
            bytes_on_disk: done,
            total_bytes: row.total_bytes.max(0) as u64,
        });
    }
    Ok(out)
}

/// Picks a leftover back up, so closing the player cleans it away as usual.
#[tauri::command]
pub async fn leftover_resume(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<()> {
    *state.temp_watch.lock() = Some(TempWatch {
        info_hash: info_hash.clone(),
        last_active: std::time::Instant::now(),
        paused: false,
    });
    // It was paused when the previous run ended, or never started at all.
    state.engine.resume(&info_hash).await?;
    Ok(())
}

/// Deletes a leftover and frees the disk space.
#[tauri::command]
pub async fn leftover_drop(state: State<'_, Arc<AppState>>, info_hash: String) -> AppResult<()> {
    if state.temp_watch.lock().as_ref().map(|w| w.info_hash.as_str()) == Some(info_hash.as_str()) {
        *state.temp_watch.lock() = None;
    }
    state.engine.delete(&info_hash).await?;
    let _ = state.db.delete_torrent(&info_hash);
    tracing::info!(%info_hash, "просмотр удалён по решению пользователя");
    Ok(())
}

/// Moves a leftover into the download folder and keeps it as a normal torrent.
///
/// The files move rather than being fetched again, so nothing already
/// downloaded is thrown away. If re-registering it afterwards fails, the files
/// are still there — the error says where.
#[tauri::command]
pub async fn leftover_save(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<String> {
    let record = state
        .db
        .get_torrent(&info_hash)?
        .ok_or_else(|| AppError::msg("этот просмотр уже убран"))?;
    let topic_id = record
        .topic_id
        .ok_or_else(|| AppError::msg("не знаем, с какой раздачи это скачано"))?;

    let details = state.engine.details(&info_hash)?;
    let top = details
        .files
        .first()
        .and_then(|f| f.components.first().cloned())
        .ok_or_else(|| AppError::msg("в раздаче нет файлов"))?;

    let from = PathBuf::from(&details.output_folder).join(&top);
    let target_root = PathBuf::from(state.config.read().download_dir.clone());
    std::fs::create_dir_all(&target_root)?;
    let to = target_root.join(&top);

    if to.exists() {
        return Err(AppError::msg(format!(
            "в папке загрузок уже есть «{top}» — уберите её и повторите"
        )));
    }

    // Stop managing it before the files move out from under the engine.
    state.engine.forget(&info_hash).await?;
    let _ = state.db.delete_torrent(&info_hash);
    *state.temp_watch.lock() = None;

    std::fs::rename(&from, &to).map_err(|e| {
        AppError::msg(format!(
            "не удалось перенести файлы в папку загрузок: {e}. Они остались в {}",
            from.display()
        ))
    })?;

    // Re-add at the new home. librqbit checks what is already there, so the
    // part that was downloaded counts and only the rest is fetched.
    let bytes = state.rutracker.download_torrent(topic_id).await?;
    let added = state
        .engine
        .add(
            AddSource::Bytes(bytes),
            AddOptions {
                output_folder: Some(target_root.to_string_lossy().to_string()),
                overwrite: true,
                ..Default::default()
            },
        )
        .await?;

    state.db.upsert_torrent(
        &added.info_hash,
        added.name.as_deref().unwrap_or(&record.name),
        &added.output_folder,
        added.total_bytes as i64,
        crate::db::models::TorrentSource::Rutracker,
        Some(topic_id),
        None,
    )?;

    tracing::info!(%top, "просмотр сохранён в загрузки");
    Ok(to.to_string_lossy().to_string())
}

/// One leftover, as the cache accounting sees it.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub info_hash: String,
    /// When the viewing started, so the oldest goes first.
    pub added_at: i64,
    pub bytes: u64,
}

/// Which leftovers to drop to get the cache back under its ceiling.
///
/// Oldest first, and only as many as it takes: a limit is a reason to free
/// space, not a reason to wipe everything somebody meant to come back to.
pub fn evictions(entries: &[CacheEntry], limit_bytes: u64) -> Vec<String> {
    if limit_bytes == 0 {
        return Vec::new();
    }
    let mut used: u64 = entries.iter().map(|e| e.bytes).sum();
    if used <= limit_bytes {
        return Vec::new();
    }

    let mut oldest: Vec<&CacheEntry> = entries.iter().collect();
    oldest.sort_by_key(|e| e.added_at);

    let mut drop = Vec::new();
    for entry in oldest {
        if used <= limit_bytes {
            break;
        }
        used = used.saturating_sub(entry.bytes);
        drop.push(entry.info_hash.clone());
    }
    drop
}

#[cfg(test)]
mod cache_tests {
    use super::{CacheEntry, evictions};

    fn entry(hash: &str, added_at: i64, gb: u64) -> CacheEntry {
        CacheEntry {
            info_hash: hash.into(),
            added_at,
            bytes: gb * 1024 * 1024 * 1024,
        }
    }

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn nothing_is_dropped_while_there_is_room() {
        let entries = [entry("a", 1, 5), entry("b", 2, 5)];
        assert!(evictions(&entries, 20 * GB).is_empty());
    }

    #[test]
    fn the_oldest_goes_first_and_only_as_far_as_needed() {
        let entries = [entry("new", 30, 8), entry("old", 10, 8), entry("mid", 20, 8)];
        // 24 GB against a 20 GB ceiling: dropping one is enough.
        assert_eq!(evictions(&entries, 20 * GB), vec!["old".to_string()]);
    }

    #[test]
    fn several_go_when_one_is_not_enough() {
        let entries = [entry("old", 10, 8), entry("mid", 20, 8), entry("new", 30, 8)];
        assert_eq!(
            evictions(&entries, 5 * GB),
            vec!["old".to_string(), "mid".to_string(), "new".to_string()]
        );
    }

    #[test]
    fn a_ceiling_of_zero_means_no_ceiling() {
        let entries = [entry("a", 1, 500)];
        assert!(evictions(&entries, 0).is_empty());
    }
}
