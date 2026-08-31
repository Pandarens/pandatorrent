//! Shared application state, owned by Tauri and handed to every command.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::config::AppConfig;
use crate::db::Db;
use crate::engine::Engine;
use crate::player::Player;
use crate::streaming::StreamServer;
use crate::error::AppResult;
use crate::trackers::rutracker::RutrackerClient;

/// Events pushed to the frontend. Kept as constants so the TypeScript side and
/// the Rust side cannot drift apart silently.
pub mod events {
    pub const PROGRESS: &str = "torrents:progress";
    pub const TORRENT_COMPLETED: &str = "torrent:completed";
    pub const UPDATES_FOUND: &str = "updates:found";
    pub const UPDATE_CHECK_STATE: &str = "updates:check-state";
    /// A torrent arrived from outside the UI — a file association or a magnet.
    pub const TORRENT_ADDED: &str = "torrent:added";
    /// A watch-history row changed, e.g. its artwork finally arrived.
    pub const HISTORY_UPDATED: &str = "history:updated";
}

pub struct AppState {
    /// Kept so background tasks can emit events without a command's handle.
    pub app_handle: tauri::AppHandle,
    pub db: Db,
    pub config: RwLock<AppConfig>,
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub engine: Arc<Engine>,
    pub rutracker: Arc<RutrackerClient>,
    /// Serves torrent files over loopback HTTP so a player can seek in them.
    pub streams: Arc<StreamServer>,
    pub player: Arc<Player>,
    /// The one release being watched without being kept, if any.
    pub temp_watch: parking_lot::Mutex<Option<TempWatch>>,
    /// What the player currently has open.
    pub now_playing: parking_lot::Mutex<Option<NowPlaying>>,
}

/// The torrent and files behind the current playback session.
#[derive(Clone)]
pub struct NowPlaying {
    pub info_hash: String,
    /// File indices in playlist order, so index N here is episode N there.
    pub files: Vec<usize>,
}

/// A release streamed straight into the cache instead of the library.
///
/// It is deleted once the film has been closed for a while, so "just watch
/// this" does not quietly fill the disk.
pub struct TempWatch {
    pub info_hash: String,
    /// Last moment the player was seen with this film open.
    pub last_active: Instant,
    /// Downloading was already stopped when the player closed, so it is not
    /// paused again on every sweep.
    pub paused: bool,
}

/// What to do with a temporary download on one sweep of the reaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempWatchAction {
    /// Leave it exactly as it is.
    Idle,
    /// Viewing resumed — let it download again.
    Resume,
    /// The player is gone — stop pulling data, but keep the files.
    Pause,
    /// Closed for longer than the grace period — delete it.
    Delete,
}

impl TempWatch {
    /// Decides what this sweep should do, and records the decision.
    ///
    /// This is deliberately separate from the background loop that calls it.
    /// The mistake worth guarding against — deciding a film is finished while
    /// it is still on screen — is invisible inside a loop that sleeps for
    /// twenty seconds, and obvious in a test. `playing` is the caller's answer
    /// to "is the film still open", and everything here follows from it.
    pub fn sweep(&mut self, now: Instant, playing: bool, grace: Duration) -> TempWatchAction {
        if playing {
            // Still watching: the grace period never starts, however long the
            // film runs.
            self.last_active = now;
            if self.paused {
                self.paused = false;
                return TempWatchAction::Resume;
            }
            return TempWatchAction::Idle;
        }

        if !self.paused {
            // Waiting out the grace period before pausing would keep
            // downloading a film nobody is watching any more.
            self.paused = true;
            return TempWatchAction::Pause;
        }

        if now.saturating_duration_since(self.last_active) > grace {
            return TempWatchAction::Delete;
        }
        TempWatchAction::Idle
    }
}

impl AppState {
    pub fn config_snapshot(&self) -> AppConfig {
        self.config.read().clone()
    }

    pub fn save_config(&self) -> AppResult<()> {
        let cfg = self.config.read();
        cfg.save(&self.config_path)
    }

    /// Where downloaded cover art is cached.
    pub fn covers_dir(&self) -> PathBuf {
        self.data_dir.join("covers")
    }

    /// Scratch space for films watched without being kept.
    pub fn stream_cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache").join("stream")
    }

    pub fn proxy(&self) -> Option<String> {
        self.config.read().network.tracker_proxy.clone()
    }
}

/// Resolves the per-user data directory, creating it if needed.
///
/// Only a fallback: normally Tauri's own app-data directory is used, so that
/// the asset-protocol scope (`$APPDATA`) and the cover cache agree.
pub fn resolve_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("PandaTorrent");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("config.json")
}

pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("panda.db")
}

pub fn session_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("session")
}


#[cfg(test)]
mod temp_watch_tests {
    use super::*;

    const GRACE: Duration = Duration::from_secs(5 * 60);

    fn watch() -> TempWatch {
        TempWatch {
            info_hash: "abc".into(),
            last_active: Instant::now(),
            paused: false,
        }
    }

    #[test]
    fn a_film_being_watched_is_never_reaped() {
        // The regression this exists for: a long film was torn down mid-play
        // because the sweep decided it had been closed. As long as the player
        // reports it open, no amount of elapsed time may delete it.
        let mut w = watch();
        let start = Instant::now();
        for minute in 1..=180 {
            let now = start + Duration::from_secs(minute * 60);
            assert_eq!(
                w.sweep(now, true, GRACE),
                TempWatchAction::Idle,
                "reaped {minute} minutes into a film that was still playing"
            );
        }
    }

    #[test]
    fn closing_the_player_stops_the_download_before_deleting_it() {
        let mut w = watch();
        let start = Instant::now();

        // First sweep after the window closes: stop pulling data, keep files.
        assert_eq!(w.sweep(start, false, GRACE), TempWatchAction::Pause);
        // Inside the grace period nothing more happens.
        assert_eq!(
            w.sweep(start + Duration::from_secs(60), false, GRACE),
            TempWatchAction::Idle
        );
        // Past it, the files go.
        assert_eq!(
            w.sweep(start + GRACE + Duration::from_secs(1), false, GRACE),
            TempWatchAction::Delete
        );
    }

    #[test]
    fn reopening_the_film_revives_it_and_restarts_the_clock() {
        let mut w = watch();
        let start = Instant::now();
        assert_eq!(w.sweep(start, false, GRACE), TempWatchAction::Pause);

        // Reopened four minutes later — inside the grace period.
        let reopened = start + Duration::from_secs(4 * 60);
        assert_eq!(w.sweep(reopened, true, GRACE), TempWatchAction::Resume);

        // The clock restarted, so the original deadline no longer applies.
        assert_eq!(
            w.sweep(start + GRACE + Duration::from_secs(1), false, GRACE),
            TempWatchAction::Pause
        );
    }

    #[test]
    fn a_brief_hiccup_in_the_playing_signal_does_not_delete_anything() {
        // The player briefly reporting "not playing" costs at most a pause,
        // which the next sweep undoes.
        let mut w = watch();
        let start = Instant::now();
        assert_eq!(w.sweep(start, false, GRACE), TempWatchAction::Pause);
        assert_eq!(
            w.sweep(start + Duration::from_secs(20), true, GRACE),
            TempWatchAction::Resume
        );
    }
}
