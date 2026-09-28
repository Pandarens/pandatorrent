pub mod commands;
pub mod config;
pub mod db;
pub mod engine;
pub mod error;
pub mod housekeeping;
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
use state::{AppState, events};
use streaming::StreamServer;
use trackers::rutracker::RutrackerClient;

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
        // `--minimised` is how the Windows autostart entry asks for a quiet
        // launch: straight to the tray, no window in the face at login.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimised"]),
        ))
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
            let state_for_boot = state.clone();

            setup_tray(app.handle())?;
            housekeeping::spawn(app.handle().clone(), state.clone());

            // Launched by double-clicking a .torrent or a magnet link.
            let argv: Vec<String> = std::env::args().collect();
            open_from_arguments(app.handle(), &argv);

            apply_autostart(app.handle(), state_for_boot.config.read().ui.autostart);

            // Started by Windows at login, or asked to keep out of the way.
            let quiet = argv.iter().any(|a| a == "--minimised")
                || state_for_boot.config.read().ui.start_minimized;
            if quiet {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            updates::spawn_watcher(app.handle().clone(), state);

            Ok(())
        })
        .on_window_event(|window, event| {
            // The hidden tracker worker window closes normally; only the main
            // window is diverted to the tray.
            if window.label() != "main" {
                return;
            }
            if matches!(event, WindowEvent::Destroyed) {
                tracing::info!("главное окно уничтожено");
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
            commands::torrents::torrent_recheck,
            commands::torrents::torrent_set_no_seeding,
            commands::torrents::torrent_set_forced,
            commands::torrents::torrent_pieces,
            commands::torrents::torrent_redownload_file,
            commands::torrents::torrent_peers,
            commands::torrents::torrent_create,
            commands::torrents::session_stats,
            commands::torrents::torrent_resume,
            commands::torrents::torrent_remove,
            commands::torrents::torrent_set_files,
            commands::torrents::torrent_open_folder,
            commands::tracker::rutracker_status,
            commands::tracker::rutracker_verify,
            commands::tracker::rutracker_open_login,
            commands::tracker::rutracker_show_window,
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
            commands::settings::settings_export,
            commands::settings::settings_import,
            commands::leftovers::leftovers_list,
            commands::leftovers::leftover_resume,
            commands::leftovers::leftover_drop,
            commands::leftovers::leftover_save,
            commands::power::system_shutdown,
            commands::power::system_shutdown_cancel,
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
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| match event {
            // A quiet exit used to leave nothing in the log at all — the
            // process was simply gone. Now the reason is written down first.
            tauri::RunEvent::ExitRequested { code, .. } => {
                tracing::info!(?code, "запрошен выход из приложения");
            }
            tauri::RunEvent::Exit => {
                tracing::info!("приложение завершается");
                release_temp_watch(app);
            }
            _ => {}
        });
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

/// Sorts out what a previous run left in the stream cache.
///
/// A viewing whose files are gone is a phantom: it used to come back as a
/// download pointing at nothing, re-checking itself on every launch. Those are
/// dropped without ceremony.
///
/// A viewing whose files are still there is left exactly as it is, and the
/// interface offers it back. Deleting it silently would throw away a
/// part-downloaded film that somebody restarted their computer intending to
/// finish.
fn tidy_stream_cache(db: &Db, engine: &Arc<Engine>, cache: &std::path::Path) {
    let Ok(rows) = db.list_torrents() else {
        return;
    };

    let mut keep: HashSet<String> = HashSet::new();

    for row in rows
        .into_iter()
        .filter(|r| std::path::Path::new(&r.output_folder).starts_with(cache))
    {
        let details = engine.details(&row.info_hash).ok();
        let root = std::path::Path::new(&row.output_folder);

        // Files are wherever the engine says they are, relative to the
        // torrent's own folder — which is the release's folder under the
        // cache for anything added lately, and the cache root itself for a
        // viewing from before releases got folders of their own.
        let on_disk = details
            .as_ref()
            .map(|d| {
                d.files.iter().any(|f| {
                    let mut path = root.to_path_buf();
                    for part in &f.components {
                        path.push(part);
                    }
                    path.exists()
                })
            })
            .unwrap_or(false);

        // What in the cache root belongs to this torrent: its folder, or, for
        // the old flat layout, its files' top-level names.
        let owned: Vec<String> = match root
            .strip_prefix(cache)
            .ok()
            .and_then(|rel| rel.components().next())
            .map(|c| c.as_os_str().to_string_lossy().to_string())
        {
            Some(first) if !first.is_empty() => vec![first],
            _ => details
                .as_ref()
                .map(|d| {
                    d.files
                        .iter()
                        .filter_map(|f| f.components.first().cloned())
                        .collect()
                })
                .unwrap_or_default(),
        };

        if on_disk {
            keep.extend(owned);
            tracing::info!(name = %row.name, "просмотр с прошлого раза ждёт решения");
            continue;
        }

        tracing::info!(name = %row.name, "убираем просмотр без файлов");
        // `forget` rather than `delete`: there is nothing left to delete, and
        // asking the engine to remove missing files only produces noise.
        if let Err(e) = tauri::async_runtime::block_on(engine.forget(&row.info_hash)) {
            tracing::warn!("не удалось убрать просмотр из сессии: {e}");
        }
        if let Err(e) = db.delete_torrent(&row.info_hash) {
            tracing::warn!("не удалось убрать просмотр из базы: {e}");
        }
    }

    // Whatever is in the cache and belongs to no torrent is debris from a run
    // that ended badly, and nothing will ever ask for it again.
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if keep.contains(&name) {
            continue;
        }
        let path = entry.path();
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = removed {
            tracing::warn!("не удалось убрать остаток кэша {}: {e}", path.display());
        }
    }
}

/// Frees the viewing in progress when the application closes.
///
/// Waiting for the grace period is pointless once there is nobody left to come
/// back to it, and leaving it behind is what turns a scratch copy into
/// gigabytes nobody asked to keep.
fn release_temp_watch(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let Some(watch) = state.temp_watch.lock().take() else {
        return;
    };
    tracing::info!(info_hash = %watch.info_hash, "освобождаем просмотр при выходе");
    let engine = state.engine.clone();
    let hash = watch.info_hash.clone();
    if let Err(e) = tauri::async_runtime::block_on(engine.delete(&hash)) {
        tracing::warn!("не удалось освободить просмотр при выходе: {e}");
    }
    let _ = state.db.delete_torrent(&hash);
}

fn init_state(app: &AppHandle) -> Result<Arc<AppState>, Box<dyn std::error::Error>> {
    let data_dir = app_data_dir(app);
    std::fs::create_dir_all(&data_dir)?;

    let config_path = state::config_path(&data_dir);
    let cfg = AppConfig::load(&config_path);
    // Materialise defaults on first run so the file is there to edit.
    let _ = cfg.save(&config_path);

    // Startup phases are timed permanently: a slow launch is the kind of
    // thing that creeps in unnoticed, and the log is where it should show.
    let started = std::time::Instant::now();
    let mut last = started;
    let mut phase = |name: &str| {
        let now = std::time::Instant::now();
        tracing::info!(
            phase = name,
            ms = now.duration_since(last).as_millis() as u64,
            total_ms = now.duration_since(started).as_millis() as u64,
            "запуск: фаза"
        );
        last = now;
    };

    let db = Db::open(&state::db_path(&data_dir))?;
    phase("база");

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

    phase("движок");
    tidy_stream_cache(&db, &engine, &data_dir.join("cache").join("stream"));
    phase("уборка кэша");

    let streams = Arc::new(tauri::async_runtime::block_on(StreamServer::start(
        engine.clone(),
    ))?);
    phase("сервер потока");
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

/// Registers or removes the "start with Windows" entry.
///
/// Best effort: a machine that refuses the registry write is not a reason to
/// fail the launch, but it is a reason to say so in the log.
pub fn apply_autostart(app: &AppHandle, wanted: bool) {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();

    // Asking to remove an entry that is not there fails with "file not found",
    // and this runs on every launch — so the state is checked first. That also
    // keeps the registry untouched when nothing needs to change.
    match manager.is_enabled() {
        Ok(current) if current == wanted => return,
        Ok(_) => {}
        Err(e) => tracing::warn!("не удалось прочитать состояние автозапуска: {e}"),
    }

    let result = if wanted {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        tracing::warn!("не удалось изменить автозапуск: {e}");
    } else {
        tracing::info!(wanted, "автозапуск изменён");
    }
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

#[cfg(test)]
mod acl_tests {
    //! Keeps the command list and the ACL from drifting apart.
    //!
    //! A command that is registered but not granted compiles, launches, and
    //! looks entirely healthy — then fails the moment somebody presses the
    //! button, with "not allowed by ACL". Fifteen commands were in that state
    //! before this test existed.

    /// Command names inside `generate_handler![...]`.
    fn registered() -> Vec<String> {
        let source = include_str!("lib.rs");
        let start = source
            .find("generate_handler![")
            .expect("нет списка команд")
            + "generate_handler![".len();
        let end = start + source[start..].find(']').expect("список не закрыт");
        source[start..end]
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| c.rsplit("::").next().unwrap_or(c).to_string())
            .collect()
    }

    /// Every name quoted in the permission file.
    fn granted() -> Vec<String> {
        include_str!("../permissions/app.toml")
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|s| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn every_registered_command_is_granted() {
        let granted = granted();
        let ungranted: Vec<String> = registered()
            .into_iter()
            .filter(|c| !granted.contains(c))
            .collect();
        assert!(
            ungranted.is_empty(),
            "эти команды зарегистрированы, но не разрешены в permissions/app.toml: {ungranted:?}"
        );
    }

    #[test]
    fn nothing_is_granted_that_does_not_exist() {
        // A stale grant is a permission nobody revoked, which is worth
        // noticing even though it cannot be exploited on its own.
        let registered = registered();
        let stale: Vec<String> = granted()
            .into_iter()
            .filter(|c| !registered.contains(c))
            .collect();
        assert!(stale.is_empty(), "разрешены несуществующие команды: {stale:?}");
    }
}
