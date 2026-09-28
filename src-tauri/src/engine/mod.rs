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

/// How many buckets a piece map is squashed into for display.
///
/// A torrent can have tens of thousands of pieces; a strip a few hundred
/// pixels wide cannot show more than this anyway, and sending every bit for
/// every open row twice a second would be waste.
const PIECE_BUCKETS: usize = 240;

/// Which pieces of a torrent are on disk, squashed into buckets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PieceMap {
    pub total_pieces: u32,
    pub have_pieces: u32,
    /// Fill of each bucket, 0–100, left to right across the torrent.
    pub buckets: Vec<u8>,
    pub files: Vec<FilePieces>,
}

/// The same, for the pieces belonging to one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePieces {
    pub index: usize,
    pub total_pieces: u32,
    pub have_pieces: u32,
    pub buckets: Vec<u8>,
}

/// Squashes a run of pieces into display buckets.
///
/// Returns how many of them are present, and the fill of each bucket.
fn bucketize(range: std::ops::Range<usize>, is_have: &dyn Fn(usize) -> bool) -> (u32, Vec<u8>) {
    let n = range.len();
    if n == 0 {
        return (0, Vec::new());
    }
    let buckets = n.min(PIECE_BUCKETS);
    let mut fills = Vec::with_capacity(buckets);
    let mut have_total = 0u32;
    for b in 0..buckets {
        let from = range.start + b * n / buckets;
        let to = range.start + (b + 1) * n / buckets;
        let have = (from..to).filter(|&i| is_have(i)).count();
        have_total += have as u32;
        fills.push((have * 100 / (to - from).max(1)) as u8);
    }
    (have_total, fills)
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
    /// Use `output_folder` exactly as given, rather than as a root under which
    /// the release gets a folder of its own. For putting a torrent back where
    /// it already is: a re-check, a re-download, an in-place update.
    pub exact_folder: bool,
}

/// The folder a release gets under a root: its name for a multi-file torrent,
/// none for a single file — the same rule librqbit applies to its default
/// folder, and applies *only* there.
///
/// Passing a folder explicitly switched that rule off, so every multi-file
/// release was landing loose in the download folder, its episodes mixed in
/// with everything else. The smoke test caught it: a torrent added on top of
/// the files it was made from found none of them, because it was looking one
/// level up.
fn release_subfolder(torrent: &[u8]) -> Option<String> {
    let meta = librqbit::torrent_from_bytes(torrent).ok()?;
    if meta.info.data.files.is_none() {
        return None;
    }
    let raw_name = meta
        .info
        .data
        .name
        .as_ref()
        .map(|b| String::from_utf8_lossy(b.as_ref()).to_string())
        .unwrap_or_default();
    let name = folder_name_for(&raw_name);
    (!name.is_empty()).then_some(name)
}

/// A torrent name made safe as a Windows folder name.
fn folder_name_for(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 32 {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned == "." || cleaned == ".." {
        return String::new();
    }
    cleaned
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

    /// Which pieces are on disk, for the torrent and for each of its files.
    ///
    /// The picture every torrent client shows: not just "how much" but
    /// "which parts" — where the gaps are, and whether the film's first half
    /// is here yet.
    pub fn pieces(&self, info_hash: &str) -> AppResult<PieceMap> {
        let idx = parse_id(info_hash)?;
        let (have, total) = self
            .api
            .api_dump_haves(idx)
            .map_err(|e| AppError::Other(e.to_string()))?;
        let total = total as usize;
        let is_have = |i: usize| have.get(i).map(|b| *b).unwrap_or(false);

        let (have_pieces, buckets) = bucketize(0..total, &is_have);

        let files = self
            .session
            .get(idx)
            .and_then(|h| h.metadata.load_full())
            .map(|meta| {
                meta.file_infos
                    .iter()
                    .enumerate()
                    .map(|(index, info)| {
                        let range = info.piece_range.start as usize..info.piece_range.end as usize;
                        let (have_pieces, buckets) = bucketize(range.clone(), &is_have);
                        FilePieces {
                            index,
                            total_pieces: range.len() as u32,
                            have_pieces,
                            buckets,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(PieceMap {
            total_pieces: total as u32,
            have_pieces,
            buckets,
            files,
        })
    }

    /// Everything needed to put a torrent back after forgetting it.
    fn readd_plan(&self, info_hash: &str) -> AppResult<(Vec<u8>, TorrentDetails, Option<Vec<usize>>)> {
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
        Ok((bytes, details, only_files))
    }

    async fn readd(
        &self,
        bytes: Vec<u8>,
        output_folder: String,
        only_files: Option<Vec<usize>>,
    ) -> AppResult<AddedTorrent> {
        self.add(
            AddSource::Bytes(bytes),
            AddOptions {
                output_folder: Some(output_folder),
                only_files,
                overwrite: true,
                paused: false,
                // It is going back where it was; do not nest it again.
                exact_folder: true,
            },
        )
        .await
    }

    /// Re-hashes a torrent's files against the piece list.
    ///
    /// Implemented as forget-and-re-add because librqbit offers no re-check
    /// action. The files are untouched throughout; only the bookkeeping is
    /// rebuilt, which is the point.
    pub async fn recheck(&self, info_hash: &str) -> AppResult<AddedTorrent> {
        let (bytes, details, only_files) = self.readd_plan(info_hash)?;
        self.forget(info_hash).await?;
        self.readd(bytes, details.output_folder, only_files).await
    }

    /// Throws one file away and fetches it again.
    ///
    /// The same forget-and-re-add as a re-check, with the file deleted in
    /// between: the re-hash then finds its pieces missing and the download
    /// fills them back in. Deleting happens only after the engine has let go
    /// of the file — Windows will not remove a file something holds open.
    pub async fn redownload_file(
        &self,
        info_hash: &str,
        file_index: usize,
    ) -> AppResult<AddedTorrent> {
        let (bytes, details, only_files) = self.readd_plan(info_hash)?;
        let file = details
            .files
            .iter()
            .find(|f| f.index == file_index)
            .ok_or_else(|| AppError::msg("в раздаче нет такого файла"))?;
        if !file.included {
            return Err(AppError::msg("этот файл исключён из загрузки"));
        }
        let mut path = std::path::PathBuf::from(&details.output_folder);
        for part in &file.components {
            path.push(part);
        }

        self.forget(info_hash).await?;

        // The handle is released asynchronously; a few short retries cover it.
        let mut removed = Ok(());
        for attempt in 0..10 {
            removed = match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
            if removed.is_ok() {
                break;
            }
            if attempt < 9 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }

        // Whatever happened to the file, the torrent goes back: losing it
        // over a locked file would be far worse than a failed re-download.
        let added = self.readd(bytes, details.output_folder, only_files).await?;
        match removed {
            Ok(()) => {
                tracing::info!(%info_hash, file = %file.name, "файл удалён и качается заново");
                Ok(added)
            }
            Err(e) => Err(AppError::msg(format!(
                "не удалось удалить «{}»: {e}. Раздача проверена заново, но файл остался",
                file.name
            ))),
        }
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
        // The name is needed before the engine has parsed anything, so it is
        // read here. A magnet has no name until its metadata arrives; under
        // a custom root that one case lands flat, and the app never does it.
        let torrent_bytes: Option<Vec<u8>> = match &source {
            AddSource::Bytes(b) => Some(b.clone()),
            AddSource::Url(_) => None,
        };
        let output_folder = match (opts.output_folder, opts.exact_folder) {
            // No folder given: librqbit puts the release under its default
            // root, in a folder of its own.
            (None, _) => None,
            (Some(folder), true) => Some(folder),
            (Some(root), false) => {
                let sub = torrent_bytes.as_deref().and_then(release_subfolder);
                Some(match sub {
                    Some(name) => std::path::Path::new(&root)
                        .join(name)
                        .to_string_lossy()
                        .to_string(),
                    None => root,
                })
            }
        };

        let add = match source {
            AddSource::Url(u) => AddTorrent::from_url(u),
            AddSource::Bytes(b) => AddTorrent::from_bytes(b),
        };
        let add_opts = AddTorrentOptions {
            paused: opts.paused,
            overwrite: opts.overwrite,
            output_folder,
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

#[cfg(test)]
mod folder_tests {
    use super::folder_name_for;

    #[test]
    fn a_release_name_becomes_a_folder_windows_accepts() {
        assert_eq!(folder_name_for("Film (2024) WEB-DL"), "Film (2024) WEB-DL");
        assert_eq!(folder_name_for("Кто: он? / она*"), "Кто_ он_ _ она_");
        // Trailing dots and spaces are silently dropped by Windows, which
        // would make the folder impossible to find by its recorded name.
        assert_eq!(folder_name_for("Series... "), "Series");
    }

    #[test]
    fn nothing_usable_means_no_folder() {
        assert_eq!(folder_name_for(".."), "");
        assert_eq!(folder_name_for("   "), "");
    }
}
