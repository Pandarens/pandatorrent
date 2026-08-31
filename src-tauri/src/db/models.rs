//! Row structs shared between the database layer and the Tauri command layer.
//! They are serialized straight to the frontend, hence `camelCase`.

use serde::{Deserialize, Serialize};

/// Where a torrent originally came from. Determines whether the update
/// watcher can track it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TorrentSource {
    Rutracker,
    File,
    Magnet,
    Url,
}

impl TorrentSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TorrentSource::Rutracker => "rutracker",
            TorrentSource::File => "file",
            TorrentSource::Magnet => "magnet",
            TorrentSource::Url => "url",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "rutracker" => TorrentSource::Rutracker,
            "magnet" => TorrentSource::Magnet,
            "url" => TorrentSource::Url,
            _ => TorrentSource::File,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentRecord {
    pub info_hash: String,
    pub name: String,
    pub output_folder: String,
    pub total_bytes: i64,
    pub added_at: i64,
    pub completed_at: Option<i64>,
    pub source: TorrentSource,
    pub topic_id: Option<i64>,
}

/// A card in the Steam-like library view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: i64,
    pub info_hash: String,
    pub title: String,
    pub cover_path: Option<String>,
    pub hero_path: Option<String>,
    pub exe_path: Option<String>,
    pub install_dir: Option<String>,
    pub category: String,
    pub last_played_at: Option<i64>,
    pub play_seconds: i64,
    pub favorite: bool,
    pub hidden: bool,
    pub topic_id: Option<i64>,
    /// Denormalised for the grid: avoids a per-card round trip.
    pub has_pending_update: bool,
}

/// A release the user marked to download later.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WishlistItem {
    pub topic_id: i64,
    pub title: String,
    pub image_url: Option<String>,
    pub size_bytes: Option<i64>,
    pub category: String,
    pub added_at: i64,
}

/// Something that was watched, kept or not.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchHistoryItem {
    pub id: i64,
    pub topic_id: Option<i64>,
    pub info_hash: Option<String>,
    pub title: String,
    pub file_name: Option<String>,
    pub image_url: Option<String>,
    pub watched_at: i64,
    /// It was streamed without being kept, so the files are already gone.
    pub temporary: bool,
    /// How far in the viewer had got, in seconds.
    pub position_seconds: Option<f64>,
    /// Length of the file, so the UI can tell "nearly finished" from "just
    /// started" without opening it.
    pub duration_seconds: Option<f64>,
}

/// A tracker topic being polled for re-uploads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedTopic {
    pub topic_id: i64,
    pub tracker: String,
    pub info_hash: String,
    pub title: Option<String>,
    pub size_bytes: Option<i64>,
    /// Tracker-side registration time of the torrent we currently hold. A newer
    /// value from the API is what "the release was updated" actually means.
    pub reg_time: Option<i64>,
    pub last_checked_at: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateStatus {
    Pending,
    Applied,
    Dismissed,
}

impl UpdateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpdateStatus::Pending => "pending",
            UpdateStatus::Applied => "applied",
            UpdateStatus::Dismissed => "dismissed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "applied" => UpdateStatus::Applied,
            "dismissed" => UpdateStatus::Dismissed,
            _ => UpdateStatus::Pending,
        }
    }
}

/// A detected re-upload: same topic, different info hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicUpdate {
    pub id: i64,
    pub topic_id: i64,
    pub title: Option<String>,
    pub old_info_hash: String,
    pub new_info_hash: String,
    pub old_size_bytes: Option<i64>,
    pub new_size_bytes: Option<i64>,
    pub new_reg_time: Option<i64>,
    pub detected_at: i64,
    pub status: UpdateStatus,
}
