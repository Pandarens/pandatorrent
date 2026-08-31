import { useEffect, useRef, useState } from 'react'

import { Sidebar, type ViewId } from './components/Sidebar'
import { LeftoverWatches } from './components/LeftoverWatches'
import { ShutdownCountdown } from './components/ShutdownCountdown'
import { Toasts } from './components/ui'
import { UpdateModal } from './components/UpdateModal'
import { onProgress, power as powerApi } from './lib/api'
import { StoreProvider, useStore } from './lib/store'
import type { TopicUpdate } from './lib/types'
import { DownloadsView } from './views/DownloadsView'
import { LibraryView } from './views/LibraryView'
import { SearchView } from './views/SearchView'
import { SettingsView } from './views/SettingsView'
import { UpdatesView } from './views/UpdatesView'

export default function App() {
  return (
    <StoreProvider>
      <Shell />
    </StoreProvider>
  )
}

function Shell() {
  const { pendingUpdates, config } = useStore()
  const [view, setView] = useState<ViewId>('library')
  const [selectedHash, setSelectedHash] = useState<string | null>(null)

  // Surface each newly detected update once, as a modal — this is the "the
  // release was updated, want to update it?" prompt. `prompted` only guards
  // against re-asking within this session; "Later" and "Update" both record a
  // durable decision on the backend.
  const [prompt, setPrompt] = useState<TopicUpdate | null>(null)
  const prompted = useRef<Set<number>>(new Set())

  useEffect(() => {
    if (prompt) return
    const next = pendingUpdates.find((u) => !prompted.current.has(u.id))
    if (next) {
      prompted.current.add(next.id)
      setPrompt(next)
    }
  }, [pendingUpdates, prompt])

  function openGame(hash: string) {
    setSelectedHash(hash)
    setView('library')
  }

  // Turning the machine off once every download is done. Armed only from
  // settings, and only after something was actually downloading — otherwise
  // opening the application with nothing to do would count as "finished".
  const [shutdownReason, setShutdownReason] = useState<string | null>(null)
  const wasBusy = useRef(false)

  useEffect(() => {
    const un = onProgress((list) => {
      if (list.some((p) => !p.finished)) {
        wasBusy.current = true
        return
      }
      if (wasBusy.current && list.length > 0 && config?.power.afterDownloads) {
        wasBusy.current = false
        setShutdownReason('Все загрузки завершены')
      }
    })
    return () => {
      void un.then((f) => f())
    }
  }, [config?.power.afterDownloads])

  return (
    <div className="app">
      <Sidebar
        view={view}
        onNavigate={(v) => {
          setView(v)
          if (v !== 'library') setSelectedHash(null)
        }}
        selectedHash={selectedHash}
        onSelectGame={openGame}
      />

      <main className="content">
        <LeftoverWatches />
        {view === 'library' && (
          <LibraryView selectedHash={selectedHash} onSelect={setSelectedHash} />
        )}
        {view === 'search' && <SearchView onOpenLibrary={openGame} />}
        {view === 'downloads' && <DownloadsView />}
        {view === 'updates' && <UpdatesView />}
        {view === 'settings' && <SettingsView />}
      </main>

      {prompt && <UpdateModal update={prompt} onClose={() => setPrompt(null)} />}

      {shutdownReason && (
        <ShutdownCountdown
          seconds={config?.power.delaySeconds ?? 60}
          reason={shutdownReason}
          onCancel={() => {
            setShutdownReason(null)
            void powerApi.cancel()
          }}
        />
      )}

      <Toasts />
    </div>
  )
}
