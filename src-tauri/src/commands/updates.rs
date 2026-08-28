//! Commands for the "release was updated" flow.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::db::models::{TopicUpdate, UpdateStatus};
use crate::engine::AddedTorrent;
use crate::error::AppResult;
use crate::state::AppState;
use crate::updates::{self, CheckOutcome};

#[tauri::command]
pub async fn updates_list(
    state: State<'_, Arc<AppState>>,
    only_pending: bool,
) -> AppResult<Vec<TopicUpdate>> {
    state
        .db
        .list_updates(only_pending.then_some(UpdateStatus::Pending))
}

#[tauri::command]
pub async fn updates_pending_count(state: State<'_, Arc<AppState>>) -> AppResult<i64> {
    state.db.count_pending_updates()
}

/// Manual "check now" from the UI.
#[tauri::command]
pub async fn updates_check_now(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> AppResult<CheckOutcome> {
    updates::check_and_notify(&app, &state).await
}

/// Downloads the new release and swaps it in, keeping already-downloaded data.
#[tauri::command]
pub async fn updates_apply(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    update_id: i64,
) -> AppResult<AddedTorrent> {
    updates::apply_update(&app, &state, update_id).await
}

/// Hides an update without applying it. The topic stays tracked, but this
/// particular hash will not be reported again.
#[tauri::command]
pub async fn updates_dismiss(
    state: State<'_, Arc<AppState>>,
    update_id: i64,
) -> AppResult<()> {
    state.db.set_update_status(update_id, UpdateStatus::Dismissed)
}

/// Turns update watching on or off for a single topic.
#[tauri::command]
pub async fn updates_set_topic_enabled(
    state: State<'_, Arc<AppState>>,
    topic_id: i64,
    enabled: bool,
) -> AppResult<()> {
    state.db.set_topic_enabled(topic_id, enabled)
}

#[tauri::command]
pub async fn updates_tracked_topics(
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<crate::db::models::TrackedTopic>> {
    state.db.list_tracked_topics(false)
}
