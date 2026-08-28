// A tracker result rendered as a card with preview artwork.
//
// Shared by the search grid and the home screen's "what is new" strips, so both
// look the same and both benefit from the same lazy, rate-limited preview
// loading.

import { useState } from 'react'

import { asAppError, player as playerApi, tracker, wishlist } from '../lib/api'
import { formatBytes } from '../lib/format'
import { usePreview } from '../lib/previews'
import { useStore } from '../lib/store'
import type { SearchItem } from '../lib/types'
import { Spinner } from './ui'

export function ResultCard({
  item,
  downloading,
  onPreview,
  onDownload,
}: {
  item: SearchItem
  downloading: boolean
  onPreview: () => void
  onDownload: () => void
}) {
  const { toast, reportError } = useStore()
  const { url, loading, ref } = usePreview(item.topicId)
  const [saved, setSaved] = useState(false)
  const [watching, setWatching] = useState(false)

  async function watchNow() {
    setWatching(true)
    try {
      await playerApi.watchTopic(item.topicId, item.title)
      toast('Открываю плеер — файл подкачивается на ходу')
    } catch (e) {
      reportError(e, 'Просмотр')
    } finally {
      setWatching(false)
    }
  }

  async function saveForLater() {
    try {
      await wishlist.add({
        topicId: item.topicId,
        title: item.title,
        imageUrl: url,
        sizeBytes: item.sizeBytes,
        category: 'movie',
      })
      setSaved(true)
      toast('Добавлено в «Посмотреть позже»')
    } catch (e) {
      reportError(e, 'Не удалось добавить')
    }
  }
  // Post images live on third-party hosts; some refuse to serve them here.
  const [broken, setBroken] = useState(false)

  return (
    <div className="result-card" ref={ref}>
      <div className="result-art" onClick={onPreview} title="Открыть описание">
        {url && !broken ? (
          <img src={url} alt="" loading="lazy" onError={() => setBroken(true)} />
        ) : (
          <div className="result-art-empty">{loading ? <Spinner /> : '🖼'}</div>
        )}
        {item.approved && <span className="art-badge">✔</span>}
        <button
          className="art-play"
          title="Посмотреть, не скачивая"
          disabled={watching}
          onClick={(e) => {
            e.stopPropagation()
            void watchNow()
          }}
        >
          {watching ? <Spinner /> : '▶'}
        </button>
        <button
          className="art-plus"
          title={saved ? 'Уже в списке «Посмотреть позже»' : 'Отложить на потом'}
          disabled={saved}
          onClick={(e) => {
            // The artwork itself opens the description; this must not.
            e.stopPropagation()
            void saveForLater()
          }}
        >
          {saved ? '✓' : '+'}
        </button>
      </div>

      <div className="result-card-body">
        <div className="result-card-title" onClick={onPreview} title={item.title}>
          {item.title}
        </div>
        <div className="result-card-meta">
          <span>{formatBytes(item.sizeBytes)}</span>
          <span className="seed">▲ {item.seeders}</span>
          <span className="leech">▼ {item.leechers}</span>
        </div>
        <div className="result-card-meta muted-line">{item.forumName ?? '—'}</div>
        <button className="btn primary sm card-dl" onClick={onDownload} disabled={downloading}>
          {downloading ? <Spinner /> : '⬇'} Скачать
        </button>
      </div>
    </div>
  )
}

/**
 * The "get this release" action, shared by every view that lists results.
 *
 * Returns which topic is currently downloading so a card can disable its own
 * button, and reports an expired session to the caller so it can offer a login.
 */
export function useTrackerDownload(opts: {
  onAdded: (infoHash: string) => void
  onNeedLogin: () => void
}) {
  const { refreshAll, toast, reportError } = useStore()
  const [downloading, setDownloading] = useState<number | null>(null)

  async function download(item: SearchItem) {
    setDownloading(item.topicId)
    try {
      const added = await tracker.download({ topicId: item.topicId })
      await refreshAll()
      toast(
        added.alreadyPresent
          ? 'Эта раздача уже была добавлена'
          : `Добавлено в загрузки: ${added.name ?? item.title}`,
      )
      opts.onAdded(added.infoHash)
    } catch (e) {
      const err = asAppError(e)
      if (err.kind === 'not_authenticated') opts.onNeedLogin()
      else reportError(e, 'Скачивание')
    } finally {
      setDownloading(null)
    }
  }

  return { downloading, download }
}
