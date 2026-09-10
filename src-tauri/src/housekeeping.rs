//! Everything that has to happen on its own, in one place.
//!
//! Five separate loops used to run on five different clocks, each taking its
//! own snapshot of the torrents and each unaware of the others. Two of them
//! ended up pulling one torrent in opposite directions every second — the
//! queue restarting a film the player had just paused. One sweep, one snapshot
//! per tick, rules applied in a fixed order: that is what stops it recurring.
//!
//! The decisions themselves live in pure, tested functions elsewhere
//! (`queue_plan`, `SeedingConfig::should_stop`, `TempWatch::sweep`,
//! `evictions`, `effective_limits`). This module only drives them.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::db;
use crate::engine;
use crate::state::{AppState, TempWatchAction, events};

/// The sweep runs once a second; everything else is a multiple of that.
const TICK: Duration = Duration::from_secs(1);

/// Ticks between progress pushes when nobody is watching — the window is away
/// in the tray, or every torrent is just seeding. A second-by-second update
/// costs a redraw of the whole list, and in the tray nobody sees it at all.
const IDLE_STRIDE: u64 = 5;
/// Ticks between player and cache checks.
const PLAYER_STRIDE: u64 = 20;
/// Ticks between looks at the tracker browser window.
const BROWSER_STRIDE: u64 = 30;
/// Ticks between speed-limit reconciliations. Checked rather than slept until
/// the boundary: the machine can suspend, the clock can move, and a loop that
/// woke up once a day would get both of those wrong.
const SCHEDULE_STRIDE: u64 = 60;

/// How long a "just watch it" download survives after the film is closed.
const TEMP_WATCH_GRACE: Duration = Duration::from_secs(5 * 60);

/// Starts the sweep.
pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // Torrents already complete at startup must not fire a notification,
        // so seed the set from the database.
        let mut announced: HashSet<String> = state
            .db
            .list_torrents()
            .map(|rows| {
                rows.into_iter()
                    .filter(|r| r.completed_at.is_some())
                    .map(|r| r.info_hash.to_uppercase())
                    .collect()
            })
            .unwrap_or_default();
        // Torrents already stopped for reaching their ratio, so the engine is
        // not asked to pause them again on every sweep.
        let mut stopped: HashSet<String> = HashSet::new();
        let mut applied_limits: Option<(u32, u32)> = None;
        let mut progress_stride: u64 = 1;
        let mut was_watched = true;
        let mut tick: u64 = 0;

        loop {
            tokio::time::sleep(TICK).await;
            tick += 1;

            // ---- the player first, so "playing" is right for what follows
            let playing = if tick % PLAYER_STRIDE == 0 {
                Some(sweep_player(&state))
            } else {
                None
            };

            // ---- speed limits, on their own clock
            if tick == 1 || tick % SCHEDULE_STRIDE == 0 {
                use chrono::Timelike;
                let wanted = state
                    .config
                    .read()
                    .effective_limits(chrono::Local::now().hour());
                if applied_limits != Some(wanted) {
                    state.engine.set_rate_limits(wanted.0, wanted.1);
                    applied_limits = Some(wanted);
                }
            }

            // ---- the torrents: one snapshot, every rule reads the same one
            if tick % progress_stride == 0 {
                let watched = main_window_is_watched(&app);
                let progress = state.engine.progress_all();
                // One read of the database per sweep, shared by every rule
                // below. Each used to query on its own, which came to three
                // queries a second for a list that changes a few times a day.
                let records = state.db.list_torrents().unwrap_or_default();

                // Completion is noticed and recorded whether or not anyone is
                // looking: the notification is the point of it.
                for p in &progress {
                    let hash = p.info_hash.to_uppercase();
                    if !p.finished || announced.contains(&hash) {
                        continue;
                    }
                    announced.insert(hash.clone());
                    let _ = state.db.mark_torrent_completed(&hash);
                    let _ = app.emit(events::TORRENT_COMPLETED, p);
                }

                // Hashes that have left the session must not linger as
                // "stopped", or a re-added torrent would inherit a decision
                // made about its predecessor.
                stopped.retain(|h| progress.iter().any(|p| &p.info_hash == h));

                if let Some(playing) = playing {
                    sweep_temp_watch(&state, playing).await;
                    enforce_cache_limit(&state).await;
                }
                stop_over_seeded(&state, &progress, &records, &mut stopped).await;
                apply_queue(&state, &progress, &records, &stopped).await;

                // Coming back from the tray gets an immediate update rather
                // than showing figures up to five seconds stale.
                if watched && !progress.is_empty() {
                    let _ = app.emit(events::PROGRESS, &progress);
                }

                let moving = progress
                    .iter()
                    .any(|p| !p.finished || p.download_speed_bps > 0);
                progress_stride = if watched && (moving || !was_watched) {
                    1
                } else {
                    IDLE_STRIDE
                };
                was_watched = watched;
            } else if let Some(playing) = playing {
                // The player group must not skip a beat just because the
                // torrent group is idling.
                sweep_temp_watch(&state, playing).await;
                enforce_cache_limit(&state).await;
            }

            // ---- the tracker window, rarely
            if tick % BROWSER_STRIDE == 0 {
                state.rutracker.browser().close_if_idle();
            }
        }
    });
}

/// Frees the mpv handle once its window is gone and records the viewing
/// position while it is not. Returns whether a film is on screen.
fn sweep_player(state: &Arc<AppState>) -> bool {
    let was_playing = state.player.is_playing();
    state.player.reap_if_closed();
    if was_playing && !state.player.is_playing() {
        // Playback just ended: release the reads it was holding.
        state.streams.abort_all();
        *state.now_playing.lock() = None;
    }

    let playing = state.player.is_playing();
    if playing {
        save_watch_position(state);
    }
    playing
}

/// Pauses, resumes or deletes the "just watch it" download according to
/// whether the film is still open. Deleting waits out [`TEMP_WATCH_GRACE`]:
/// reopening a film a minute later should not have to download it again.
async fn sweep_temp_watch(state: &Arc<AppState>, playing: bool) {
    let mut to_pause = None;
    let mut expired = None;

    {
        let mut guard = state.temp_watch.lock();
        let now = std::time::Instant::now();
        let action = guard.as_mut().map(|w| w.sweep(now, playing, TEMP_WATCH_GRACE));
        let info_hash = guard.as_ref().map(|w| w.info_hash.clone());
        match (action, info_hash) {
            (Some(TempWatchAction::Resume), Some(h)) => to_pause = Some((h, false)),
            (Some(TempWatchAction::Pause), Some(h)) => to_pause = Some((h, true)),
            (Some(TempWatchAction::Delete), _) => expired = guard.take().map(|w| w.info_hash),
            _ => {}
        }
    }

    if let Some((info_hash, pause)) = to_pause {
        let result = if pause {
            state.engine.pause(&info_hash).await
        } else {
            state.engine.resume(&info_hash).await
        };
        if let Err(e) = result {
            tracing::warn!(
                "could not {} temporary stream: {e}",
                if pause { "pause" } else { "resume" }
            );
        }
    }

    if let Some(info_hash) = expired {
        tracing::info!("dropping temporary stream {info_hash}");
        if let Err(e) = state.engine.delete(&info_hash).await {
            tracing::warn!("could not delete temporary stream: {e}");
        }
        let _ = state.db.delete_torrent(&info_hash);
    }
}

/// Whether the main window is actually in front of someone.
///
/// Hidden in the tray or minimised, a pushed update costs a full redraw that
/// nobody sees — which is most of what this application does with its day.
fn main_window_is_watched(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let visible = window.is_visible().unwrap_or(true);
    let minimised = window.is_minimized().unwrap_or(false);
    visible && !minimised
}

/// Writes down how far into the film the viewer has got.
///
/// Called on the sweep rather than on closing: the player can go away without
/// warning, and a position saved a few seconds ago is far better than none.
fn save_watch_position(state: &Arc<AppState>) {
    let Some(playback) = state.player.playback() else {
        return;
    };
    let Some(position) = playback.position else {
        return;
    };
    let guard = state.now_playing.lock();
    let Some(current) = guard.as_ref() else {
        return;
    };
    let name = playback
        .playlist_pos
        .and_then(|i| current.names.get(i).cloned());
    if let Err(e) = state.db.history_set_position(
        current.topic_id,
        name.as_deref(),
        position,
        playback.duration,
    ) {
        tracing::warn!("не удалось сохранить позицию просмотра: {e}");
    }
}

/// Keeps the "watch online" cache under its ceiling.
///
/// Leftovers are kept until somebody decides about them, which is right — but
/// kept forever they would fill a disk. The oldest go first, and never the one
/// being watched.
async fn enforce_cache_limit(state: &Arc<AppState>) {
    use crate::commands::leftovers::{CacheEntry, evictions};

    let limit_gb = state.config.read().stream_cache_limit_gb as u64;
    if limit_gb == 0 {
        return;
    }

    let cache = state.stream_cache_dir();
    let current = state.temp_watch.lock().as_ref().map(|w| w.info_hash.clone());
    let Ok(rows) = state.db.list_torrents() else {
        return;
    };
    let progress = state.engine.progress_all();

    let entries: Vec<CacheEntry> = rows
        .into_iter()
        .filter(|r| std::path::Path::new(&r.output_folder).starts_with(&cache))
        .filter(|r| current.as_deref() != Some(r.info_hash.as_str()))
        .map(|r| CacheEntry {
            bytes: progress
                .iter()
                .find(|p| p.info_hash.eq_ignore_ascii_case(&r.info_hash))
                .map(|p| p.progress_bytes)
                .unwrap_or(0),
            added_at: r.added_at,
            info_hash: r.info_hash,
        })
        .collect();

    for info_hash in evictions(&entries, limit_gb * 1024 * 1024 * 1024) {
        tracing::info!(%info_hash, "кэш просмотра переполнен, убираем самый старый");
        if let Err(e) = state.engine.delete(&info_hash).await {
            tracing::warn!("не удалось убрать старый просмотр: {e}");
            continue;
        }
        let _ = state.db.delete_torrent(&info_hash);
    }
}

/// Pauses finished torrents that have given back as much as was asked of them.
async fn stop_over_seeded(
    state: &Arc<AppState>,
    progress: &[engine::TorrentProgress],
    records: &[db::models::TorrentRecord],
    stopped: &mut HashSet<String>,
) {
    let seeding = state.config.read().seeding.clone();
    // Individual releases can be marked "download, then stop" even when the
    // global rules are off, so this cannot bail out on the config alone.
    let marked: HashSet<String> = records
        .iter()
        .filter(|r| r.no_seeding)
        .map(|r| r.info_hash.to_uppercase())
        .collect();
    if !seeding.is_active() && marked.is_empty() {
        // A limit lifted while running frees everything paused for it.
        stopped.clear();
        return;
    }

    for p in progress {
        if !p.finished || p.state == "paused" || stopped.contains(&p.info_hash) {
            continue;
        }
        let personal = marked.contains(&p.info_hash.to_uppercase());
        if !personal && !seeding.should_stop(p.uploaded_bytes, p.total_bytes) {
            continue;
        }
        stopped.insert(p.info_hash.clone());
        tracing::info!(
            name = p.name.as_deref().unwrap_or("раздача"),
            "раздача остановлена по настройке"
        );
        if let Err(e) = state.engine.pause(&p.info_hash).await {
            tracing::warn!("не удалось остановить раздачу: {e}");
            stopped.remove(&p.info_hash);
        }
    }
}

/// Pushes live stats to the UI and turns "finished" into a one-shot event.
/// Keeps no more than the allowed number of downloads running at once.
///
/// Anything the seeding rules have stopped counts as untouchable here, exactly
/// like a download somebody paused by hand: two mechanisms taking turns to
/// start and stop the same torrent would be worse than either alone.
async fn apply_queue(
    state: &Arc<AppState>,
    progress: &[engine::TorrentProgress],
    records: &[db::models::TorrentRecord],
    seeding_stopped: &HashSet<String>,
) {
    use crate::commands::torrents::{QueueItem, queue_plan};

    let max_active = state.config.read().max_active_downloads;
    let cache = state.stream_cache_dir();

    let items: Vec<QueueItem> = progress
        .iter()
        .filter_map(|p| {
            let record = records
                .iter()
                .find(|r| r.info_hash.eq_ignore_ascii_case(&p.info_hash))?;
            Some(QueueItem {
                user_paused: record.user_paused
                    || seeding_stopped.contains(&p.info_hash)
                    || (p.finished && record.no_seeding),
                info_hash: p.info_hash.clone(),
                added_at: record.added_at,
                finished: p.finished,
                forced: record.forced,
                scratch: std::path::Path::new(&record.output_folder).starts_with(&cache),
                running: p.state != "paused",
            })
        })
        .collect();

    let (start, pause) = queue_plan(&items, max_active);
    for info_hash in pause {
        tracing::info!(%info_hash, "очередь: ждёт своей очереди");
        if let Err(e) = state.engine.pause(&info_hash).await {
            tracing::warn!("не удалось приостановить по очереди: {e}");
        }
    }
    for info_hash in start {
        if let Err(e) = state.engine.resume(&info_hash).await {
            tracing::warn!("не удалось запустить по очереди: {e}");
        }
    }
}
