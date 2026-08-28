// Left rail: navigation plus a quick jump list of installed games.

import { useMemo, useState } from 'react'

import { formatSpeed } from '../lib/format'
import { useStore } from '../lib/store'

export type ViewId = 'library' | 'search' | 'downloads' | 'updates' | 'settings'

const NAV: Array<{ id: ViewId; icon: string; label: string }> = [
  { id: 'library', icon: '🎮', label: 'Библиотека' },
  { id: 'search', icon: '🔍', label: 'Поиск' },
  { id: 'downloads', icon: '⬇', label: 'Загрузки' },
  { id: 'updates', icon: '🔄', label: 'Обновления' },
  { id: 'settings', icon: '⚙', label: 'Настройки' },
]

export function Sidebar({
  view,
  onNavigate,
  selectedHash,
  onSelectGame,
}: {
  view: ViewId
  onNavigate: (v: ViewId) => void
  selectedHash: string | null
  onSelectGame: (infoHash: string) => void
}) {
  const { library, progress, pendingUpdates } = useStore()
  const [filter, setFilter] = useState('')

  const active = useMemo(
    () => Object.values(progress).filter((p) => !p.finished && p.state !== 'paused'),
    [progress],
  )

  const totalDown = active.reduce((sum, p) => sum + p.downloadSpeedBps, 0)

  const games = useMemo(() => {
    const q = filter.trim().toLowerCase()
    const list = q ? library.filter((g) => g.title.toLowerCase().includes(q)) : library
    return list.slice(0, 200)
  }, [library, filter])

  const badges: Partial<Record<ViewId, { text: string; warn?: boolean }>> = {
    downloads: active.length ? { text: String(active.length) } : undefined,
    updates: pendingUpdates.length
      ? { text: String(pendingUpdates.length), warn: true }
      : undefined,
  }

  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark">🐼</span>
        <span>Panda Torrent</span>
      </div>

      <nav className="nav">
        {NAV.map((item) => {
          const badge = badges[item.id]
          return (
            <button
              key={item.id}
              className={view === item.id ? 'nav-item active' : 'nav-item'}
              onClick={() => onNavigate(item.id)}
            >
              <span className="nav-icon">{item.icon}</span>
              <span>{item.label}</span>
              {badge && (
                <span className={badge.warn ? 'nav-badge warn' : 'nav-badge'}>
                  {badge.text}
                </span>
              )}
            </button>
          )
        })}
      </nav>

      <div className="sidebar-section">Загружено ({library.length})</div>

      {library.length > 8 && (
        <div style={{ padding: '0 14px 8px' }}>
          <input
            className="input"
            placeholder="Фильтр…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
      )}

      <div className="sidebar-list">
        {games.length === 0 && (
          <div style={{ padding: '8px 12px', color: 'var(--muted)', fontSize: 12.5 }}>
            {library.length === 0 ? 'Пока пусто' : 'Ничего не найдено'}
          </div>
        )}
        {games.map((g) => (
          <button
            key={g.infoHash}
            className={
              selectedHash === g.infoHash && view === 'library'
                ? 'sidebar-game active'
                : 'sidebar-game'
            }
            onClick={() => onSelectGame(g.infoHash)}
            title={g.title}
          >
            {g.hasPendingUpdate && <span className="dot" style={{ background: 'var(--warn)' }} />}
            <span className="label">{g.title}</span>
          </button>
        ))}
      </div>

      <div className="sidebar-footer">
        <span>{active.length ? `${active.length} активных` : 'Простой'}</span>
        <span style={{ color: totalDown ? 'var(--accent)' : undefined }}>
          {totalDown ? `↓ ${formatSpeed(totalDown)}` : ''}
        </span>
      </div>
    </aside>
  )
}
