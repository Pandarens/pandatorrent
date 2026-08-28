// TypeScript mirrors of the Rust DTOs. Field names match the `camelCase`
// serde renaming used on the Rust side.

export type TorrentSource = 'rutracker' | 'file' | 'magnet' | 'url'
export type TorrentState = 'initializing' | 'live' | 'paused' | 'error'
export type UpdateStatus = 'pending' | 'applied' | 'dismissed'

/** Every rejected `invoke` resolves to this shape. */
export interface AppError {
  kind:
    | 'other'
    | 'not_authenticated'
    | 'bad_credentials'
    | 'api_unavailable'
    | 'tracker_unreachable'
    | 'parse'
    | 'torrent_not_found'
    | 'network'
    | 'db'
    | 'io'
    | 'engine'
  message: string
}

export interface TorrentProgress {
  infoHash: string
  id: number | null
  name: string | null
  state: TorrentState
  finished: boolean
  error: string | null
  progressBytes: number
  totalBytes: number
  uploadedBytes: number
  downloadSpeedBps: number
  uploadSpeedBps: number
  etaSeconds: number | null
  peersLive: number
  peersSeen: number
  peersConnecting: number
}

export interface TorrentFileEntry {
  index: number
  name: string
  components: string[]
  length: number
  included: boolean
}

export interface TorrentDetails {
  infoHash: string
  id: number | null
  name: string | null
  outputFolder: string
  files: TorrentFileEntry[]
  progress: TorrentProgress | null
}

/** Database row plus live stats, flattened by serde on the Rust side. */
export interface TorrentView {
  infoHash: string
  name: string
  outputFolder: string
  totalBytes: number
  addedAt: number
  completedAt: number | null
  source: TorrentSource
  topicId: number | null
  progress: TorrentProgress | null
}

export interface AddedTorrent {
  infoHash: string
  id: number | null
  name: string | null
  outputFolder: string
  totalBytes: number
  files: TorrentFileEntry[]
  alreadyPresent: boolean
}

export interface LibraryItem {
  id: number
  infoHash: string
  title: string
  coverPath: string | null
  heroPath: string | null
  exePath: string | null
  installDir: string | null
  category: string
  lastPlayedAt: number | null
  playSeconds: number
  favorite: boolean
  hidden: boolean
  topicId: number | null
  hasPendingUpdate: boolean
}

export interface ExecutableCandidate {
  path: string
  fileName: string
  sizeBytes: number
  depth: number
  isInstaller: boolean
  score: number
}

export type SearchSort =
  | 'registered'
  | 'title'
  | 'downloads'
  | 'size'
  | 'seeders'
  | 'leechers'

export interface SearchQuery {
  text: string
  forumIds: number[]
  sort: SearchSort | null
  ascending: boolean
  page: number
}

export interface SearchItem {
  topicId: number
  title: string
  forumId: number | null
  forumName: string | null
  author: string | null
  sizeBytes: number | null
  seeders: number
  leechers: number
  downloads: number
  registeredAt: number | null
  approved: boolean
}

export interface SearchPage {
  items: SearchItem[]
  total: number | null
  page: number
  pageSize: number
}

export interface ForumEntry {
  id: number
  title: string
}

/** A forum plus its subforums, e.g. "Игры для Windows" → "Новинки". */
export interface CatalogForum {
  id: number
  title: string
  subforums: ForumEntry[]
}

export interface CatalogCategory {
  id: number
  title: string
  forums: CatalogForum[]
}

/** One "what is new" strip on the home screen. */
export interface NewReleaseSection {
  forumId: number
  forumTitle: string
  items: SearchItem[]
  /** Set when this strip failed; the others still render. */
  error: string | null
}

/** A piece of a forum post. Spoilers nest, exactly as on the site. */
export type PostBlock =
  | { kind: 'text'; text: string }
  | { kind: 'image'; url: string }
  | { kind: 'spoiler'; title: string; blocks: PostBlock[] }

export interface TopicComment {
  author: string | null
  postedAt: string | null
  blocks: PostBlock[]
}

export interface TopicDetails {
  topicId: number
  title: string
  /** Structured opening post: text, images and spoilers. */
  blocks: PostBlock[]
  /** Flattened text, for places that need a single string. */
  description: string
  images: string[]
  magnet: string | null
  infoHash: string | null
  sizeBytes: number | null
  comments: TopicComment[]
}

export interface TrackerStatus {
  username: string | null
  /** The worker browser page reports an active tracker session. */
  hasSession: boolean
  host: string
  /** A Cloudflare interstitial is currently on screen. */
  challenged: boolean
  /** The webview holds a bb_session cookie — a hint, not proof. */
  hasCookie: boolean
  /** hasSession came from a live page, not from the remembered flag. */
  verified: boolean
}

export interface TopicUpdate {
  id: number
  topicId: number
  title: string | null
  oldInfoHash: string
  newInfoHash: string
  oldSizeBytes: number | null
  newSizeBytes: number | null
  newRegTime: number | null
  detectedAt: number
  status: UpdateStatus
}

export interface TrackedTopic {
  topicId: number
  tracker: string
  infoHash: string
  title: string | null
  sizeBytes: number | null
  regTime: number | null
  lastCheckedAt: number | null
  enabled: boolean
}

/** Which source the last update check managed to use. */
export type CheckMethod = 'api' | 'pages'

export interface CheckOutcome {
  checked: number
  method: CheckMethod
  /** Topics left for the next run because of the per-run page-check cap. */
  deferred: number
  newUpdates: TopicUpdate[]
  missingTopics: number[]
  /** Explains e.g. why the slower page-based fallback ran. */
  note: string | null
}

export interface NetworkConfig {
  listenPort: number
  enableDht: boolean
  enableUpnp: boolean
  enableLsd: boolean
  downloadLimitKbps: number
  uploadLimitKbps: number
  maxPeersPerTorrent: number
  trackerProxy: string | null
}

export interface RutrackerConfig {
  username: string | null
  host: string
  /** When a session was last confirmed; survives restarts. */
  loggedInAt: number | null
}

export interface UpdatesConfig {
  enabled: boolean
  intervalMinutes: number
  checkOnStartup: boolean
  autoDownload: boolean
  notifyDesktop: boolean
}

export interface FeaturedForum {
  id: number
  title: string
}

export interface HomeConfig {
  enabled: boolean
  forums: FeaturedForum[]
  perForum: number
}

/** Playback settings, translated into mpv options by the backend. */
export interface PlayerConfig {
  /** 'off' | 'dynaudnorm' | 'loudnorm' */
  audioNormalize: string
  volume: number
  hardwareDecoding: boolean
  subtitleLang: string
  audioLang: string
  onScreenControls: boolean
  extraOptions: string[]
}

/** Live playback state, driving the app's own control bar. */
export interface Playback {
  title: string
  position: number | null
  duration: number | null
  paused: boolean
  volume: number | null
  muted: boolean
  /** Current episode index within the playlist. */
  playlistPos: number | null
  playlistCount: number | null
  fullscreen: boolean

  // Streaming stats, flattened in by the backend. Absent when the app does not
  // know which torrent is behind the picture.
  /** Download speed of the torrent being streamed, bytes per second. */
  downloadSpeedBps?: number | null
  peers?: number | null
  /** Bytes of the current episode already on disk. */
  fileDownloaded?: number | null
  fileTotal?: number | null
  /** The next episode is already fully downloaded. */
  nextReady?: boolean
}

/** Something that was watched, kept or not. */
export interface WatchHistoryItem {
  id: number
  topicId: number | null
  infoHash: string | null
  title: string
  fileName: string | null
  imageUrl: string | null
  watchedAt: number
  /** Streamed without being kept — the files are already gone. */
  temporary: boolean
}

export interface PlayerStatus {
  available: boolean
  playing: boolean
  /** What is on screen right now, when something is. */
  title: string | null
  problem: string | null
}

/** A release marked to download later. */
export interface WishlistItem {
  topicId: number
  title: string
  imageUrl: string | null
  sizeBytes: number | null
  category: string
  addedAt: number
}

export interface UiConfig {
  minimizeToTray: boolean
  startMinimized: boolean
  autostart: boolean
  libraryView: string
  /** 'list' | 'grid' for tracker search results. */
  searchView: string
}

export interface AppConfig {
  downloadDir: string
  network: NetworkConfig
  rutracker: RutrackerConfig
  updates: UpdatesConfig
  ui: UiConfig
  home: HomeConfig
  player: PlayerConfig
}

export interface SettingsUpdate {
  config: AppConfig
  restartRequired: boolean
}

/** Result of the tracker-connection diagnostic. */
export interface SelfTest {
  ok: boolean
  hasCookie: boolean
  challenged: boolean
  loggedIn: boolean
  bytes: number
  url: string
  message: string
}

/** A newer release on GitHub, if there is one. */
export interface AppUpdate {
  available: boolean
  currentVersion: string
  version: string | null
  notes: string | null
  publishedAt: string | null
  /** Explains an inconclusive check, e.g. nothing published yet. */
  message: string | null
}

export interface AppInfo {
  version: string
  dataDir: string
  coversDir: string
}
