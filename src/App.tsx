import { useEffect, useRef, useState } from 'react'

import { Sidebar, type ViewId } from './components/Sidebar'
import { Toasts } from './components/ui'
import { UpdateModal } from './components/UpdateModal'
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
  const { pendingUpdates } = useStore()
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
        {view === 'library' && (
          <LibraryView selectedHash={selectedHash} onSelect={setSelectedHash} />
        )}
        {view === 'search' && <SearchView onOpenLibrary={openGame} />}
        {view === 'downloads' && <DownloadsView />}
        {view === 'updates' && <UpdatesView />}
        {view === 'settings' && <SettingsView />}
      </main>

      {prompt && <UpdateModal update={prompt} onClose={() => setPrompt(null)} />}

      <Toasts />
    </div>
  )
}
