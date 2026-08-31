// Finding things on RuTracker: by query, or by browsing the section catalogue.
//
// Both paths hit the same `tracker.php` endpoint and share one result parser,
// so browsing a forum is just a search with no words in it. Results render
// either as a dense list or as a grid with preview artwork.

import { useCallback, useState, type FormEvent } from 'react'

import { asAppError, settings as settingsApi, tracker } from '../lib/api'
import { formatBytes, formatDate } from '../lib/format'
import { useStore } from '../lib/store'
import type { BrowseTarget } from '../lib/browse'
import type {
  SearchItem,
  SearchPage,
  SearchSort,
} from '../lib/types'
import { CatalogPanel } from '../components/CatalogPanel'
import { NewReleases } from '../components/NewReleases'
import { ResultCard, useTrackerDownload } from '../components/ResultCard'
import { TopicPreviewModal } from '../components/TopicPreviewModal'
import { Empty, Spinner } from '../components/ui'
import { TrackerLogin } from '../components/TrackerLogin'

const COLUMNS: Array<{ id: SearchSort; label: string; numeric?: boolean }> = [
  { id: 'title', label: 'Название' },
  { id: 'size', label: 'Размер', numeric: true },
  { id: 'seeders', label: 'S', numeric: true },
  { id: 'leechers', label: 'L', numeric: true },
  { id: 'downloads', label: '↓', numeric: true },
  { id: 'registered', label: 'Добавлено', numeric: true },
]

type ViewMode = 'list' | 'grid'

export function SearchView({ onOpenLibrary }: { onOpenLibrary: (hash: string) => void }) {
  const { trackerStatus, config, refreshConfig, reportError } = useStore()

  const [text, setText] = useState('')
  const [forum, setForum] = useState<BrowseTarget | null>(null)
  const [sort, setSort] = useState<SearchSort>('seeders')
  const [ascending, setAscending] = useState(false)
  const [page, setPage] = useState(0)
  const [result, setResult] = useState<SearchPage | null>(null)
  const [busy, setBusy] = useState(false)
  const [needLogin, setNeedLogin] = useState(false)
  const [preview, setPreview] = useState<number | null>(null)
  const [catalogOpen, setCatalogOpen] = useState(false)

  const { downloading, download } = useTrackerDownload({
    onAdded: onOpenLibrary,
    onNeedLogin: () => setNeedLogin(true),
  })

  const view: ViewMode = config?.ui.searchView === 'grid' ? 'grid' : 'list'

  async function setView(next: ViewMode) {
    if (!config || next === view) return
    try {
      await settingsApi.set({ ...config, ui: { ...config.ui, searchView: next } })
      await refreshConfig()
    } catch (e) {
      reportError(e, 'Настройки вида')
    }
  }

  const run = useCallback(
    async (
      nextPage: number,
      opts?: {
        sort?: SearchSort
        ascending?: boolean
        text?: string
        forum?: BrowseTarget | null
      },
    ) => {
      const query = opts?.text ?? text
      const inForum = opts?.forum !== undefined ? opts.forum : forum
      // Browsing a section needs no words; a free search does.
      if (!query.trim() && !inForum) return

      setBusy(true)
      try {
        const res = await tracker.search({
          text: query.trim(),
          forumIds: inForum ? inForum.forumIds : [],
          sort: opts?.sort ?? sort,
          ascending: opts?.ascending ?? ascending,
          page: nextPage,
        })
        setResult(res)
        setPage(nextPage)
      } catch (e) {
        const err = asAppError(e)
        if (err.kind === 'not_authenticated') setNeedLogin(true)
        else reportError(e, 'Поиск')
      } finally {
        setBusy(false)
      }
    },
    [text, forum, sort, ascending, reportError],
  )

  function submit(e: FormEvent) {
    e.preventDefault()
    void run(0)
  }

  function toggleSort(id: SearchSort) {
    const asc = id === sort ? !ascending : false
    setSort(id)
    setAscending(asc)
    void run(0, { sort: id, ascending: asc })
  }

  function pickForum(picked: BrowseTarget) {
    setForum(picked)
    // A section is browsed newest-first; "most seeded" is a search notion.
    setSort('registered')
    setAscending(false)
    void run(0, { forum: picked, sort: 'registered', ascending: false })
  }

  function clearForum() {
    setForum(null)
    setResult(null)
    if (text.trim()) void run(0, { forum: null })
  }

  return (
    <div className="page search-page">
      <div className="page-head">
        <h1 className="page-title">Поиск на RuTracker</h1>
        <div className="spacer" />
        {trackerStatus?.hasSession ? (
          <span className="tag accent">
            {trackerStatus.username ?? 'вход выполнен'} · {trackerStatus.host}
          </span>
        ) : (
          <button className="btn primary sm" onClick={() => setNeedLogin(true)}>
            Войти на трекер
          </button>
        )}
      </div>

      <form className="search-bar" onSubmit={submit}>
        <button
          type="button"
          className={catalogOpen ? 'btn primary' : 'btn'}
          onClick={() => setCatalogOpen((v) => !v)}
          title="Разделы трекера"
        >
          ☰ Каталог
        </button>

        <input
          className="input"
          placeholder={forum ? `Поиск внутри «${forum.title}»` : 'Например: Cyberpunk 2077'}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <button
          className="btn primary"
          type="submit"
          disabled={busy || (!text.trim() && !forum)}
        >
          {busy ? <Spinner /> : '🔍'} Найти
        </button>

        <div className="view-toggle">
          <button
            type="button"
            className={view === 'list' ? 'active' : ''}
            onClick={() => void setView('list')}
            title="Списком"
          >
            ☰
          </button>
          <button
            type="button"
            className={view === 'grid' ? 'active' : ''}
            onClick={() => void setView('grid')}
            title="Сеткой с обложками"
          >
            ▦
          </button>
        </div>
      </form>

      {forum && (
        <div className="filter-chips">
          <span className="tag accent">
            Раздел: {forum.title}
            <button className="chip-x" onClick={clearForum} title="Убрать фильтр">
              ✕
            </button>
          </span>
        </div>
      )}

      {!trackerStatus?.hasSession && (
        <div className="banner info">
          <span>ℹ</span>
          <span>
            RuTracker закрыт проверкой Cloudflare, поэтому вход выполняется на самой
            странице трекера, в отдельном окне браузера. Приложение пароль не видит.
          </span>
        </div>
      )}

      <div className={catalogOpen ? 'search-body with-catalog' : 'search-body'}>
        {catalogOpen && <CatalogPanel activeForumId={forum?.id ?? null} onPick={pickForum} />}

        <div className="search-results">
          {!result && !busy && (
            <>
              <NewReleases onOpenGame={onOpenLibrary} onOpenSection={pickForum} />
              <Empty
                icon="🔍"
                title="Введите запрос или откройте каталог"
                hint="Каталог позволяет смотреть разделы трекера без поискового запроса."
              />
            </>
          )}

          {result && result.items.length === 0 && (
            <Empty
              icon="🤷"
              title="Ничего не найдено"
              hint="Попробуйте другой запрос или раздел."
            />
          )}

          {result && result.items.length > 0 && (
            <>
              <div className="page-sub" style={{ marginBottom: 10 }}>
                {result.total != null
                  ? `Найдено: ${result.total}`
                  : `Показано: ${result.items.length}`}
              </div>

              {view === 'list' ? (
                <ResultsList
                  items={result.items}
                  sort={sort}
                  ascending={ascending}
                  downloading={downloading}
                  onSort={toggleSort}
                  onPreview={setPreview}
                  onDownload={download}
                />
              ) : (
                <ResultsGrid
                  items={result.items}
                  downloading={downloading}
                  onPreview={setPreview}
                  onDownload={download}
                />
              )}

              <div className="pager">
                <button
                  className="btn sm"
                  disabled={page === 0 || busy}
                  onClick={() => run(page - 1)}
                >
                  ← Назад
                </button>
                <span className="page-sub">Страница {page + 1}</span>
                <button
                  className="btn sm"
                  disabled={busy || result.items.length < result.pageSize}
                  onClick={() => run(page + 1)}
                >
                  Вперёд →
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      {needLogin && <TrackerLogin onClose={() => setNeedLogin(false)} />}
      {preview != null && (
        <TopicPreviewModal
          topicId={preview}
          onClose={() => setPreview(null)}
          onDownload={() => {
            const item = result?.items.find((i) => i.topicId === preview)
            setPreview(null)
            if (item) void download(item)
          }}
        />
      )}
    </div>
  )
}

function ResultsList({
  items,
  sort,
  ascending,
  downloading,
  onSort,
  onPreview,
  onDownload,
}: {
  items: SearchItem[]
  sort: SearchSort
  ascending: boolean
  downloading: number | null
  onSort: (id: SearchSort) => void
  onPreview: (topicId: number) => void
  onDownload: (item: SearchItem) => void
}) {
  return (
    <table className="result-table">
      <thead>
        <tr>
          {COLUMNS.map((c) => (
            <th key={c.id} className={c.numeric ? 'num' : undefined} onClick={() => onSort(c.id)}>
              {c.label}
              {sort === c.id ? (ascending ? ' ▲' : ' ▼') : ''}
            </th>
          ))}
          <th />
        </tr>
      </thead>
      <tbody>
        {items.map((item) => (
          <tr key={item.topicId}>
            <td>
              <span
                className="result-title"
                onClick={() => onPreview(item.topicId)}
                title="Открыть описание"
              >
                {item.title}
              </span>
              <span className="result-forum">
                {item.forumName ?? '—'}
                {item.author ? ` · ${item.author}` : ''}
                {item.approved ? ' · ✔ проверено' : ''}
              </span>
            </td>
            <td className="num">{formatBytes(item.sizeBytes)}</td>
            <td className="num seed">{item.seeders}</td>
            <td className="num leech">{item.leechers}</td>
            <td className="num">{item.downloads}</td>
            <td className="num">{formatDate(item.registeredAt)}</td>
            <td className="num">
              <button
                className="btn primary sm"
                onClick={() => onDownload(item)}
                disabled={downloading === item.topicId}
              >
                {downloading === item.topicId ? <Spinner /> : '⬇'} Скачать
              </button>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function ResultsGrid({
  items,
  downloading,
  onPreview,
  onDownload,
}: {
  items: SearchItem[]
  downloading: number | null
  onPreview: (topicId: number) => void
  onDownload: (item: SearchItem) => void
}) {
  return (
    <div className="result-grid">
      {items.map((item) => (
        <ResultCard
          key={item.topicId}
          item={item}
          downloading={downloading === item.topicId}
          onPreview={() => onPreview(item.topicId)}
          onDownload={() => onDownload(item)}
        />
      ))}
    </div>
  )
}
