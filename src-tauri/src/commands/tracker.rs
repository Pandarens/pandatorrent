//! Commands backing the RuTracker sign-in and search views.
//!
//! Signing in is not a form POST any more: Cloudflare only lets a real browser
//! through, so [`rutracker_open_login`] shows the tracker's own login page in
//! the worker window and the app watches for the session to appear.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::models::TorrentSource;
use crate::engine::{AddOptions, AddSource, AddedTorrent};
use crate::error::{AppError, AppResult};
use crate::library;
use crate::state::AppState;
use crate::trackers::rutracker::browser::{JobResult, PageState};
use crate::trackers::rutracker::{CatalogCategory, SearchPage, SearchQuery, TopicDetails, auth};

use super::torrents::{install_dir, register};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerStatus {
    pub username: Option<String>,
    /// The worker page reports a logged-in session.
    pub has_session: bool,
    pub host: String,
    /// A Cloudflare interstitial is currently on screen.
    pub challenged: bool,
    /// The webview holds a `bb_session` cookie. A hint, not proof — only a
    /// page carrying the logout link settles it — but it is readable even when
    /// the injected agent is silent, which makes it useful for diagnosis.
    pub has_cookie: bool,
    /// `has_session` came from a live page rather than from the remembered
    /// flag. False right after launch, before any browser window has opened.
    pub verified: bool,
}

/// Cheap, cached status — no page load, so it is safe to call on every render.
///
/// Right after launch no browser window exists yet, so nothing can read the
/// cookie jar. Falling back to the remembered flag is what stops the app from
/// asking the user to sign in again on every start while the session is in fact
/// still valid; `verified` says which of the two answers this is.
#[tauri::command]
pub async fn rutracker_status(state: State<'_, Arc<AppState>>) -> AppResult<TrackerStatus> {
    let browser = state.rutracker.browser();
    let page = browser.page_state();
    let cfg = state.config.read();

    // The browser's remembered answer wins; it only changes when a page
    // actually shows a logout link or the guest login form. Falling back to
    // the current page made the badge flip every time the worker window was
    // opened or closed, or landed on a page carrying neither marker.
    let known = browser.session_known();
    Ok(TrackerStatus {
        username: cfg.rutracker.username.clone(),
        has_session: known.unwrap_or(cfg.rutracker.logged_in_at.is_some()),
        host: browser.host(),
        challenged: page.challenged,
        has_cookie: browser.session_cookie().is_some(),
        verified: known.is_some(),
    })
}

/// Loads a forum page to confirm the session, and refreshes the cached
/// username while it is there.
#[tauri::command]
pub async fn rutracker_verify(state: State<'_, Arc<AppState>>) -> AppResult<TrackerStatus> {
    let browser = state.rutracker.browser();
    let html = browser.get_text("index.php").await?;
    let logged_in = auth::is_logged_in_html(&html);
    let username = auth::logged_in_username(&html);
    browser.set_session(logged_in);

    {
        let mut cfg = state.config.write();
        cfg.rutracker.username = if logged_in { username.clone() } else { None };
        // Remembered so the next launch does not offer to sign in again.
        cfg.rutracker.logged_in_at = logged_in.then(crate::db::now);
    }
    let _ = state.save_config();

    Ok(TrackerStatus {
        username,
        has_session: logged_in,
        host: state.rutracker.host(),
        challenged: false,
        has_cookie: state.rutracker.browser().session_cookie().is_some(),
        verified: true,
    })
}

/// Opens the tracker's login page in the worker window for the user to sign in.
#[tauri::command]
pub async fn rutracker_open_login(state: State<'_, Arc<AppState>>) -> AppResult<()> {
    state.rutracker.open_login().await
}

/// Result of the "check the tracker connection" diagnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTest {
    pub ok: bool,
    /// The webview holds a session cookie.
    pub has_cookie: bool,
    /// The Cloudflare interstitial is still on screen.
    pub challenged: bool,
    pub logged_in: bool,
    /// Size of the page that came back, as evidence the transport works.
    pub bytes: usize,
    pub url: String,
    pub message: String,
}

/// Exercises the whole browser transport in one call: opening the worker
/// window, clearing the Cloudflare challenge, running a `fetch()` inside the
/// tracker page and getting the bytes back over IPC.
///
/// This integration has a lot of moving parts outside our control, so it is
/// worth being able to answer "is it the tracker or is it us?" in one click.
#[tauri::command]
pub async fn rutracker_selftest(state: State<'_, Arc<AppState>>) -> AppResult<SelfTest> {
    let browser = state.rutracker.browser();
    match browser.get_text("index.php").await {
        Ok(html) => {
            let page = browser.page_state();
            let logged_in = auth::is_logged_in_html(&html);
            Ok(SelfTest {
                ok: true,
                has_cookie: browser.session_cookie().is_some(),
                challenged: page.challenged,
                logged_in,
                bytes: html.len(),
                url: page.url,
                message: if logged_in {
                    "Связь с трекером есть, вход выполнен".into()
                } else {
                    "Связь с трекером есть, но вход не выполнен".into()
                },
            })
        }
        Err(e) => Ok(SelfTest {
            ok: false,
            has_cookie: browser.session_cookie().is_some(),
            challenged: browser.page_state().challenged,
            logged_in: false,
            bytes: 0,
            url: browser.page_state().url,
            message: e.to_string(),
        }),
    }
}

/// Hides the worker window once the user is done with it.
#[tauri::command]
pub async fn rutracker_hide_login(state: State<'_, Arc<AppState>>) -> AppResult<()> {
    state.rutracker.hide_login();
    Ok(())
}

#[tauri::command]
pub async fn rutracker_logout(state: State<'_, Arc<AppState>>) -> AppResult<TrackerStatus> {
    let browser = state.rutracker.browser();
    state.rutracker.logout().await?;
    browser.set_session(false);
    {
        let mut cfg = state.config.write();
        cfg.rutracker.username = None;
        cfg.rutracker.logged_in_at = None;
    }
    let _ = state.save_config();

    Ok(TrackerStatus {
        username: None,
        has_session: false,
        host: state.rutracker.host(),
        challenged: false,
        has_cookie: false,
        verified: true,
    })
}

#[tauri::command]
pub async fn rutracker_search(
    state: State<'_, Arc<AppState>>,
    query: SearchQuery,
) -> AppResult<SearchPage> {
    state.rutracker.search(&query).await
}

#[tauri::command]
pub async fn rutracker_topic(
    state: State<'_, Arc<AppState>>,
    topic_id: i64,
) -> AppResult<TopicDetails> {
    state.rutracker.topic(topic_id).await
}

/// How long a cached catalogue stays fresh. The tracker's section list changes
/// perhaps a couple of times a year, and refetching it costs a page load.
const CATALOG_TTL_SECS: i64 = 7 * 24 * 3600;

const KV_CATALOG: &str = "rutracker.catalog";

#[derive(Serialize, Deserialize)]
struct Cached<T> {
    fetched_at: i64,
    value: T,
}

fn read_cache<T: for<'de> Deserialize<'de>>(state: &AppState, key: &str) -> Option<T> {
    read_cache_with_ttl(state, key, CATALOG_TTL_SECS)
}

fn read_cache_with_ttl<T: for<'de> Deserialize<'de>>(
    state: &AppState,
    key: &str,
    ttl_secs: i64,
) -> Option<T> {
    let raw = state.db.kv_get(key).ok()??;
    let cached: Cached<T> = serde_json::from_str(&raw).ok()?;
    let age = crate::db::now() - cached.fetched_at;
    (age < ttl_secs).then_some(cached.value)
}

fn write_cache<T: Serialize>(state: &AppState, key: &str, value: &T) {
    let payload = Cached {
        fetched_at: crate::db::now(),
        value,
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = state.db.kv_set(key, &json);
    }
}

/// The tracker's whole section tree, for browsing without a search query.
///
/// Cached, because the list is near-static and a miss costs a page load through
/// the browser transport.
#[tauri::command]
pub async fn rutracker_catalog(
    state: State<'_, Arc<AppState>>,
    refresh: bool,
) -> AppResult<Vec<CatalogCategory>> {
    if !refresh {
        if let Some(cached) = read_cache::<Vec<CatalogCategory>>(&state, KV_CATALOG) {
            return Ok(cached);
        }
    }
    let categories = state.rutracker.catalog().await?;
    write_cache(&state, KV_CATALOG, &categories);
    Ok(categories)
}

/// One "what is new" strip on the home screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReleaseSection {
    pub forum_id: i64,
    pub forum_title: String,
    pub items: Vec<crate::trackers::rutracker::SearchItem>,
    /// Set when this strip could not be loaded; the others still render.
    pub error: Option<String>,
}

/// Cached for half an hour: the strips are a glance at what is new, not a live
/// feed, and each one costs a request through the browser transport.
const NEW_RELEASES_TTL_SECS: i64 = 30 * 60;
const KV_NEW_RELEASES: &str = "rutracker.new_releases";

/// Latest releases from the forums pinned on the home screen.
///
/// Browsing a forum is a search with no words, so this reuses the ordinary
/// search path with a newest-first sort.
#[tauri::command]
pub async fn home_new_releases(
    state: State<'_, Arc<AppState>>,
    refresh: bool,
) -> AppResult<Vec<NewReleaseSection>> {
    let (enabled, forums, per_forum) = {
        let cfg = state.config.read();
        (
            cfg.home.enabled,
            cfg.home.forums.clone(),
            cfg.home.per_forum.clamp(1, 50) as usize,
        )
    };
    if !enabled || forums.is_empty() {
        return Ok(Vec::new());
    }

    if !refresh {
        if let Some(cached) = read_cache_with_ttl::<Vec<NewReleaseSection>>(
            &state,
            KV_NEW_RELEASES,
            NEW_RELEASES_TTL_SECS,
        ) {
            // A cache entry made before the user edited the list would show the
            // wrong strips, so it is only reused while the shape still matches.
            if cached.len() == forums.len()
                && cached
                    .iter()
                    .zip(&forums)
                    .all(|(section, forum)| section.forum_id == forum.id)
            {
                return Ok(cached);
            }
        }
    }

    let mut sections = Vec::with_capacity(forums.len());
    for forum in forums {
        let query = SearchQuery {
            text: String::new(),
            forum_ids: vec![forum.id],
            sort: Some(crate::trackers::rutracker::SearchSort::Registered),
            ascending: false,
            page: 0,
        };
        // One dead forum must not blank the whole home screen.
        let (items, error) = match state.rutracker.search(&query).await {
            Ok(page) => (page.items.into_iter().take(per_forum).collect(), None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        sections.push(NewReleaseSection {
            forum_id: forum.id,
            forum_title: forum.title,
            items,
            error,
        });
    }

    // Only worth caching once something actually loaded.
    if sections.iter().any(|s| !s.items.is_empty()) {
        write_cache(&state, KV_NEW_RELEASES, &sections);
    }
    Ok(sections)
}

/// Every forum in the catalogue, flattened for the settings picker.
#[tauri::command]
pub async fn rutracker_all_forums(
    state: State<'_, Arc<AppState>>,
) -> AppResult<Vec<crate::trackers::rutracker::ForumEntry>> {
    let categories = match read_cache::<Vec<CatalogCategory>>(&state, KV_CATALOG) {
        Some(cached) => cached,
        None => {
            let fresh = state.rutracker.catalog().await?;
            write_cache(&state, KV_CATALOG, &fresh);
            fresh
        }
    };
    Ok(crate::trackers::rutracker::forums::flatten(&categories))
}

/// Preview artwork for one search result, for the grid view.
///
/// The result table carries no images, so this reads the topic page and takes
/// its first post image. That is a page load per card, so the answer — including
/// "this topic has no image" — is cached permanently, and the grid asks only
/// for cards the user actually scrolls to.
#[tauri::command]
pub async fn rutracker_topic_preview(
    state: State<'_, Arc<AppState>>,
    topic_id: i64,
) -> AppResult<Option<String>> {
    if let Some(cached) = state.db.get_topic_preview(topic_id)? {
        return Ok(cached);
    }
    let image = match state.rutracker.topic(topic_id).await {
        Ok(topic) => topic.images.into_iter().next(),
        // A topic that will not load is worth retrying later, so nothing is
        // written to the cache here.
        Err(e) => return Err(e),
    };
    state.db.set_topic_preview(topic_id, image.as_deref())?;
    Ok(image)
}

/// Downloads a topic's `.torrent`, starts it, and wires it into the library and
/// the update watcher in one step — this is the main "get this game" action.
#[tauri::command]
pub async fn rutracker_download(
    state: State<'_, Arc<AppState>>,
    topic_id: i64,
    output_folder: Option<String>,
    title: Option<String>,
    category: Option<String>,
) -> AppResult<AddedTorrent> {
    let bytes = state.rutracker.download_torrent(topic_id).await?;

    let added = state
        .engine
        .add(
            AddSource::Bytes(bytes.clone()),
            AddOptions {
                output_folder,
                ..Default::default()
            },
        )
        .await?;

    register(
        &state,
        &added,
        TorrentSource::Rutracker,
        Some(topic_id),
        Some(&bytes),
    )?;

    // The API would give a registration time to use as an update baseline, but
    // it is often switched off; without it the first hash change is what marks
    // a re-upload, which is still correct.
    let meta = state
        .rutracker
        .api()
        .get_topic_data(&[topic_id])
        .await
        .ok()
        .and_then(|m| m.get(&topic_id).cloned());

    let display_title = title
        .filter(|t| !t.trim().is_empty())
        .or_else(|| added.name.clone().map(|n| library::clean_title(&n)))
        .unwrap_or_else(|| format!("Раздача {topic_id}"));

    state.db.upsert_tracked_topic(
        topic_id,
        &added.info_hash,
        Some(&display_title),
        meta.as_ref()
            .and_then(|m| m.size_bytes)
            .or(Some(added.total_bytes as i64)),
        meta.as_ref().and_then(|m| m.reg_time),
    )?;

    state.db.upsert_library_item(
        &added.info_hash,
        &display_title,
        Some(&install_dir(&added)),
        category.as_deref().unwrap_or("game"),
    )?;

    Ok(added)
}

/// Registers an already-added torrent with the update watcher by looking its
/// topic up by info hash.
///
/// Only the JSON API can do this lookup, so it quietly returns `None` while the
/// tracker has that API switched off.
#[tauri::command]
pub async fn rutracker_track_existing(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> AppResult<Option<i64>> {
    let record = state
        .db
        .get_torrent(&info_hash)?
        .ok_or(AppError::TorrentNotFound)?;

    let api = state.rutracker.api();
    let Ok(Some(topic_id)) = api.topic_id_by_hash(&info_hash).await else {
        return Ok(None);
    };

    let meta = api
        .get_topic_data(&[topic_id])
        .await
        .ok()
        .and_then(|m| m.get(&topic_id).cloned());

    state.db.upsert_torrent(
        &record.info_hash,
        &record.name,
        &record.output_folder,
        record.total_bytes,
        TorrentSource::Rutracker,
        Some(topic_id),
        None,
    )?;
    state.db.upsert_tracked_topic(
        topic_id,
        &info_hash,
        meta.as_ref()
            .and_then(|m| m.topic_title.clone())
            .or(Some(record.name.clone()))
            .as_deref(),
        meta.as_ref().and_then(|m| m.size_bytes),
        meta.as_ref().and_then(|m| m.reg_time),
    )?;

    Ok(Some(topic_id))
}

// ------------------------------------------------------------------------
// Called by the injected agent inside the worker webview, not by the UI.
// Reaching these requires the remote-origin capability in
// `capabilities/tracker.json`.
// ------------------------------------------------------------------------

#[tauri::command]
pub async fn tracker_page_state(
    state: State<'_, Arc<AppState>>,
    page_state: PageState,
) -> AppResult<()> {
    state.rutracker.browser().on_page_state(page_state);
    Ok(())
}

#[tauri::command]
pub async fn tracker_job_result(
    state: State<'_, Arc<AppState>>,
    result: JobResult,
) -> AppResult<()> {
    state.rutracker.browser().on_job_result(result);
    Ok(())
}
