//! Detection and application of re-uploaded releases.
//!
//! On RuTracker a maintainer who ships a new build replaces the torrent
//! attached to the topic: the topic id stays, the info hash changes and
//! `reg_time` moves forward. Spotting that transition is the whole feature.
//!
//! There are two ways to see it, and the watcher prefers the cheap one:
//!
//! 1. **The JSON API** — no login, no Cloudflare challenge, 100 topics per
//!    request. The tracker currently has it switched off.
//! 2. **Topic pages** — the fallback. Each page carries `#tor-hash`, but costs
//!    one browser page load and needs a live session, so runs are capped and
//!    topics rotate by least-recently-checked.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::db::models::{TopicUpdate, TorrentSource, TrackedTopic, UpdateStatus};
use crate::engine::{AddOptions, AddSource, AddedTorrent};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, events};
use crate::trackers::rutracker::TopicData;

/// Topic pages checked in a single fallback run. Keeps one poll from turning
/// into a hundred page loads against the tracker.
const MAX_PAGE_CHECKS: usize = 40;
/// Pause between page loads in fallback mode, to stay polite.
const PAGE_CHECK_DELAY: Duration = Duration::from_millis(1500);

/// Which source the last check actually used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckMethod {
    Api,
    Pages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckOutcome {
    pub checked: usize,
    pub method: CheckMethod,
    /// Topics left for the next run because of [`MAX_PAGE_CHECKS`].
    pub deferred: usize,
    pub new_updates: Vec<TopicUpdate>,
    /// Topics the tracker no longer knows about — deleted or moved releases.
    pub missing_topics: Vec<i64>,
    /// Human-readable explanation shown in the UI, e.g. why the fallback ran.
    pub note: Option<String>,
}

/// Runs one poll over the enabled tracked topics.
pub async fn check_now(state: &AppState) -> AppResult<CheckOutcome> {
    let topics = state.db.list_tracked_topics(true)?;
    if topics.is_empty() {
        return Ok(CheckOutcome {
            checked: 0,
            method: CheckMethod::Api,
            deferred: 0,
            new_updates: Vec::new(),
            missing_topics: Vec::new(),
            note: None,
        });
    }

    let ids: Vec<i64> = topics.iter().map(|t| t.topic_id).collect();

    // Try the cheap path first; it may come back at any time.
    let (data, method, deferred, note) = match state.rutracker.api().get_topic_data(&ids).await {
        Ok(data) => (data, CheckMethod::Api, 0, None),
        Err(e) => {
            let reason = match &e {
                AppError::ApiUnavailable(text) => {
                    format!("API трекера отключён ({text}), проверка идёт по страницам тем")
                }
                other => format!("API трекера недоступен ({other}), проверка идёт по страницам тем"),
            };
            tracing::info!("{reason}");

            let (data, deferred) = check_via_pages(state, &topics).await?;
            (data, CheckMethod::Pages, deferred, Some(reason))
        }
    };

    let mut missing_topics = Vec::new();
    let mut fresh_ids = Vec::new();
    let mut examined = Vec::new();

    for topic in &topics {
        let Some(remote) = data.get(&topic.topic_id) else {
            // In page mode a topic simply may not have been visited this run.
            if method == CheckMethod::Api {
                missing_topics.push(topic.topic_id);
            }
            continue;
        };
        examined.push(topic.topic_id);

        let Some(remote_hash) = remote.info_hash.as_deref() else {
            continue;
        };

        let local_hash = topic.info_hash.to_uppercase();
        if remote_hash == local_hash {
            // Same release — top up metadata we may not have had before.
            state.db.set_topic_current(
                topic.topic_id,
                remote_hash,
                remote.size_bytes,
                remote.reg_time,
            )?;
            continue;
        }

        // A different hash with an *older* registration time is not an update;
        // our local record is ahead, so leave it alone. Page mode has no
        // reg_time, and there a hash change is decisive on its own.
        if let (Some(remote_time), Some(local_time)) = (remote.reg_time, topic.reg_time) {
            if remote_time <= local_time {
                continue;
            }
        }

        let is_new = state.db.insert_update(
            topic.topic_id,
            &local_hash,
            remote_hash,
            topic.size_bytes,
            remote.size_bytes,
            remote.reg_time,
        )?;
        if is_new {
            fresh_ids.push(topic.topic_id);
        }
    }

    state.db.touch_topics_checked(&examined)?;

    let new_updates: Vec<TopicUpdate> = state
        .db
        .list_updates(Some(UpdateStatus::Pending))?
        .into_iter()
        .filter(|u| fresh_ids.contains(&u.topic_id))
        .collect();

    Ok(CheckOutcome {
        checked: examined.len(),
        method,
        deferred,
        new_updates,
        missing_topics,
        note,
    })
}

/// Fallback: read `#tor-hash` off topic pages through the browser transport.
///
/// Returns the data gathered plus how many topics were deferred to a later run.
async fn check_via_pages(
    state: &AppState,
    topics: &[TrackedTopic],
) -> AppResult<(HashMap<i64, TopicData>, usize)> {
    // Topic pages are behind the login wall, so without a session this path
    // cannot work at all — say so instead of silently reporting "no updates".
    if !state.rutracker.cached_logged_in() && !state.rutracker.verify_session().await? {
        return Err(AppError::NotAuthenticated);
    }

    // Least-recently-checked first, so every topic gets its turn across runs.
    let mut queue: Vec<&TrackedTopic> = topics.iter().collect();
    queue.sort_by_key(|t| t.last_checked_at.unwrap_or(0));

    let deferred = queue.len().saturating_sub(MAX_PAGE_CHECKS);
    let mut out = HashMap::new();

    for (i, topic) in queue.iter().take(MAX_PAGE_CHECKS).enumerate() {
        if i > 0 {
            tokio::time::sleep(PAGE_CHECK_DELAY).await;
        }
        match state.rutracker.topic(topic.topic_id).await {
            Ok(details) => {
                out.insert(
                    topic.topic_id,
                    TopicData {
                        topic_id: topic.topic_id,
                        info_hash: details.info_hash,
                        size_bytes: details.size_bytes,
                        topic_title: Some(details.title),
                        // Not published on the page in a form worth parsing;
                        // a hash change alone is enough to call it an update.
                        ..Default::default()
                    },
                );
            }
            // One unreachable topic must not abort the whole run.
            Err(e) => tracing::warn!("topic {} check failed: {e}", topic.topic_id),
        }
    }

    Ok((out, deferred))
}

/// Runs a check and pushes the results to the UI and the notification centre.
pub async fn check_and_notify(app: &AppHandle, state: &AppState) -> AppResult<CheckOutcome> {
    let _ = app.emit(events::UPDATE_CHECK_STATE, "checking");
    let outcome = match check_now(state).await {
        Ok(o) => o,
        Err(e) => {
            let _ = app.emit(events::UPDATE_CHECK_STATE, "error");
            return Err(e);
        }
    };
    let _ = app.emit(events::UPDATE_CHECK_STATE, "idle");

    if !outcome.new_updates.is_empty() {
        let _ = app.emit(events::UPDATES_FOUND, &outcome.new_updates);

        // Applying them without asking, when that is what was asked for. The
        // setting existed from the start and did nothing at all — a promise
        // the settings screen never kept.
        if state.config.read().updates.auto_download {
            for update in &outcome.new_updates {
                match apply_update(app, state, update.id).await {
                    Ok(_) => tracing::info!(
                        title = update.title.as_deref().unwrap_or("раздача"),
                        "обновление скачано автоматически"
                    ),
                    Err(e) => tracing::warn!("не удалось обновить раздачу автоматически: {e}"),
                }
            }
        }

        if state.config.read().updates.notify_desktop {
            let automatic = state.config.read().updates.auto_download;
            let body = match outcome.new_updates.len() {
                1 => outcome.new_updates[0]
                    .title
                    .clone()
                    .unwrap_or_else(|| "Раздача обновилась".to_string()),
                n => format!("Обновилось раздач: {n}"),
            };
            let _ = app
                .notification()
                .builder()
                .title(if automatic {
                    "Panda Torrent — раздача обновлена"
                } else {
                    "Panda Torrent — доступно обновление"
                })
                .body(body)
                .show();
        }
    }

    Ok(outcome)
}

/// Background loop. Sleeps between polls and re-reads the interval each time,
/// so a settings change takes effect without a restart.
pub fn spawn_watcher(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let (enabled, on_startup) = {
            let cfg = state.config.read();
            (cfg.updates.enabled, cfg.updates.check_on_startup)
        };

        if enabled && on_startup {
            // Let the session settle and the UI mount before hitting the network.
            tokio::time::sleep(Duration::from_secs(20)).await;
            if let Err(e) = check_and_notify(&app, &state).await {
                tracing::warn!("startup update check failed: {e}");
            }
        }

        loop {
            let (enabled, interval) = {
                let cfg = state.config.read();
                (cfg.updates.enabled, cfg.updates.interval_minutes.max(15))
            };
            tokio::time::sleep(Duration::from_secs(interval as u64 * 60)).await;
            if !enabled {
                continue;
            }
            if let Err(e) = check_and_notify(&app, &state).await {
                tracing::warn!("update check failed: {e}");
            }
        }
    });
}

/// Downloads the refreshed release and swaps it in, keeping the library card,
/// the download folder and every already-downloaded piece.
pub async fn apply_update(
    app: &AppHandle,
    state: &AppState,
    update_id: i64,
) -> AppResult<AddedTorrent> {
    let update = state
        .db
        .get_update(update_id)?
        .ok_or_else(|| AppError::msg("обновление не найдено"))?;

    let old = state
        .db
        .get_torrent(&update.old_info_hash)?
        .ok_or(AppError::TorrentNotFound)?;

    // Fetched before anything is torn down, so a failed download leaves the
    // existing torrent untouched.
    let bytes = state.rutracker.download_torrent(update.topic_id).await?;

    // Dropping the old torrent frees the files for the new one. `forget`
    // deliberately leaves everything on disk.
    if state.engine.has(&update.old_info_hash) {
        state.engine.forget(&update.old_info_hash).await?;
    }

    let added = state
        .engine
        .add(
            AddSource::Bytes(bytes.clone()),
            AddOptions {
                output_folder: Some(old.output_folder.clone()),
                paused: false,
                only_files: None,
                // Unchanged pieces are re-used instead of re-downloaded, which
                // is the whole point of updating in place.
                overwrite: true,
            },
        )
        .await?;

    state.db.upsert_torrent(
        &added.info_hash,
        added.name.as_deref().unwrap_or(&old.name),
        &added.output_folder,
        added.total_bytes as i64,
        TorrentSource::Rutracker,
        Some(update.topic_id),
        Some(&bytes),
    )?;

    // The new row must exist before the old one is dropped, or the cascade
    // would take the library card with it.
    state
        .db
        .migrate_to_new_hash(&update.old_info_hash, &added.info_hash)?;
    state.db.set_topic_current(
        update.topic_id,
        &added.info_hash,
        update.new_size_bytes,
        update.new_reg_time,
    )?;
    state
        .db
        .set_update_status(update_id, UpdateStatus::Applied)?;

    let _ = app.emit(events::UPDATES_FOUND, Vec::<TopicUpdate>::new());
    Ok(added)
}
