// Typed wrappers around the Tauri command layer.
//
// Everything the UI does goes through this file, so command names and argument
// shapes live in exactly one place.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  PeerView,
  SessionSummary,
  Leftover,
  AddedTorrent,
  CatalogCategory,
  ForumEntry,
  NewReleaseSection,
  Playback,
  PlayerStatus,
  AppConfig,
  AppError,
  AppInfo,
  AppUpdate,
  CheckOutcome,
  ExecutableCandidate,
  LibraryItem,
  SearchPage,
  SearchQuery,
  SelfTest,
  SettingsUpdate,
  TopicDetails,
  TopicUpdate,
  TorrentDetails,
  TorrentFileEntry,
  TorrentProgress,
  TorrentView,
  TrackedTopic,
  TrackerStatus,
  WatchHistoryItem,
  WishlistItem,
} from './types'

/** Narrows an unknown rejection into the error shape the backend sends. */
export function asAppError(e: unknown): AppError {
  if (e && typeof e === 'object' && 'kind' in e && 'message' in e) {
    return e as AppError
  }
  return {
    kind: 'other',
    message: e instanceof Error ? e.message : String(e),
  }
}

export const events = {
  progress: 'torrents:progress',
  torrentCompleted: 'torrent:completed',
  updatesFound: 'updates:found',
  updateCheckState: 'updates:check-state',
  /** Login state changed in the worker browser window. */
  trackerAuth: 'tracker:auth',
  /** The worker window needs the user: a Cloudflare check or a sign-in. */
  trackerAttention: 'tracker:attention',
  /** A torrent arrived from a file association or a magnet link. */
  torrentAdded: 'torrent:added',
  /** A watch-history row changed, e.g. its artwork arrived. */
  historyUpdated: 'history:updated',
} as const

export function onProgress(cb: (p: TorrentProgress[]) => void): Promise<UnlistenFn> {
  return listen<TorrentProgress[]>(events.progress, (e) => cb(e.payload))
}

export function onTorrentCompleted(
  cb: (p: TorrentProgress) => void,
): Promise<UnlistenFn> {
  return listen<TorrentProgress>(events.torrentCompleted, (e) => cb(e.payload))
}

export function onUpdatesFound(cb: (u: TopicUpdate[]) => void): Promise<UnlistenFn> {
  return listen<TopicUpdate[]>(events.updatesFound, (e) => cb(e.payload))
}

export function onUpdateCheckState(cb: (s: string) => void): Promise<UnlistenFn> {
  return listen<string>(events.updateCheckState, (e) => cb(e.payload))
}

export function onTrackerAuth(cb: (loggedIn: boolean) => void): Promise<UnlistenFn> {
  return listen<boolean>(events.trackerAuth, (e) => cb(e.payload))
}

export function onHistoryUpdated(cb: () => void): Promise<UnlistenFn> {
  return listen(events.historyUpdated, () => cb())
}

export function onTorrentAdded(cb: (name: string) => void): Promise<UnlistenFn> {
  return listen<string>(events.torrentAdded, (e) => cb(e.payload))
}

export function onTrackerAttention(cb: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>(events.trackerAttention, (e) => cb(e.payload))
}

// ------------------------------------------------------------------ torrents

export const torrents = {
  list: () => invoke<TorrentView[]>('torrents_list'),
  progress: () => invoke<TorrentProgress[]>('torrents_progress'),
  details: (infoHash: string) => invoke<TorrentDetails>('torrent_details', { infoHash }),
  addUrl: (url: string, outputFolder?: string) =>
    invoke<AddedTorrent>('torrent_add_url', { url, outputFolder: outputFolder ?? null }),
  addFile: (path: string, outputFolder?: string) =>
    invoke<AddedTorrent>('torrent_add_file', { path, outputFolder: outputFolder ?? null }),
  pause: (infoHash: string) => invoke<void>('torrent_pause', { infoHash }),
  /** Re-hash the files on disk against the torrent's piece list. */
  recheck: (infoHash: string) =>
    invoke<AddedTorrent>('torrent_recheck', { infoHash }),
  setNoSeeding: (infoHash: string, on: boolean) =>
    invoke<void>('torrent_set_no_seeding', { infoHash, on }),
  setForced: (infoHash: string, on: boolean) =>
    invoke<void>('torrent_set_forced', { infoHash, on }),
  peers: (infoHash: string) => invoke<PeerView[]>('torrent_peers', { infoHash }),
  sessionStats: () => invoke<SessionSummary>('session_stats'),
  create: (source: string, saveTo: string, name: string | null, trackers: string[]) =>
    invoke<void>('torrent_create', { source, saveTo, name, trackers }),
  resume: (infoHash: string) => invoke<void>('torrent_resume', { infoHash }),
  remove: (infoHash: string, deleteFiles: boolean) =>
    invoke<void>('torrent_remove', { infoHash, deleteFiles }),
  setFiles: (infoHash: string, files: number[]) =>
    invoke<void>('torrent_set_files', { infoHash, files }),
  openFolder: (infoHash: string) => invoke<void>('torrent_open_folder', { infoHash }),
}

// ------------------------------------------------------------------- tracker

export const tracker = {
  /** Cached state — no network round trip. */
  status: () => invoke<TrackerStatus>('rutracker_status'),
  /** Loads a forum page to confirm the session really is alive. */
  verify: () => invoke<TrackerStatus>('rutracker_verify'),
  /**
   * Shows the tracker's own login page in the worker window. Cloudflare only
   * lets a real browser through, so this is the only way in.
   */
  openLogin: () => invoke<void>('rutracker_open_login'),
  hideLogin: () => invoke<void>('rutracker_hide_login'),
  /** End-to-end check of the browser transport, for the settings screen. */
  selftest: () => invoke<SelfTest>('rutracker_selftest'),
  logout: () => invoke<TrackerStatus>('rutracker_logout'),
  search: (query: SearchQuery) => invoke<SearchPage>('rutracker_search', { query }),
  topic: (topicId: number) => invoke<TopicDetails>('rutracker_topic', { topicId }),
  /** Whole section tree for browsing without a query; cached backend-side. */
  catalog: (refresh = false) => invoke<CatalogCategory[]>('rutracker_catalog', { refresh }),
  /** Every forum, flattened — for the settings picker. */
  allForums: () => invoke<ForumEntry[]>('rutracker_all_forums'),
  /** Latest releases from the forums pinned to the home screen. */
  newReleases: (refresh = false) =>
    invoke<NewReleaseSection[]>('home_new_releases', { refresh }),
  /** Preview image for one result. Costs a topic page load on a cache miss. */
  topicPreview: (topicId: number) =>
    invoke<string | null>('rutracker_topic_preview', { topicId }),
  download: (opts: {
    topicId: number
    outputFolder?: string | null
    title?: string | null
    category?: string | null
  }) =>
    invoke<AddedTorrent>('rutracker_download', {
      topicId: opts.topicId,
      outputFolder: opts.outputFolder ?? null,
      title: opts.title ?? null,
      category: opts.category ?? null,
    }),
  trackExisting: (infoHash: string) =>
    invoke<number | null>('rutracker_track_existing', { infoHash }),
}

// ------------------------------------------------------------------- library

export const library = {
  list: (includeHidden = false) => invoke<LibraryItem[]>('library_list', { includeHidden }),
  add: (infoHash: string, title?: string, category?: string) =>
    invoke<number>('library_add', {
      infoHash,
      title: title ?? null,
      category: category ?? null,
    }),
  scanExecutables: (infoHash: string) =>
    invoke<ExecutableCandidate[]>('library_scan_executables', { infoHash }),
  setExe: (infoHash: string, exePath: string | null) =>
    invoke<void>('library_set_exe', { infoHash, exePath }),
  setTitle: (infoHash: string, title: string) =>
    invoke<void>('library_set_title', { infoHash, title }),
  setFlag: (infoHash: string, flag: 'favorite' | 'hidden', value: boolean) =>
    invoke<void>('library_set_flag', { infoHash, flag, value }),
  launch: (infoHash: string) => invoke<void>('library_launch', { infoHash }),
  openFolder: (infoHash: string) => invoke<void>('library_open_folder', { infoHash }),
  fetchCover: (infoHash: string, imageUrl?: string) =>
    invoke<{ path: string }>('library_fetch_cover', {
      infoHash,
      imageUrl: imageUrl ?? null,
    }),
}

// -------------------------------------------------------------------- player

export const player = {
  status: () => invoke<PlayerStatus>('player_status'),
  /** Position, duration and playlist state for the control bar. */
  playback: () => invoke<Playback | null>('player_playback'),
  /** Video files in a torrent, biggest first. */
  videoFiles: (infoHash: string) =>
    invoke<TorrentFileEntry[]>('player_video_files', { infoHash }),
  /** Streams one file through the local server and opens it in mpv. */
  play: (infoHash: string, fileId: number) =>
    invoke<void>('player_play', { infoHash, fileId }),
  /**
   * Watches a release without keeping it: it streams into a scratch folder that
   * is cleared a few minutes after the film is closed, and at startup.
   */
  watchTopic: (topicId: number, title?: string) =>
    invoke<void>('player_watch_topic', { topicId, title: title ?? null }),
  stop: () => invoke<void>('player_stop'),
  command: (args: string[]) => invoke<void>('player_command', { args }),

  // Thin wrappers so the UI never has to know mpv's command vocabulary.
  togglePause: () => invoke<void>('player_command', { args: ['cycle', 'pause'] }),
  seekBy: (seconds: number) =>
    invoke<void>('player_command', { args: ['seek', String(seconds), 'relative'] }),
  seekTo: (seconds: number) =>
    invoke<void>('player_command', { args: ['seek', String(seconds), 'absolute'] }),
  setVolume: (volume: number) =>
    invoke<void>('player_command', { args: ['set', 'volume', String(volume)] }),
  toggleMute: () => invoke<void>('player_command', { args: ['cycle', 'mute'] }),
  setTrack: (kind: 'aid' | 'sid', id: number | 'no') =>
    invoke<void>('player_command', { args: ['set', kind, String(id)] }),
  setSpeed: (speed: number) =>
    invoke<void>('player_command', { args: ['set', 'speed', String(speed)] }),
  setSubDelay: (seconds: number) =>
    invoke<void>('player_command', { args: ['set', 'sub-delay', seconds.toFixed(1)] }),
  toggleFullscreen: () => invoke<void>('player_command', { args: ['cycle', 'fullscreen'] }),
  nextEpisode: () =>
    invoke<void>('player_command', { args: ['playlist-next', 'weak'] }),
  prevEpisode: () => invoke<void>('player_command', { args: ['playlist-prev', 'weak'] }),
  goToEpisode: (index: number) =>
    invoke<void>('player_command', { args: ['set', 'playlist-pos', String(index)] }),
}

// ------------------------------------------------------------------- history

export const history = {
  list: () => invoke<WatchHistoryItem[]>('history_list'),
  remove: (id: number) => invoke<void>('history_remove', { id }),
  clear: () => invoke<void>('history_clear'),
}

// ------------------------------------------------------------------ wishlist

export const wishlist = {
  list: () => invoke<WishlistItem[]>('wishlist_list'),
  add: (item: {
    topicId: number
    title: string
    imageUrl?: string | null
    sizeBytes?: number | null
    category?: string | null
  }) =>
    invoke<void>('wishlist_add', {
      topicId: item.topicId,
      title: item.title,
      imageUrl: item.imageUrl ?? null,
      sizeBytes: item.sizeBytes ?? null,
      category: item.category ?? null,
    }),
  remove: (topicId: number) => invoke<void>('wishlist_remove', { topicId }),
}

// ------------------------------------------------------------------- updates

export const updates = {
  list: (onlyPending = true) => invoke<TopicUpdate[]>('updates_list', { onlyPending }),
  pendingCount: () => invoke<number>('updates_pending_count'),
  checkNow: () => invoke<CheckOutcome>('updates_check_now'),
  apply: (updateId: number) => invoke<AddedTorrent>('updates_apply', { updateId }),
  dismiss: (updateId: number) => invoke<void>('updates_dismiss', { updateId }),
  setTopicEnabled: (topicId: number, enabled: boolean) =>
    invoke<void>('updates_set_topic_enabled', { topicId, enabled }),
  trackedTopics: () => invoke<TrackedTopic[]>('updates_tracked_topics'),
}

// ------------------------------------------------------------------ settings

export const settings = {
  get: () => invoke<AppConfig>('settings_get'),
  set: (config: AppConfig) => invoke<SettingsUpdate>('settings_set', { config }),
  mirrors: () => invoke<string[]>('settings_mirrors'),
  appInfo: () => invoke<AppInfo>('app_info'),
  openLogs: () => invoke<void>('logs_open'),
  exportTo: (path: string) => invoke<void>('settings_export', { path }),
  importFrom: (path: string) => invoke<AppConfig>('settings_import', { path }),
}

// ------------------------------------------------------- application update

export const appUpdate = {
  /** Asks GitHub whether a newer release exists. */
  check: () => invoke<AppUpdate>('app_update_check'),
  /** Downloads, installs and restarts. Only signed artifacts are accepted. */
  install: () => invoke<void>('app_update_install'),
  onProgress: (cb: (percent: number) => void) =>
    listen<number>('app-update:progress', (e) => cb(e.payload)),
}

// ------------------------------------------------------- unfinished viewings

export const leftovers = {
  list: () => invoke<Leftover[]>('leftovers_list'),
  resume: (infoHash: string) => invoke<void>('leftover_resume', { infoHash }),
  drop: (infoHash: string) => invoke<void>('leftover_drop', { infoHash }),
  /** Moves it into the download folder; resolves to where it landed. */
  save: (infoHash: string) => invoke<string>('leftover_save', { infoHash }),
}

// ------------------------------------------------------------------- power

export const power = {
  shutdown: () => invoke<void>('system_shutdown'),
  cancel: () => invoke<void>('system_shutdown_cancel'),
}
