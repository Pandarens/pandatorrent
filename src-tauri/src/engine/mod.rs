//! Thin facade over the librqbit BitTorrent session.
//!
//! Everything above this module speaks in the DTOs defined here, never in
//! librqbit types, so swapping the engine out later is a change confined to
//! this file.

use std::net::Ipv6Addr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, DhtSessionConfig, ListenerMode,
    ListenerOptions, Session, SessionOptions, SessionPersistenceConfig, TorrentStatsState,
    api::TorrentIdOrHash, limits::LimitsConfig,
};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};

/// Live, fast-changing state of one torrent. Static fields (name, folder,
/// tracker topic) come from the database instead, so this payload stays small
/// enough to poll once a second.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentProgress {
    pub info_hash: String,
    pub id: Option<usize>,
    pub name: Option<String>,
    /// `initializing` | `live` | `paused` | `error`
    pub state: String,
    pub finished: bool,
    pub error: Option<String>,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub download_speed_bps: u64,
    pub upload_speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub peers_live: u32,
    pub peers_seen: u32,
    pub peers_connecting: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFileEntry {
    pub index: usize,
    pub name: String,
    pub components: Vec<String>,
    pub length: u64,
    pub included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentDetails {
    pub info_hash: String,
    pub id: Option<usize>,
    pub name: Option<String>,
    pub output_folder: String,
    pub files: Vec<TorrentFileEntry>,
    pub progress: Option<TorrentProgress>,
}

/// What the user handed us to add.
pub enum AddSource {
    /// A magnet link, an `http(s)://` link to a `.torrent`, or a bare 40-char hash.
    Url(String),
    /// Raw `.torrent` bytes — this is what the RuTracker downloader produces.
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddOptions {
    pub output_folder: Option<String>,
    pub paused: bool,
    /// Restrict the download to these file indices; `None` means all files.
    pub only_files: Option<Vec<usize>>,
    /// Reuse files already on disk instead of erroring out. Set when replacing
    /// a torrent with an updated release, where most pieces are unchanged.
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedTorrent {
    pub info_hash: String,
    pub id: Option<usize>,
    pub name: Option<String>,
    pub output_folder: String,
    pub total_bytes: u64,
    pub files: Vec<TorrentFileEntry>,
    /// True when the session already had this info hash.
    pub already_present: bool,
}

/// Anything that can back a seekable HTTP response.
pub trait SeekableRead: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin + Send> SeekableRead for T {}

/// A readable, seekable view of one file inside a torrent.
///
/// librqbit does not re-export its own stream type, and keeping it out of this
/// module's public surface is what the facade is for anyway.
pub struct FileStream {
    pub reader: Box<dyn SeekableRead>,
    pub len: u64,
}

pub struct Engine {
    session: Arc<Session>,
    api: Api,
}

impl Engine {
    pub async fn start(cfg: &AppConfig, session_dir: PathBuf) -> AppResult<Arc<Self>> {
        std::fs::create_dir_all(&cfg.download_dir)?;

        let opts = SessionOptions {
            // Persisting the session lets torrents resume on the next launch
            // without re-hashing everything.
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(session_dir),
            }),
            fastresume: true,
            dht: if cfg.network.enable_dht {
                Some(DhtSessionConfig::default())
            } else {
                None
            },
            disable_local_service_discovery: !cfg.network.enable_lsd,
            peer_limit: Some(cfg.network.max_peers_per_torrent as usize),
            ratelimits: LimitsConfig {
                download_bps: kbps_to_bps(cfg.network.download_limit_kbps),
                upload_bps: kbps_to_bps(cfg.network.upload_limit_kbps),
            },
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpAndUtp,
                listen_addr: (Ipv6Addr::UNSPECIFIED, cfg.network.listen_port).into(),
                enable_upnp_port_forwarding: cfg.network.enable_upnp,
                ..Default::default()
            }),
            client_name_and_version: Some(format!(
                "Panda Torrent {}",
                env!("CARGO_PKG_VERSION")
            )),
            ..Default::default()
        };

        let session = Session::new_with_opts(cfg.download_dir.clone(), opts)
            .await
            .map_err(AppError::Engine)?;
        let api = Api::new(session.clone(), None);
        Ok(Arc::new(Self { session, api }))
    }

    pub async fn shutdown(&self) {
        self.session.stop().await;
    }

    /// Live stats for every torrent in the session, keyed by info hash.
    pub fn progress_all(&self) -> Vec<TorrentProgress> {
        self.session.with_torrents(|torrents| {
            torrents
                .map(|(id, handle)| {
                    let stats = handle.stats();
                    to_progress(
                        handle.info_hash().as_string(),
                        Some(id),
                        handle.name(),
                        stats,
                    )
                })
                .collect()
        })
    }

    pub fn progress_one(&self, info_hash: &str) -> AppResult<TorrentProgress> {
        let idx = parse_id(info_hash)?;
        let handle = self
            .session
            .get(idx)
            .ok_or(AppError::TorrentNotFound)?;
        let stats = handle.stats();
        Ok(to_progress(
            handle.info_hash().as_string(),
            Some(handle.id()),
            handle.name(),
            stats,
        ))
    }

    pub fn details(&self, info_hash: &str) -> AppResult<TorrentDetails> {
        let idx = parse_id(info_hash)?;
        let d = self
            .api
            .api_torrent_details(idx)
            .map_err(|e| AppError::Other(e.to_string()))?;
        let progress = self.progress_one(info_hash).ok();
        Ok(TorrentDetails {
            info_hash: d.info_hash,
            id: d.id,
            name: d.name,
            output_folder: d.output_folder,
            files: d
                .files
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(index, f)| TorrentFileEntry {
                    index,
                    name: f.name,
                    components: f.components,
                    length: f.length,
                    included: f.included,
                })
                .collect(),
            progress,
        })
    }

    pub async fn add(&self, source: AddSource, opts: AddOptions) -> AppResult<AddedTorrent> {
        let add = match source {
            AddSource::Url(u) => AddTorrent::from_url(u),
            AddSource::Bytes(b) => AddTorrent::from_bytes(b),
        };
        let add_opts = AddTorrentOptions {
            paused: opts.paused,
            overwrite: opts.overwrite,
            output_folder: opts.output_folder,
            only_files: opts.only_files,
            ..Default::default()
        };

        let response = self
            .session
            .add_torrent(add, Some(add_opts))
            .await
            .map_err(AppError::Engine)?;

        let (id, handle, already_present) = match response {
            AddTorrentResponse::Added(id, handle) => (Some(id), handle, false),
            AddTorrentResponse::AlreadyManaged(id, handle) => (Some(id), handle, true),
            AddTorrentResponse::ListOnly(_) => {
                return Err(AppError::msg("торрент добавлен в режиме просмотра"));
            }
        };

        // `details` needs the torrent registered in the session, which it now is.
        let info_hash = handle.info_hash().as_string();
        let details = self.details(&info_hash)?;
        let total_bytes = handle.stats().total_bytes;

        Ok(AddedTorrent {
            info_hash,
            id,
            name: handle.name(),
            output_folder: details.output_folder,
            total_bytes,
            files: details.files,
            already_present,
        })
    }

    pub async fn pause(&self, info_hash: &str) -> AppResult<()> {
        self.api
            .api_torrent_action_pause(parse_id(info_hash)?)
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        Ok(())
    }

    pub async fn resume(&self, info_hash: &str) -> AppResult<()> {
        self.api
            .api_torrent_action_start(parse_id(info_hash)?)
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        Ok(())
    }

    /// Remove from the session but keep the files on disk.
    pub async fn forget(&self, info_hash: &str) -> AppResult<()> {
        self.api
            .api_torrent_action_forget(parse_id(info_hash)?)
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        Ok(())
    }

    /// Remove from the session and delete the downloaded files.
    pub async fn delete(&self, info_hash: &str) -> AppResult<()> {
        self.api
            .api_torrent_action_delete(parse_id(info_hash)?)
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        Ok(())
    }

    pub async fn set_only_files(&self, info_hash: &str, files: Vec<usize>) -> AppResult<()> {
        self.api
            .api_torrent_action_update_only_files(parse_id(info_hash)?, &files.into_iter().collect())
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        Ok(())
    }

    /// Bytes downloaded per file, in the torrent's file order.
    ///
    /// Used to tell when the episode being watched is complete, so the next one
    /// can start downloading before the viewer gets there.
    pub fn file_progress(&self, info_hash: &str) -> AppResult<Vec<u64>> {
        let idx = parse_id(info_hash)?;
        let handle = self.session.get(idx).ok_or(AppError::TorrentNotFound)?;
        Ok(handle.stats().file_progress)
    }

    /// Waits until a torrent can actually be streamed from.
    ///
    /// `stream()` needs resolved metadata and a live torrent; a torrent that
    /// was only just added is neither. Launching the player before that made
    /// the stream server answer 404 and mpv fall back to its empty
    /// "drop files here" screen.
    pub async fn wait_until_streamable(
        &self,
        info_hash: &str,
        timeout: std::time::Duration,
    ) -> AppResult<()> {
        let started = std::time::Instant::now();
        let mut resumed = false;

        loop {
            let progress = self.progress_one(info_hash)?;

            if let Some(error) = progress.error {
                return Err(AppError::msg(format!("торрент не готов: {error}")));
            }
            // Metadata resolved and pieces flowing: good enough to open.
            if progress.state == "live" && progress.total_bytes > 0 {
                return Ok(());
            }
            // A paused torrent will never become live on its own.
            if progress.state == "paused" && !resumed {
                resumed = true;
                self.resume(info_hash).await?;
            }

            if started.elapsed() > timeout {
                return Err(AppError::msg(
                    "не удалось подготовить раздачу к просмотру — нет пиров или метаданных",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    /// A seekable reader over one file of a torrent.
    ///
    /// Reads block until the pieces they need arrive, and seeking steers the
    /// engine towards the new position — which is what makes watching a film
    /// while it downloads work at all.
    pub async fn file_stream(&self, info_hash: &str, file_id: usize) -> AppResult<FileStream> {
        let stream = self
            .api
            .api_stream(parse_id(info_hash)?, file_id)
            .await
            .map_err(|e| AppError::Other(e.to_string()))?;
        Ok(FileStream {
            len: stream.len(),
            reader: Box::new(stream),
        })
    }

    pub fn has(&self, info_hash: &str) -> bool {
        parse_id(info_hash)
            .ok()
            .and_then(|idx| self.session.get(idx))
            .is_some()
    }
}

fn kbps_to_bps(kbps: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(kbps.saturating_mul(1024))
}

fn parse_id(info_hash: &str) -> AppResult<TorrentIdOrHash> {
    TorrentIdOrHash::parse(info_hash).map_err(|_| AppError::TorrentNotFound)
}

fn to_progress(
    info_hash: String,
    id: Option<usize>,
    name: Option<String>,
    stats: librqbit::TorrentStats,
) -> TorrentProgress {
    let state = match stats.state {
        TorrentStatsState::Initializing { .. } => "initializing",
        TorrentStatsState::Live => "live",
        TorrentStatsState::Paused => "paused",
        TorrentStatsState::Error => "error",
    };

    let (down_bps, up_bps, peers_live, peers_seen, peers_connecting) = match &stats.live {
        Some(live) => {
            let p = &live.snapshot.peer_stats;
            (
                mib_per_sec_to_bps(live.download_speed.mbps),
                mib_per_sec_to_bps(live.upload_speed.mbps),
                p.live,
                p.seen,
                p.connecting,
            )
        }
        None => (0, 0, 0, 0, 0),
    };

    // librqbit exposes ETA only as an opaque display type, so derive it from
    // the numbers we already have.
    let eta_seconds = if down_bps > 0 && stats.total_bytes > stats.progress_bytes {
        Some((stats.total_bytes - stats.progress_bytes) / down_bps)
    } else {
        None
    };

    TorrentProgress {
        info_hash,
        id,
        name,
        state: state.to_string(),
        finished: stats.finished,
        error: stats.error,
        progress_bytes: stats.progress_bytes,
        total_bytes: stats.total_bytes,
        uploaded_bytes: stats.uploaded_bytes,
        download_speed_bps: down_bps,
        upload_speed_bps: up_bps,
        eta_seconds,
        peers_live,
        peers_seen,
        peers_connecting,
    }
}

fn mib_per_sec_to_bps(mib: f64) -> u64 {
    (mib * 1024.0 * 1024.0).max(0.0) as u64
}
