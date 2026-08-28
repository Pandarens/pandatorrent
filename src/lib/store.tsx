// App-wide state: the few collections every view needs, plus toasts.
//
// Deliberately a plain context rather than a state library — the data set is
// small and the live-updating part (download progress) is handled by a single
// event subscription that never re-renders anything else.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'

import {
  asAppError,
  library as libraryApi,
  onProgress,
  onTorrentCompleted,
  onTorrentAdded,
  onTrackerAttention,
  onTrackerAuth,
  onUpdateCheckState,
  onUpdatesFound,
  settings as settingsApi,
  torrents as torrentsApi,
  tracker as trackerApi,
  updates as updatesApi,
} from './api'
import type {
  AppConfig,
  LibraryItem,
  TopicUpdate,
  TorrentProgress,
  TorrentView,
  TrackerStatus,
} from './types'

export type ToastKind = 'info' | 'error' | 'warn'

export interface Toast {
  id: number
  text: string
  kind: ToastKind
}

interface Store {
  library: LibraryItem[]
  torrents: TorrentView[]
  progress: Record<string, TorrentProgress>
  pendingUpdates: TopicUpdate[]
  trackerStatus: TrackerStatus | null
  config: AppConfig | null
  checkingUpdates: boolean
  toasts: Toast[]

  refreshLibrary: () => Promise<void>
  refreshTorrents: () => Promise<void>
  refreshUpdates: () => Promise<void>
  refreshTracker: () => Promise<void>
  refreshConfig: () => Promise<void>
  refreshAll: () => Promise<void>

  toast: (text: string, kind?: ToastKind) => void
  /** Reports a rejected command as a toast and returns the parsed error. */
  reportError: (e: unknown, prefix?: string) => ReturnType<typeof asAppError>
}

const StoreContext = createContext<Store | null>(null)

export function StoreProvider({ children }: { children: ReactNode }) {
  const [library, setLibrary] = useState<LibraryItem[]>([])
  const [torrents, setTorrents] = useState<TorrentView[]>([])
  const [progress, setProgress] = useState<Record<string, TorrentProgress>>({})
  const [pendingUpdates, setPendingUpdates] = useState<TopicUpdate[]>([])
  const [trackerStatus, setTrackerStatus] = useState<TrackerStatus | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [checkingUpdates, setCheckingUpdates] = useState(false)
  const [toasts, setToasts] = useState<Toast[]>([])

  const nextToastId = useRef(1)

  const toast = useCallback((text: string, kind: ToastKind = 'info') => {
    const id = nextToastId.current++
    setToasts((t) => [...t, { id, text, kind }])
    window.setTimeout(() => {
      setToasts((t) => t.filter((x) => x.id !== id))
    }, 6000)
  }, [])

  const reportError = useCallback(
    (e: unknown, prefix?: string) => {
      const err = asAppError(e)
      toast(prefix ? `${prefix}: ${err.message}` : err.message, 'error')
      return err
    },
    [toast],
  )

  const refreshLibrary = useCallback(async () => {
    try {
      setLibrary(await libraryApi.list(false))
    } catch (e) {
      reportError(e, 'Библиотека')
    }
  }, [reportError])

  const refreshTorrents = useCallback(async () => {
    try {
      setTorrents(await torrentsApi.list())
    } catch (e) {
      reportError(e, 'Загрузки')
    }
  }, [reportError])

  const refreshUpdates = useCallback(async () => {
    try {
      setPendingUpdates(await updatesApi.list(true))
    } catch (e) {
      reportError(e, 'Обновления')
    }
  }, [reportError])

  const refreshTracker = useCallback(async () => {
    try {
      setTrackerStatus(await trackerApi.status())
    } catch (e) {
      reportError(e, 'Трекер')
    }
  }, [reportError])

  const refreshConfig = useCallback(async () => {
    try {
      setConfig(await settingsApi.get())
    } catch (e) {
      reportError(e, 'Настройки')
    }
  }, [reportError])

  const refreshAll = useCallback(async () => {
    await Promise.all([
      refreshLibrary(),
      refreshTorrents(),
      refreshUpdates(),
      refreshTracker(),
      refreshConfig(),
    ])
  }, [refreshLibrary, refreshTorrents, refreshUpdates, refreshTracker, refreshConfig])

  useEffect(() => {
    void refreshAll()
  }, [refreshAll])

  // Live wiring. Progress arrives once a second and only touches its own slice
  // of state, so the rest of the tree stays still.
  useEffect(() => {
    const unlisteners: Array<Promise<() => void>> = [
      onProgress((list) => {
        setProgress(Object.fromEntries(list.map((p) => [p.infoHash.toUpperCase(), p])))
      }),
      onTorrentCompleted((p) => {
        toast(`Загрузка завершена: ${p.name ?? p.infoHash}`)
        void refreshTorrents()
        void refreshLibrary()
      }),
      onUpdatesFound(() => {
        void refreshUpdates()
        void refreshLibrary()
      }),
      onUpdateCheckState((s) => setCheckingUpdates(s === 'checking')),
      // The worker browser window reports sign-in changes on its own, including
      // a session that expired mid-search.
      onTrackerAuth(() => {
        void refreshTracker()
      }),
      onTrackerAttention((message) => toast(message, 'warn')),
      // Opened from Explorer or a magnet link while the app was already running.
      onTorrentAdded((name) => {
        toast(`Открыт торрент: ${name}`)
        void refreshAll()
      }),
    ]

    return () => {
      unlisteners.forEach((p) => {
        void p.then((un) => un())
      })
    }
  }, [toast, refreshTorrents, refreshLibrary, refreshUpdates, refreshTracker, refreshAll])

  const value = useMemo<Store>(
    () => ({
      library,
      torrents,
      progress,
      pendingUpdates,
      trackerStatus,
      config,
      checkingUpdates,
      toasts,
      refreshLibrary,
      refreshTorrents,
      refreshUpdates,
      refreshTracker,
      refreshConfig,
      refreshAll,
      toast,
      reportError,
    }),
    [
      library,
      torrents,
      progress,
      pendingUpdates,
      trackerStatus,
      config,
      checkingUpdates,
      toasts,
      refreshLibrary,
      refreshTorrents,
      refreshUpdates,
      refreshTracker,
      refreshConfig,
      refreshAll,
      toast,
      reportError,
    ],
  )

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>
}

export function useStore(): Store {
  const ctx = useContext(StoreContext)
  if (!ctx) throw new Error('useStore must be used inside StoreProvider')
  return ctx
}
