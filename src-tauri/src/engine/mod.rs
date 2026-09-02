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
    /// Bytes of this file already on disk, so a download can be inspected
    /// file by file the way any torrent client shows it.
    pub downloaded: u64,
}

/// One peer we are connected to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerView {
    pub address: String,
    /// What the other end says it is running, when it says.
    pub client: Option<String>,
    pub state: String,
    pub downloaded: u64,
    pub uploaded: u64,
}

/// Session-wide totals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub download_speed_bps: u64,
    pub upload_speed_bps: u64,
    pub uptime_seconds: u64,
    /// Nodes in the DHT routing table — a rough health signal.
    pub dht_nodes: u64,
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
    /// Reuse files already on disk instead of refusing to add the torrent.
    ///
    /// This is what every torrent client does: the existing files are hashed
    /// against the piece list, whatever matches counts as downloaded, and only
    /// the rest is fetched. Adding with this off meant a release already on
    /// disk started again from nothing.
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

    /// Builds a `.torrent` from a file or folder on disk.
    ///
    /// Free-standing rather than a method: making a torrent needs no session,
    /// and pretending otherwise would tie it to a running engine for nothing.
    pub async fn create_torrent_bytes(
        path: &std::path::Path,
        name: Option<String>,
        trackers: Vec<String>,
    ) -> AppResult<Vec<u8>> {
        let options = librqbit::CreateTorrentOptions {
            name: name.as_deref(),
            trackers,
            // librqbit picks a piece length to suit the size.
            piece_length: None,
        };
        // Hashing a folder is heavy and blocking; four threads is plenty for
        // a one-off and leaves the rest of the machine alone.
        let spawner = librqbit::spawn_utils::BlockingSpawner::new(4);
        let result = librqbit::create_torrent(path, options, &spawner)
            .await
            .map_err(|e| AppError::msg(format!("не удалось собрать торрент: {e}")))?;
        let bytes = result
            .as_bytes()
            .map_err(|e| AppError::msg(format!("не удалось записать торрент: {e}")))?;
        Ok(bytes.to_vec())
    }

    /// Who we are exchanging pieces with, for one torrent.
    pub fn peers(&self, info_hash: &str) -> AppResult<Vec<PeerView>> {
        let idx = parse_id(info_hash)?;
        // The filter type is not exported by librqbit, but it implements
        // `Default`, and the parameter position tells the compiler which type
        // that is — so it can be asked for without ever naming it.
        let snapshot = self
            .api
            .api_peer_stats(idx, Default::default())
            .map_err(|e| AppError::Other(e.to_string()))?;

        let mut peers: Vec<PeerView> = snapshot
            .peers
            .into_iter()
            .map(|(address, stats)| PeerView {
                address,
                client: stats.client_name,
                state: stats.state.to_string(),
                downloaded: stats.counters.fetched_bytes,
                uploaded: stats.counters.uploaded_bytes,
            })
            .collect();
        // Busiest first: that is the interesting end of the list.
        peers.sort_by(|a, b| b.downloaded.cmp(&a.downloaded));
        Ok(peers)
    }

    /// Totals for the whole session, for the status line.
    pub fn session_stats(&self) -> SessionSummary {
        let stats = self.api.api_session_stats();
        SessionSummary {
            download_speed_bps: (stats.download_speed.mbps * 125_000.0) as u64,
            upload_speed_bps: (stats.upload_speed.mbps * 125_000.0) as u64,
            uptime_seconds: stats.uptime_seconds,
            dht_nodes: self.api.api_dht_stats().map(|d| (d.routing_table_size + d.routing_table_size_v6) as u64).unwrap_or(0),
        }
    }

    /// Changes the speed limits of a running session.
    ///
    /// These do not need a restart, contrary to what the settings screen used
    /// to claim: the session exposes its limiter, and a schedule that could
    /// only take effect at launch would be no schedule at all.
    pub fn set_rate_limits(&self, download_kbps: u32, upload_kbps: u32) {
        self.session
            .ratelimits
            .set_download_bps(kbps_to_bps(download_kbps));
        self.session
            .ratelimits
            .set_upload_bps(kbps_to_bps(upload_kbps));
        tracing::info!(download_kbps, upload_kbps, "лимиты скорости применены");
    }

    /// The original `.torrent` of something already in the session.
    ///
    /// Needed to re-add a torrent, which is how a re-check is done: librqbit
    /// has no re-check of its own, and hashing the files is exactly what it
    /// does when a torrent is added onto files that are already there.
    pub fn torrent_bytes(&self, info_hash: &str) -> Option<Vec<u8>> {
        let idx = parse_id(info_hash).ok()?;
        let handle = self.session.get(idx)?;
        let metadata = handle.metadata.load_full()?;
        Some(metadata.torrent_bytes.to_vec())
    }

    /// Re-hashes a torrent's files against the piece list.
    ///
    /// Implemented as forget-and-re-add because librqbit offers no re-check
    /// action. The files are untouched throughout; only the bookkeeping is
    /// rebuilt, which is the point.
    pub async fn recheck(&self, info_hash: &str) -> AppResult<AddedTorrent> {
        let bytes = self
            .torrent_bytes(info_hash)
            .ok_or_else(|| AppError::msg("торрент ещё не готов к проверке"))?;
        let details = self.details(info_hash)?;

        // Keep the file selection: a release narrowed to one episode must not
        // silently widen to the whole season because it was re-checked.
        let only_files: Vec<usize> = details
            .files
            .iter()
            .filter(|f| f.included)
            .map(|f| f.index)
            .collect();
        let only_files = (only_files.len() < details.files.len()).then_some(only_files);

        self.forget(info_hash).await?;
        self.add(
            AddSource::Bytes(bytes),
            AddOptions {
                output_folder: Some(details.output_folder),
                only_files,
                overwrite: true,
                paused: false,
            },
        )
        .await
    }

    pub fn details(&self, info_hash: &str) -> AppResult<TorrentDetails> {
        let idx = parse_id(info_hash)?;
        let d = self
            .api
            .api_torrent_details(idx)
            .map_err(|e| AppError::Other(e.to_string()))?;
        let progress = self.progress_one(info_hash).ok();
        let done = self.file_progress(info_hash).unwrap_or_default();
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
                    downloaded: done.get(index).copied().unwrap_or(0),
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
