//! Shared application state, owned by Tauri and handed to every command.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

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
}

pub struct AppState {
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
