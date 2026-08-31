//! Video playback of torrents that are still downloading.
//!
//! The video is drawn into a window of ours: mpv gets that window's `HWND`
//! through its `wid` option and renders into it, while the interface — a
//! transparent webview stacked above it, see [`embed`] — draws the controls on
//! top.
//!
//! An earlier attempt at this failed because it leaned on mpv's own on-screen
//! controller, which never receives input when mpv does not own the window.
//! Nothing depends on it now: every control goes through the client API, so the
//! interface is ours, it is in Russian, and closing is immediate because the
//! close button is ours too.
//!
//! The media comes from [`crate::streaming`], so playback starts long before
//! the download finishes.

pub mod embed;
pub mod mpv;

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::config::PlayerConfig;
use crate::error::{AppError, AppResult};

use mpv::Mpv;

/// Label of the window the video is drawn into.
pub const PLAYER_LABEL: &str = "player";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStatus {
    /// libmpv was found and loaded.
    pub available: bool,
    /// A player instance exists and still has a file open.
    pub playing: bool,
    pub title: Option<String>,
    /// Why playback is unavailable, when it is.
    pub problem: Option<String>,
}

/// What the controls display and drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playback {
    pub title: String,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub volume: Option<f64>,
    pub muted: bool,
    /// Index in the playlist — the current episode of a season.
    pub playlist_pos: Option<usize>,
    pub playlist_count: Option<usize>,
    pub fullscreen: bool,
}

pub struct Player {
    app: AppHandle,
    instance: Mutex<Option<Mpv>>,
    /// Lets the window-close callback, which must be `'static`, reach us back.
    self_ref: RwLock<Weak<Player>>,
}

impl Player {
    pub fn new(app: AppHandle) -> Arc<Self> {
        let me = Arc::new(Self {
            app,
            instance: Mutex::new(None),
            self_ref: RwLock::new(Weak::new()),
        });
        *me.self_ref.write() = Arc::downgrade(&me);
        me
    }

    fn search_dirs(&self) -> Vec<PathBuf> {
        let resource = self.app.path().resource_dir().ok();
        mpv::search_dirs(resource.as_deref())
    }

    fn self_ref(&self) -> Option<Arc<Player>> {
        self.self_ref.read().upgrade()
    }

    /// Reports whether playback can work at all, without starting anything.
    pub fn status(&self) -> PlayerStatus {
        {
            let guard = self.instance.lock();
            if let Some(player) = guard.as_ref() {
                let title = player.get_property("media-title");
                if title.is_some() {
                    return PlayerStatus {
                        available: true,
                        playing: true,
                        title,
                        problem: None,
                    };
                }
            }
        }
        match Mpv::load(&self.search_dirs()) {
            Ok(_) => PlayerStatus {
                available: true,
                playing: false,
                title: None,
                problem: None,
            },
            Err(e) => PlayerStatus {
                available: false,
                playing: false,
                title: None,
                problem: Some(e.to_string()),
            },
        }
    }

    /// Opens a playlist in the player window, starting at `start`.
    ///
    /// A season is handed over as one playlist so playback rolls on to the next
    /// episode by itself and the controls can jump between them.
    pub async fn play(
        &self,
        items: &[String],
        start: usize,
        title: &str,
        cfg: &PlayerConfig,
    ) -> AppResult<()> {
        if items.is_empty() {
            return Err(AppError::msg("нечего воспроизводить"));
        }
        self.stop();

        let window = self.ensure_window(title)?;
        // A window built a moment ago may not have its native handle yet, and
        // failing here is what made the first click on a film report an error
        // while the second — reusing the window — worked.
        let hwnd_value = wait_for_hwnd(&window).await?;

        let mut player = Mpv::load(&self.search_dirs())?;

        // `wid` is only read before initialisation, which is why the window has
        // to exist by now.
        player.set_option("wid", &hwnd_value.to_string())?;
        player.set_option("title", &format!("{title} — Panda Torrent"))?;
        for (name, value) in cfg.mpv_options() {
            // A rejected option must never stop the film from playing.
            if let Err(e) = player.set_option(&name, &value) {
                tracing::warn!("mpv option {name}={value} rejected: {e}");
            }
        }
        player.init()?;

        for (i, url) in items.iter().enumerate() {
            let mode = if i == 0 { "replace" } else { "append" };
            player.command(&["loadfile", url, mode])?;
        }
        if start > 0 && start < items.len() {
            // Set after the whole list exists, so the index is meaningful.
            let _ = player.set_property("playlist-pos", &start.to_string());
        }

        *self.instance.lock() = Some(player);

        // mpv creates its child window a moment later, so the restacking is
        // retried until it appears — otherwise the video covers the controls.
        tauri::async_runtime::spawn(async move {
            for _ in 0..40 {
                if embed::push_video_to_back(hwnd_value) > 0 {
                    // One more nudge: mpv can recreate the surface when the
                    // first frame arrives.
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    embed::push_video_to_back(hwnd_value);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        });

        let _ = window.show();
        let _ = window.set_focus();
        Ok(())
    }

    /// The window the video is drawn into, created on first use.
    fn ensure_window(&self, title: &str) -> AppResult<WebviewWindow> {
        if let Some(existing) = self.app.get_webview_window(PLAYER_LABEL) {
            let _ = existing.set_title(&format!("{title} — Panda Torrent"));
            return Ok(existing);
        }

        let window = WebviewWindowBuilder::new(
            &self.app,
            PLAYER_LABEL,
            // The same bundle, told by its query string to render only the
            // player interface.
            WebviewUrl::App("index.html?window=player".into()),
        )
        .title(format!("{title} — Panda Torrent"))
        .inner_size(1280.0, 760.0)
        .min_inner_size(640.0, 400.0)
        .center()
        .visible(false)
        // The interface supplies its own title bar and close button, so the
        // system frame would only duplicate them. Resizing and moving are
        // handled by the grips in the page, which ask Windows to do the work.
        .decorations(false)
        .resizable(true)
        // The webview has to be see-through for the video behind it to show.
        .transparent(true)
        .build()
        .map_err(|e| AppError::msg(format!("не удалось открыть окно плеера: {e}")))?;

        // Closing the window is the same as stopping playback, and handling it
        // here means teardown starts the instant the button is pressed rather
        // than after mpv notices on its own.
        if let Some(player) = self.self_ref() {
            window.on_window_event(move |event| {
                if matches!(
                    event,
                    WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
                ) {
                    player.stop();
                }
            });
        }
        Ok(window)
    }

    /// Live playback state for the controls.
    pub fn playback(&self) -> Option<Playback> {
        let guard = self.instance.lock();
        let player = guard.as_ref()?;
        let title = player.get_property("media-title")?;
        Some(Playback {
            title,
            position: player.get_property("time-pos").and_then(|v| v.parse().ok()),
            duration: player.get_property("duration").and_then(|v| v.parse().ok()),
            paused: player
                .get_property("pause")
                .map(|v| v == "yes")
                .unwrap_or(false),
            volume: player.get_property("volume").and_then(|v| v.parse().ok()),
            muted: player
                .get_property("mute")
                .map(|v| v == "yes")
                .unwrap_or(false),
            playlist_pos: player
                .get_property("playlist-pos")
                .and_then(|v| v.parse().ok()),
            playlist_count: player
                .get_property("playlist-count")
                .and_then(|v| v.parse().ok()),
            fullscreen: player
                .get_property("fullscreen")
                .map(|v| v == "yes")
                .unwrap_or(false),
        })
    }

    /// Applies settings to a running player, so changing audio normalisation
    /// takes effect without restarting the film.
    pub fn apply(&self, cfg: &PlayerConfig) -> AppResult<()> {
        let guard = self.instance.lock();
        let Some(player) = guard.as_ref() else {
            return Ok(());
        };
        for (name, value) in cfg.mpv_properties() {
            if let Err(e) = player.set_property(&name, &value) {
                tracing::warn!("mpv property {name}={value} rejected: {e}");
            }
        }
        Ok(())
    }

    /// Sends a raw mpv command, e.g. `["cycle", "pause"]`.
    pub fn command(&self, args: &[String]) -> AppResult<()> {
        let guard = self.instance.lock();
        let player = guard
            .as_ref()
            .ok_or_else(|| AppError::msg("проигрыватель не запущен"))?;
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        player.command(&refs)
    }

    /// Stops playback and closes the player window.
    pub fn stop(&self) {
        // Taken out under the lock, dropped outside it. `mpv_terminate_destroy`
        // waits for mpv's own threads to wind down, and doing that while
        // holding the mutex blocked every other player call.
        let instance = self.instance.lock().take();
        if let Some(player) = instance {
            // Quitting first aborts the in-flight network read; without it
            // teardown waits on a stream still expecting torrent pieces.
            let _ = player.command(&["quit"]);
            std::thread::spawn(move || drop(player));
        }

        if let Some(window) = self.app.get_webview_window(PLAYER_LABEL) {
            let _ = window.close();
        }
    }

    /// Drops the handle once the player window is gone.
    ///
    /// The window is the signal, not `media-title`: that property goes missing
    /// for a moment while mpv switches episodes or re-opens the stream, and
    /// treating those moments as "closed" would tear down playback mid-film.
    pub fn reap_if_closed(&self) {
        let has_window = self.app.get_webview_window(PLAYER_LABEL).is_some();
        let has_instance = self.instance.lock().is_some();
        if has_instance && !has_window {
            self.stop();
        }
    }

    /// True while a film is open — used to keep temporary downloads alive.
    pub fn is_playing(&self) -> bool {
        self.instance.lock().is_some() && self.app.get_webview_window(PLAYER_LABEL).is_some()
    }
}

/// Waits for a freshly created window to have a usable native handle.
async fn wait_for_hwnd(window: &WebviewWindow) -> AppResult<isize> {
    let mut last = None;
    for _ in 0..40 {
        match window.hwnd() {
            Ok(hwnd) => {
                let value = hwnd.0 as isize;
                if value != 0 {
                    return Ok(value);
                }
            }
            Err(e) => last = Some(e.to_string()),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(AppError::msg(format!(
        "окно плеера не готово{}",
        last.map(|e| format!(": {e}")).unwrap_or_default()
    )))
}

/// Video extensions worth offering a "watch" button for.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "mov", "ts", "m2ts", "mpg", "mpeg", "wmv", "webm", "flv", "vob",
    "ogm", "rmvb", "divx",
];

pub fn is_video_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Releases ship a short sample next to the film; offering it as the thing
    // to watch is always wrong.
    if lower.contains("sample") {
        return false;
    }
    VIDEO_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_containers() {
        assert!(is_video_file("Film.2024.BDRip.mkv"));
        assert!(is_video_file("movie.MP4"));
        assert!(!is_video_file("readme.txt"));
        assert!(!is_video_file("cover.jpg"));
    }

    #[test]
    fn skips_sample_clips() {
        assert!(!is_video_file("Film-sample.mkv"));
    }
}
