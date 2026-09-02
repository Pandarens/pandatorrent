// Active and finished torrents, with live counters from the progress event.

import { useMemo, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import { library as libraryApi, torrents as torrentsApi, tracker } from '../lib/api'
import { formatBytes, formatEta, formatSpeed, stateLabel } from '../lib/format'
import { useStore } from '../lib/store'
import type { TorrentProgress, TorrentView } from '../lib/types'
import { Empty, Modal, ProgressBar } from '../components/ui'
import { ConfirmDialog } from '../components/ConfirmDialog'
import { TorrentFiles } from '../components/TorrentFiles'
import { CreateTorrentModal } from '../components/CreateTorrentModal'

type Filter = 'all' | 'active' | 'done'
type SortKey = 'added' | 'name' | 'size' | 'progress' | 'speed'

const SORT_LABELS: Record<SortKey, string> = {
  added: 'По добавлению',
  name: 'По названию',
  size: 'По размеру',
  progress: 'По прогрессу',
  speed: 'По скорости',
}

/**
 * Asks the tracker API whether this info hash belongs to a known topic, so a
 * torrent added by hand still gets update watching. The API needs no login,
 * and a failure here is not worth bothering the user about.
 */
async function tryTrack(infoHash: string): Promise<number | null> {
  try {
    return await tracker.trackExisting(infoHash)
  } catch {
    return null
  }
}

export function DownloadsView({
  shutdownOnce = false,
  onShutdownOnce,
}: {
  /** Turn the computer off when this batch finishes — just this once. */
  shutdownOnce?: boolean
  onShutdownOnce?: (on: boolean) => void
}) {
  const { torrents, progress, refreshAll, toast, reportError } = useStore()
  const [filter, setFilter] = useState<Filter>('all')
  const [addOpen, setAddOpen] = useState(false)
  const [createOpen, setCreateOpen] = useState(false)
  const [search, setSearch] = useState('')
  const [sort, setSort] = useState<SortKey>('added')
  const [descending, setDescending] = useState(true)

  const rows = useMemo(() => {
    return torrents
      .map((t) => ({ t, p: progress[t.infoHash.toUpperCase()] ?? t.progress ?? null }))
      .filter(({ p }) => {
        if (filter === 'all') return true
        const finished = p?.finished ?? false
        return filter === 'done' ? finished : !finished
      })
      .filter(({ t }) => {
        const needle = search.trim().toLowerCase()
        return needle === '' || t.name.toLowerCase().includes(needle)
      })
      .sort((a, b) => {
        const size = (r: typeof a) => r.p?.totalBytes || r.t.totalBytes
        const share = (r: typeof a) => {
          const total = size(r)
          return total > 0 ? (r.p?.progressBytes ?? 0) / total : 0
        }
        let by = 0
        switch (sort) {
          case 'name':
            by = a.t.name.localeCompare(b.t.name, 'ru')
            break
          case 'size':
            by = size(a) - size(b)
            break
          case 'progress':
            by = share(a) - share(b)
            break
          case 'speed':
            by = (a.p?.downloadSpeedBps ?? 0) - (b.p?.downloadSpeedBps ?? 0)
            break
          default:
            by = a.t.addedAt - b.t.addedAt
        }
        return descending ? -by : by
      })
  }, [torrents, progress, filter, search, sort, descending])

  async function addFromFile() {
    const picked = await open({
      multiple: false,
      filters: [{ name: 'Torrent', extensions: ['torrent'] }],
    })
    if (typeof picked !== 'string') return
    try {
      const added = await torrentsApi.addFile(picked)
      await libraryApi.add(added.infoHash)
      const topicId = await tryTrack(added.infoHash)
      await refreshAll()
      toast(
        topicId
          ? `Добавлено: ${added.name ?? added.infoHash}. Раздача #${topicId} взята под наблюдение.`
          : `Добавлено: ${added.name ?? added.infoHash}`,
      )
    } catch (e) {
      reportError(e, 'Добавление файла')
    }
  }

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">Загрузки</h1>
        <span className="page-sub">{rows.length}</span>
        <div className="spacer" />
        {(['all', 'active', 'done'] as Filter[]).map((f) => (
          <button
            key={f}
            className={filter === f ? 'btn primary sm' : 'btn sm'}
            onClick={() => setFilter(f)}
          >
            {f === 'all' ? 'Все' : f === 'active' ? 'Активные' : 'Завершённые'}
          </button>
        ))}
        <button className="btn sm" onClick={addFromFile}>
          📄 Из файла
        </button>
        <button className="btn sm" title="Собрать .torrent из своих файлов" onClick={() => setCreateOpen(true)}>
          ✚ Создать
        </button>
        <button className="btn primary sm" onClick={() => setAddOpen(true)}>
          🧲 Magnet / ссылка
        </button>
      </div>

      <div className="downloads-bar">
        <input
          className="input"
          type="search"
          placeholder="Найти среди загрузок"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select
          className="select"
          value={sort}
          title="Порядок в списке"
          onChange={(e) => setSort(e.target.value as SortKey)}
        >
          {(Object.keys(SORT_LABELS) as SortKey[]).map((k) => (
            <option key={k} value={k}>
              {SORT_LABELS[k]}
            </option>
          ))}
        </select>
        <button
          className="btn sm"
          title={descending ? 'Сначала большие' : 'Сначала меньшие'}
          onClick={() => setDescending((v) => !v)}
        >
          {descending ? '↓' : '↑'}
        </button>

        <div className="spacer" />
        {onShutdownOnce && (
          <label
            className="checkbox inline"
            title="Только для этих загрузок — настройка не сохраняется"
          >
            <input
              type="checkbox"
              checked={shutdownOnce}
              onChange={(e) => onShutdownOnce(e.target.checked)}
            />
            <span>Выключить компьютер по завершении</span>
          </label>
        )}
      </div>

      {rows.length === 0 ? (
        <Empty
          icon="⬇"
          title="Загрузок нет"
          hint="Добавьте магнет-ссылку, .torrent-файл или найдите раздачу через поиск."
        />
      ) : (
        <>
          {rows
            .filter(({ p, t }) => !(p?.finished ?? t.completedAt != null))
            .map(({ t, p }) => (
              <TorrentRow key={t.infoHash} torrent={t} progress={p} />
            ))}

          {/* Finished releases keep seeding, but they are not what anyone is
              watching — so they sit below instead of among the active ones. */}
          {(() => {
            const done = rows.filter(({ p, t }) => p?.finished ?? t.completedAt != null)
            if (done.length === 0) return null
            return (
              <>
                <div className="section-split">
                  <span>Готово</span>
                  <span className="section-count">{done.length}</span>
                  <span className="section-line" />
                </div>
                {done.map(({ t, p }) => (
                  <TorrentRow key={t.infoHash} torrent={t} progress={p} />
                ))}
              </>
            )
          })()}
        </>
      )}

      {addOpen && <AddUrlModal onClose={() => setAddOpen(false)} />}
      {createOpen && <CreateTorrentModal onClose={() => setCreateOpen(false)} />}
    </div>
  )
}

function TorrentRow({
  torrent,
  progress,
}: {
  torrent: TorrentView
  progress: TorrentProgress | null
}) {
  const { refreshAll, toast, reportError } = useStore()
  const [busy, setBusy] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const [open, setOpen] = useState(false)

  const state = progress?.state ?? 'paused'
  const finished = progress?.finished ?? torrent.completedAt != null
  const hasError = Boolean(progress?.error)
  const done = progress?.progressBytes ?? 0
  const total = progress?.totalBytes || torrent.totalBytes

  const variant = hasError ? 'error' : finished ? 'done' : state === 'paused' ? 'paused' : undefined

  async function act(fn: () => Promise<unknown>, message?: string) {
    setBusy(true)
    try {
      await fn()
      await refreshAll()
      if (message) toast(message)
    } catch (e) {
      reportError(e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="torrent-row">
      <div className="torrent-head">
        <button
          className="torrent-name as-open"
          title={open ? 'Свернуть' : 'Показать файлы раздачи'}
          onClick={() => setOpen(!open)}
        >
          <span className={open ? 'chevron open' : 'chevron'}>›</span>
          <span className="torrent-name-text">{torrent.name}</span>
        </button>
        <span className={finished ? 'tag accent' : 'tag'}>
          {/* A queue-held download is not the same as one somebody stopped,
              and calling both "пауза" hides why nothing is happening. */}
          {state === 'paused' && !finished && !torrent.userPaused
            ? 'В очереди'
            : stateLabel(state, finished, hasError)}
        </span>
        <div className="torrent-actions">
          {state === 'paused' ? (
            <button
              className="btn sm"
              disabled={busy}
              onClick={() => act(() => torrentsApi.resume(torrent.infoHash))}
            >
              ▶
            </button>
          ) : (
            <button
              className="btn sm"
              disabled={busy}
              onClick={() => act(() => torrentsApi.pause(torrent.infoHash))}
            >
              ⏸
            </button>
          )}
          <button
            className="btn sm"
            disabled={busy || state === 'initializing'}
            title="Проверить файлы на диске"
            onClick={() =>
              act(() => torrentsApi.recheck(torrent.infoHash), 'Проверяю файлы')
            }
          >
            ✓
          </button>
          <button
            className="btn sm"
            disabled={busy}
            onClick={() => act(() => torrentsApi.openFolder(torrent.infoHash))}
          >
            📁
          </button>
          <button
            className="btn sm danger"
            disabled={busy}
            title="Удалить"
            onClick={() => setConfirming(true)}
          >
            🗑
          </button>
        </div>
      </div>

      {open && <TorrentFiles infoHash={torrent.infoHash} noSeeding={torrent.noSeeding} />}

      {confirming && (
        <ConfirmDialog
          title="Удалить раздачу"
          icon="🗑"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                <strong>{torrent.name}</strong>
              </p>
              <p style={{ color: 'var(--text-dim)', marginBottom: 0 }}>
                Можно убрать раздачу только из списка — скачанные файлы останутся на диске.
              </p>
            </>
          }
          choices={[
            { label: 'Только из списка', value: false, kind: 'primary' },
            { label: 'Вместе с файлами', value: true, kind: 'danger' },
          ]}
          onCancel={() => setConfirming(false)}
          onPick={(withFiles) => {
            setConfirming(false)
            void act(
              () => torrentsApi.remove(torrent.infoHash, withFiles),
              withFiles ? 'Удалено вместе с файлами' : 'Удалено из списка',
            )
          }}
        />
      )}

      <ProgressBar done={done} total={total} variant={variant} />

      <div className="torrent-stats">
        <span>
          {formatBytes(done)} / {formatBytes(total)}
          {total > 0 ? ` (${((done / total) * 100).toFixed(1)}%)` : ''}
        </span>
        {progress && !finished && <span>↓ {formatSpeed(progress.downloadSpeedBps)}</span>}
        {progress && <span>↑ {formatSpeed(progress.uploadSpeedBps)}</span>}
        {progress && total > 0 && (
          <span title="Отдано по отношению к размеру раздачи">
            Рейтинг: {(progress.uploadedBytes / total).toFixed(2)}
          </span>
        )}
        {progress && !finished && <span>Осталось: {formatEta(progress.etaSeconds)}</span>}
        {progress && (
          <span>
            Пиры: {progress.peersLive} / {progress.peersSeen}
          </span>
        )}
        {torrent.topicId && <span>Раздача #{torrent.topicId}</span>}
        {hasError && <span style={{ color: 'var(--danger)' }}>{progress?.error}</span>}
      </div>
    </div>
  )
}

function AddUrlModal({ onClose }: { onClose: () => void }) {
  const { refreshAll, toast, reportError } = useStore()
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)

  async function add() {
    setBusy(true)
    try {
      const added = await torrentsApi.addUrl(url.trim())
      await libraryApi.add(added.infoHash)
      const topicId = await tryTrack(added.infoHash)
      await refreshAll()
      toast(
        topicId
          ? `Добавлено: ${added.name ?? added.infoHash}. Раздача #${topicId} взята под наблюдение.`
          : `Добавлено: ${added.name ?? added.infoHash}`,
      )
      onClose()
    } catch (e) {
      reportError(e, 'Добавление')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal
      title="Добавить торрент"
      icon="🧲"
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button className="btn primary" onClick={add} disabled={busy || !url.trim()}>
            Добавить
          </button>
        </>
      }
    >
      <div className="field">
        <label htmlFor="magnet">Magnet-ссылка, ссылка на .torrent или info hash</label>
        <input
          id="magnet"
          className="input"
          value={url}
          autoFocus
          placeholder="magnet:?xt=urn:btih:…"
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && url.trim() && void add()}
        />
      </div>
    </Modal>
  )
}
