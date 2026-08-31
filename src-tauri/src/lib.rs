pub mod commands;
pub mod config;
pub mod db;
pub mod engine;
pub mod error;
pub mod library;
pub mod player;
pub mod state;
pub mod streaming;
pub mod trackers;
pub mod updates;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use config::AppConfig;
use db::Db;
use engine::Engine;
use state::{AppState, TempWatchAction, events};
use streaming::StreamServer;
use trackers::rutracker::RutrackerClient;

/// How often live download stats are pushed to the UI.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

/// How often to look when there is nothing to watch — the window is away in
/// the tray, or every torrent is just seeding. A second-by-second update costs
/// a redraw of the whole list, and in the tray nobody sees it at all.
const IDLE_PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // A second launch — double-clicking a .torrent in Explorer, say —
            // hands its arguments to the running instance instead of starting
            // another session.
            show_main_window(app);
            open_from_arguments(app, &argv);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Before the state, so that anything going wrong while it is
            // built is written down rather than lost.
            if let Some(logs) = init_logging(&app_data_dir(app.handle())) {
                log_panics(logs);
            }
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "запуск");

            let state = init_state(app.handle())?;
            app.manage(state.clone());

            setup_tray(app.handle())?;
            spawn_progress_emitter(app.handle().clone(), state.clone());
            spawn_browser_reaper(state.clone());
            spawn_temp_watch_reaper(state.clone());

            // Launched by double-clicking a .torrent or a magnet link.
            let argv: Vec<String> = std::env::args().collect();
            open_from_arguments(app.handle(), &argv);
            updates::spawn_watcher(app.handle().clone(), state);

            Ok(())
        })
        .on_window_event(|window, event| {
            // The hidden tracker worker window closes normally; only the main
            // window is diverted to the tray.
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let minimize = window
                    .app_handle()
                    .try_state::<Arc<AppState>>()
                    .map(|s| s.config.read().ui.minimize_to_tray)
                    .unwrap_or(false);
                if minimize {
                    // Keep seeding in the background instead of quitting.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::torrents::torrents_list,
            commands::torrents::torrents_progress,
            commands::torrents::torrent_details,
            commands::torrents::torrent_add_url,
            commands::torrents::torrent_add_file,
            commands::torrents::torrent_pause,
            commands::torrents::torrent_resume,
            commands::torrents::torrent_remove,
            commands::torrents::torrent_set_files,
            commands::torrents::torrent_open_folder,
            commands::tracker::rutracker_status,
            commands::tracker::rutracker_verify,
            commands::tracker::rutracker_open_login,
            commands::tracker::rutracker_hide_login,
            commands::tracker::rutracker_selftest,
            commands::tracker::rutracker_logout,
            commands::tracker::rutracker_search,
            commands::tracker::rutracker_topic,
            commands::tracker::rutracker_catalog,
            commands::tracker::rutracker_all_forums,
            commands::tracker::home_new_releases,
            commands::tracker::rutracker_topic_preview,
            commands::tracker::rutracker_download,
            commands::tracker::rutracker_track_existing,
            commands::tracker::tracker_page_state,
            commands::tracker::tracker_job_result,
            commands::library::library_list,
            commands::library::library_add,
            commands::library::library_scan_executables,
            commands::library::library_set_exe,
            commands::library::library_set_title,
            commands::library::library_set_flag,
            commands::library::library_launch,
            commands::library::library_open_folder,
            commands::library::library_fetch_cover,
            commands::library::wishlist_list,
            commands::library::wishlist_add,
            commands::library::wishlist_remove,
            commands::updates::updates_list,
            commands::updates::updates_pending_count,
            commands::updates::updates_check_now,
            commands::updates::updates_apply,
            commands::updates::updates_dismiss,
            commands::updates::updates_set_topic_enabled,
            commands::updates::updates_tracked_topics,
            commands::settings::settings_get,
            commands::settings::settings_set,
            commands::settings::settings_mirrors,
            commands::settings::app_info,
            commands::settings::logs_open,
            commands::app_update::app_update_check,
            commands::app_update::app_update_install,
            commands::player::player_status,
            commands::player::player_video_files,
            commands::player::player_playback,
            commands::player::player_play,
            commands::player::player_stop,
            commands::player::player_command,
            commands::player::player_watch_topic,
            commands::player::history_list,
            commands::player::history_remove,
            commands::player::history_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Where application data lives.
///
/// Tauri's own app-data directory keeps the asset-protocol scope (`$APPDATA`)
/// and the cover cache in agreement.
fn app_data_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| state::resolve_data_dir())
}

/// Records panics in the log folder before the process goes down.
///
/// The release profile aborts on panic and is stripped of symbols, so a crash
/// otherwise leaves nothing behind but a Windows error code — which is exactly
/// what the first one did. The line is written straight to the file rather
/// than through `tracing`, because the log writer is asynchronous and an abort
/// never gives it the chance to flush.
fn log_panics(logs_dir: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "место неизвестно".to_string());
        let line = format!("unix={seconds}  {location}  {info}\n");

        use std::io::Write;
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("panic.log"))
            .and_then(|mut f| f.write_all(line.as_bytes()));

        tracing::error!(%location, "паника: {info}");
        previous(info);
    }));
}

/// Log files older than this are removed on startup.
const LOG_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Starts writing the log to a file under the data directory.
///
/// Before this existed nothing set up a subscriber, so every `tracing::warn!`
/// in the code went nowhere and a fault the user hit left nothing to read
/// afterwards. Faults in here are swallowed on purpose: failing to open a log
/// file is not a reason to refuse to start.
fn init_logging(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = data_dir.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    prune_old_logs(&dir);

    let appender = tracing_appender::rolling::daily(&dir, "panda-torrent.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    // The guard flushes on drop, and it has to outlive every log call, so it
    // is deliberately kept for the lifetime of the process.
    std::mem::forget(guard);

    // librqbit is chatty at info level; the app's own messages are the point.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,librqbit=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok()?;

    Some(dir)
}

/// Keeps the log folder from growing without end.
fn prune_old_logs(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|age| age > LOG_RETENTION).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn init_state(app: &AppHandle) -> Result<Arc<AppState>, Box<dyn std::error::Error>> {
    let data_dir = app_data_dir(app);
    std::fs::create_dir_all(&data_dir)?;

    let config_path = state::config_path(&data_dir);
    let cfg = AppConfig::load(&config_path);
    // Materialise defaults on first run so the file is there to edit.
    let _ = cfg.save(&config_path);

    let db = Db::open(&state::db_path(&data_dir))?;

    let engine = tauri::async_runtime::block_on(Engine::start(
        &cfg,
        state::session_dir(&data_dir),
    ))?;

    // The worker webview keeps its own persistent cookie jar, so the tracker
    // session survives restarts without the app ever handling a credential.
    let rutracker = Arc::new(RutrackerClient::new(
        app.clone(),
        &cfg.rutracker,
        cfg.network.tracker_proxy.as_deref(),
    )?);

    // Anything left in the stream cache belongs to a previous run and will
    // never be resumed, so the cheapest correct thing is to drop it at start.
    let cache = data_dir.join("cache").join("stream");
    if cache.exists() {
        if let Err(e) = std::fs::remove_dir_all(&cache) {
            tracing::warn!("could not clear the stream cache: {e}");
        }
    }

    let streams = Arc::new(tauri::async_runtime::block_on(StreamServer::start(
        engine.clone(),
    ))?);
    let player = player::Player::new(app.clone());

    Ok(Arc::new(AppState {
        app_handle: app.clone(),
        db,
        config: RwLock::new(cfg),
        config_path,
        data_dir,
        engine,
        rutracker,
        streams,
        player,
        temp_watch: parking_lot::Mutex::new(None),
        now_playing: parking_lot::Mutex::new(None),
    }))
}

/// Pushes live stats to the UI and turns "finished" into a one-shot event.
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

fn spawn_progress_emitter(app: AppHandle, state: Arc<AppState>) {
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

        let mut delay = PROGRESS_INTERVAL;
        let mut was_watched = true;

        loop {
            tokio::time::sleep(delay).await;

            let watched = main_window_is_watched(&app);
            let progress = state.engine.progress_all();

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

            // Coming back from the tray gets an immediate update rather than
            // showing figures up to five seconds stale.
            if watched && !progress.is_empty() {
                let _ = app.emit(events::PROGRESS, &progress);
            }

            let moving = progress.iter().any(|p| !p.finished || p.download_speed_bps > 0);
            delay = if watched && (moving || !was_watched) {
                PROGRESS_INTERVAL
            } else {
                IDLE_PROGRESS_INTERVAL
            };
            was_watched = watched;
        }
    });
}

/// Closes the tracker's browser window once it has been idle, so a client that
/// is only seeding does not hold a WebView2 instance it is not using.
fn spawn_browser_reaper(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            state.rutracker.browser().close_if_idle();
        }
    });
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

/// How long a "just watch it" download survives after the film is closed.
const TEMP_WATCH_GRACE: Duration = Duration::from_secs(5 * 60);

/// Deletes films that were streamed without being kept, once they have been
/// closed for [`TEMP_WATCH_GRACE`].
///
/// The delay is deliberate: reopening a film a minute later should not have to
/// download it again.
fn spawn_temp_watch_reaper(state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;

            // Frees the mpv handle as soon as its window is gone, which is also
            // what starts this grace period.
            let was_playing = state.player.is_playing();
            state.player.reap_if_closed();
            if was_playing && !state.player.is_playing() {
                // Playback just ended: release the reads it was holding.
                state.streams.abort_all();
                *state.now_playing.lock() = None;
            }

            let playing = state.player.is_playing();
            if playing {
                save_watch_position(&state);
            }
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
                    (Some(TempWatchAction::Delete), _) => {
                        expired = guard.take().map(|w| w.info_hash)
                    }
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
                    tracing::warn!("could not {} temporary stream: {e}", if pause { "pause" } else { "resume" });
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
    });
}

/// Adds any `.torrent` files or magnet links given on the command line.
///
/// This is what makes "open with Panda Torrent" and the file association work:
/// Windows simply launches the app with the file as an argument, and a second
/// launch forwards its arguments to the instance already running.
fn open_from_arguments(app: &AppHandle, argv: &[String]) {
    let targets: Vec<String> = argv
        .iter()
        .skip(1)
        .filter(|arg| is_openable(arg))
        .cloned()
        .collect();
    if targets.is_empty() {
        return;
    }

    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let state = state.inner().clone();
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        for target in targets {
            let result = if target.starts_with("magnet:") {
                commands::torrents::add_url(&state, &target).await
            } else {
                commands::torrents::add_path(&state, &target).await
            };
            match result {
                Ok(name) => {
                    let _ = app.emit(events::TORRENT_ADDED, &name);
                    tracing::info!("added from command line: {name}");
                }
                Err(e) => tracing::warn!("could not open {target}: {e}"),
            }
        }
    });
}

fn is_openable(arg: &str) -> bool {
    arg.starts_with("magnet:") || arg.to_lowercase().ends_with(".torrent")
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Открыть Panda Torrent", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::with_id("panda-tray")
        .tooltip("Panda Torrent")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
