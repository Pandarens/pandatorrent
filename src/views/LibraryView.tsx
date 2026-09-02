// The Steam-like part: a grid of cover cards, and a hero page per game.

import { useEffect, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

import { library as libraryApi, settings as settingsApi, torrents as torrentsApi } from '../lib/api'
import { formatBytes, formatDate, formatPlaytime, progressPercent } from '../lib/format'
import { useStore } from '../lib/store'
import type { ExecutableCandidate, LibraryItem, TopicUpdate } from '../lib/types'
import { Empty, Modal, ProgressBar, Spinner } from '../components/ui'
import { ContinueWatching } from '../components/ContinueWatching'
import { Wishlist } from '../components/Wishlist'
import { ConfirmDialog } from '../components/ConfirmDialog'
import { WatchHistory } from '../components/WatchHistory'
import { WatchButton } from '../components/WatchButton'
import { UpdateModal } from '../components/UpdateModal'

/** Local files can only be shown through Tauri's asset protocol. */
function coverSrc(path: string | null): string | null {
  return path ? convertFileSrc(path) : null
}

export function LibraryView({
  selectedHash,
  onSelect,
}: {
  selectedHash: string | null
  onSelect: (hash: string | null) => void
}) {
  const { library } = useStore()
  const selected = useMemo(
    () => library.find((g) => g.infoHash === selectedHash) ?? null,
    [library, selectedHash],
  )

  if (selected) {
    return <GameDetail game={selected} onBack={() => onSelect(null)} />
  }
  return <LibraryGrid onSelect={onSelect} />
}

function LibraryGrid({ onSelect }: { onSelect: (hash: string) => void }) {
  const { library, progress, config, reportError } = useStore()

  // Remembered in settings, the same way the search view already is.
  const asList = config?.ui.libraryView === 'list'
  async function setLibraryView(mode: 'grid' | 'list') {
    if (!config) return
    try {
      await settingsApi.set({ ...config, ui: { ...config.ui, libraryView: mode } })
    } catch (e) {
      reportError(e, 'Вид библиотеки')
    }
  }
  const [query, setQuery] = useState('')
  const [onlyFavorites, setOnlyFavorites] = useState(false)

  const items = useMemo(() => {
    const q = query.trim().toLowerCase()
    return library.filter(
      (g) => (!q || g.title.toLowerCase().includes(q)) && (!onlyFavorites || g.favorite),
    )
  }, [library, query, onlyFavorites])

  if (library.length === 0) {
    return (
      <div className="page">
        <ContinueWatching />
        <ContinueWatching />
      <Wishlist onOpenGame={onSelect} />
        <WatchHistory />
        <Empty
          icon="🎮"
          title="Библиотека пуста"
          hint="Найдите раздачу на вкладке «Поиск» — скачанное появится здесь карточками."
        />
      </div>
    )
  }

  return (
    <div className="page">
      <ContinueWatching />
      <Wishlist onOpenGame={onSelect} />
      <WatchHistory />

      <div className="page-head">
        <h1 className="page-title">Библиотека</h1>
        <span className="page-sub">{items.length} из {library.length}</span>
        <div className="spacer" />
        <button
          className={onlyFavorites ? 'btn primary sm' : 'btn sm'}
          onClick={() => setOnlyFavorites((v) => !v)}
        >
          ★ Избранное
        </button>
        <button
          className="btn sm"
          title={asList ? 'Показать плитками' : 'Показать списком'}
          onClick={() => void setLibraryView(asList ? 'grid' : 'list')}
        >
          {asList ? '▦' : '☰'}
        </button>
        <input
          className="input"
          style={{ width: 240 }}
          placeholder="Поиск по библиотеке…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {items.length === 0 ? (
        <Empty icon="🔍" title="Ничего не найдено" />
      ) : (
        <div className={asList ? 'library-list' : 'library-grid'}>
          {items.map((game) =>
            asList ? (
              <GameLine
                key={game.infoHash}
                game={game}
                onClick={() => onSelect(game.infoHash)}
              />
            ) : (
              <GameCard
                key={game.infoHash}
                game={game}
                downloading={progress[game.infoHash.toUpperCase()]}
                onClick={() => onSelect(game.infoHash)}
              />
            ),
          )}
        </div>
      )}
    </div>
  )
}

/** One library entry as a line, for people with more titles than wall space. */
function GameLine({ game, onClick }: { game: LibraryItem; onClick: () => void }) {
  const src = coverSrc(game.coverPath)
  return (
    <button className="library-line" onClick={onClick} title={game.title}>
      <span className="library-line-art">
        {src ? <img src={src} alt="" loading="lazy" /> : <span>🎮</span>}
      </span>
      <span className="library-line-title">{game.title}</span>
      {game.favorite && <span className="library-line-star">★</span>}
      <span className="library-line-meta">{game.category}</span>
    </button>
  )
}

function GameCard({
  game,
  downloading,
  onClick,
}: {
  game: LibraryItem
  downloading?: { progressBytes: number; totalBytes: number; finished: boolean }
  onClick: () => void
}) {
  const src = coverSrc(game.coverPath)
  const pct =
    downloading && !downloading.finished
      ? progressPercent(downloading.progressBytes, downloading.totalBytes)
      : null

  return (
    <button className="game-card" onClick={onClick}>
      <div className="game-cover">
        {src ? (
          <img src={src} alt={game.title} loading="lazy" />
        ) : (
          <div className="placeholder">{game.title}</div>
        )}
        {game.favorite && <span className="cover-fav">★</span>}
        {game.hasPendingUpdate && <span className="cover-badge">Обновление</span>}
      </div>
      <div>
        <div className="game-name" title={game.title}>
          {game.title}
        </div>
        <div className="game-meta">
          {pct != null
            ? `Загрузка ${pct.toFixed(0)}%`
            : game.lastPlayedAt
              ? formatPlaytime(game.playSeconds)
              : 'Готово к запуску'}
        </div>
      </div>
    </button>
  )
}

function GameDetail({ game, onBack }: { game: LibraryItem; onBack: () => void }) {
  const { progress, pendingUpdates, refreshLibrary, toast, reportError } = useStore()
  const [exePicker, setExePicker] = useState(false)
  const [renaming, setRenaming] = useState(false)
  const [newTitle, setNewTitle] = useState(game.title)
  const [busyCover, setBusyCover] = useState(false)
  const [updateModal, setUpdateModal] = useState<TopicUpdate | null>(null)
  const [confirmingRemove, setConfirmingRemove] = useState(false)

  const live = progress[game.infoHash.toUpperCase()]
  const pending = pendingUpdates.find((u) => u.topicId === game.topicId) ?? null
  const cover = coverSrc(game.coverPath)

  async function launch() {
    try {
      await libraryApi.launch(game.infoHash)
      await refreshLibrary()
    } catch (e) {
      const err = reportError(e, 'Запуск')
      if (err.message.includes('не выбран')) setExePicker(true)
    }
  }

  async function fetchCover() {
    setBusyCover(true)
    try {
      await libraryApi.fetchCover(game.infoHash)
      await refreshLibrary()
      toast('Обложка загружена')
    } catch (e) {
      reportError(e, 'Обложка')
    } finally {
      setBusyCover(false)
    }
  }

  async function toggleFavorite() {
    try {
      await libraryApi.setFlag(game.infoHash, 'favorite', !game.favorite)
      await refreshLibrary()
    } catch (e) {
      reportError(e)
    }
  }

  async function rename() {
    try {
      await libraryApi.setTitle(game.infoHash, newTitle)
      await refreshLibrary()
      setRenaming(false)
    } catch (e) {
      reportError(e, 'Переименование')
    }
  }

  return (
    <div className="page">
      <button className="btn ghost sm" onClick={onBack} style={{ marginBottom: 14 }}>
        ← Назад в библиотеку
      </button>

      <div className="hero">
        <div
          className={cover ? 'hero-art' : 'hero-art empty-art'}
          style={cover ? { backgroundImage: `url("${cover}")` } : undefined}
        />
        <div className="hero-overlay">
          <h1 className="hero-title">{game.title}</h1>
          <div className="hero-sub">
            <span>{formatPlaytime(game.playSeconds)}</span>
            {game.lastPlayedAt && <span>Последний запуск: {formatDate(game.lastPlayedAt)}</span>}
            {game.topicId && <span>Раздача #{game.topicId}</span>}
          </div>
        </div>
      </div>

      {pending && (
        <div className="banner warn">
          <span style={{ fontSize: 18 }}>🔄</span>
          <span style={{ flex: 1 }}>
            Раздача обновилась на трекере. Можно докачать разницу, не теряя уже скачанное.
          </span>
          <button className="btn primary sm" onClick={() => setUpdateModal(pending)}>
            Посмотреть
          </button>
        </div>
      )}

      {live && !live.finished && (
        <div className="card" style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', gap: 12, marginBottom: 8 }}>
            <strong>Загружается</strong>
            <div className="spacer" />
            <span>
              {formatBytes(live.progressBytes)} / {formatBytes(live.totalBytes)}
            </span>
          </div>
          <ProgressBar done={live.progressBytes} total={live.totalBytes} />
        </div>
      )}

      <div className="hero-actions" style={{ marginBottom: 22 }}>
        <button
          className="btn primary big"
          onClick={launch}
          disabled={Boolean(live && !live.finished)}
        >
          ▶ {game.exePath ? 'Играть' : 'Выбрать и играть'}
        </button>
        <WatchButton infoHash={game.infoHash} />
        <button className="btn" onClick={() => setExePicker(true)}>
          Выбрать файл запуска
        </button>
        <button className="btn" onClick={() => void libraryApi.openFolder(game.infoHash)}>
          📁 Папка
        </button>
        <button className="btn" onClick={toggleFavorite}>
          {game.favorite ? '★ В избранном' : '☆ В избранное'}
        </button>
      </div>

      <div className="detail-grid">
        <div className="card">
          <h3 className="card-title">Сведения</h3>
          <dl className="kv">
            <dt>Название</dt>
            <dd>
              {renaming ? (
                <div style={{ display: 'flex', gap: 8 }}>
                  <input
                    className="input"
                    value={newTitle}
                    autoFocus
                    onChange={(e) => setNewTitle(e.target.value)}
                    onKeyDown={(e) => e.key === 'Enter' && void rename()}
                  />
                  <button className="btn sm primary" onClick={rename}>
                    ОК
                  </button>
                  <button className="btn sm ghost" onClick={() => setRenaming(false)}>
                    Отмена
                  </button>
                </div>
              ) : (
                <span>
                  {game.title}{' '}
                  <button
                    className="btn ghost sm"
                    onClick={() => {
                      setNewTitle(game.title)
                      setRenaming(true)
                    }}
                  >
                    ✎
                  </button>
                </span>
              )}
            </dd>

            <dt>Папка установки</dt>
            <dd className="mono">{game.installDir ?? '—'}</dd>

            <dt>Файл запуска</dt>
            <dd className="mono">{game.exePath ?? 'не выбран'}</dd>

            <dt>Info hash</dt>
            <dd className="mono">{game.infoHash}</dd>
          </dl>
        </div>

        <div className="card">
          <h3 className="card-title">Обложка</h3>
          <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
            Загружается из первой картинки темы на трекере.
          </p>
          <button
            className="btn"
            onClick={fetchCover}
            disabled={busyCover || !game.topicId}
            style={{ width: '100%' }}
          >
            {busyCover ? <Spinner /> : null}
            {game.coverPath ? 'Обновить обложку' : 'Загрузить обложку'}
          </button>
          {!game.topicId && (
            <p className="hint" style={{ marginTop: 10 }}>
              Раздача не связана с темой трекера, автоматическая обложка недоступна.
            </p>
          )}

          <h3 className="card-title" style={{ marginTop: 22 }}>
            Опасная зона
          </h3>
          <button
            className="btn danger"
            style={{ width: '100%' }}
            onClick={() => setConfirmingRemove(true)}
          >
            Удалить из библиотеки
          </button>
        </div>
      </div>

      {exePicker && (
        <ExePicker
          game={game}
          onClose={() => setExePicker(false)}
          onPicked={async () => {
            setExePicker(false)
            await refreshLibrary()
          }}
        />
      )}

      {confirmingRemove && (
        <ConfirmDialog
          title="Удалить из библиотеки"
          icon="🗑"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                <strong>{game.title}</strong>
              </p>
              <p style={{ color: 'var(--text-dim)', marginBottom: 0 }}>
                Карточка исчезнет из библиотеки. Файлы можно оставить на диске.
              </p>
            </>
          }
          choices={[
            { label: 'Только из библиотеки', value: false, kind: 'primary' },
            { label: 'Вместе с файлами', value: true, kind: 'danger' },
          ]}
          onCancel={() => setConfirmingRemove(false)}
          onPick={async (withFiles) => {
            setConfirmingRemove(false)
            try {
              await torrentsApi.remove(game.infoHash, withFiles)
              await refreshLibrary()
              toast(withFiles ? 'Удалено вместе с файлами' : 'Удалено из библиотеки')
              onBack()
            } catch (e) {
              reportError(e, 'Удаление')
            }
          }}
        />
      )}

      {updateModal && (
        <UpdateModal update={updateModal} onClose={() => setUpdateModal(null)} />
      )}
    </div>
  )
}

function ExePicker({
  game,
  onClose,
  onPicked,
}: {
  game: LibraryItem
  onClose: () => void
  onPicked: () => void
}) {
  const { reportError } = useStore()
  const [items, setItems] = useState<ExecutableCandidate[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    libraryApi
      .scanExecutables(game.infoHash)
      .then((list) => {
        if (!cancelled) setItems(list)
      })
      .catch((e) => {
        if (!cancelled) setError(reportError(e, 'Поиск .exe').message)
      })
    return () => {
      cancelled = true
    }
  }, [game.infoHash, reportError])

  async function choose(path: string) {
    try {
      await libraryApi.setExe(game.infoHash, path)
      onPicked()
    } catch (e) {
      reportError(e)
    }
  }

  return (
    <Modal title="Файл для запуска" icon="▶" wide onClose={onClose}>
      {error && <div className="banner error">{error}</div>}
      {!items && !error && <Spinner />}
      {items && items.length === 0 && (
        <Empty icon="📂" title="Исполняемых файлов не найдено" hint="Возможно, игру ещё нужно установить." />
      )}
      {items && items.length > 0 && (
        <div className="exe-list">
          {items.map((c) => (
            <button
              key={c.path}
              className={c.path === game.exePath ? 'exe-item chosen' : 'exe-item'}
              onClick={() => choose(c.path)}
            >
              <span style={{ fontSize: 16 }}>{c.isInstaller ? '📦' : '▶'}</span>
              <span style={{ flex: 1, minWidth: 0 }}>
                <span className="exe-name">{c.fileName}</span>
                <span className="exe-path">{c.path}</span>
              </span>
              <span style={{ color: 'var(--muted)', fontSize: 12 }}>
                {formatBytes(c.sizeBytes)}
              </span>
              {c.isInstaller && <span className="tag warn">установщик</span>}
            </button>
          ))}
        </div>
      )}
    </Modal>
  )
}
