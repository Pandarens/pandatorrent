// "Watch later": releases marked with the + button but not downloaded.
//
// They sit in the library next to what is installed, because from the user's
// point of view both answer the same question — what am I going to watch.

import { useEffect, useState } from 'react'

import { tracker, wishlist as wishlistApi } from '../lib/api'
import { formatBytes } from '../lib/format'
import { useStore } from '../lib/store'
import type { WishlistItem } from '../lib/types'
import { Spinner } from './ui'

export function Wishlist({ onOpenGame }: { onOpenGame: (infoHash: string) => void }) {
  const { refreshAll, toast, reportError } = useStore()
  const [items, setItems] = useState<WishlistItem[] | null>(null)
  const [busy, setBusy] = useState<number | null>(null)

  async function load() {
    try {
      setItems(await wishlistApi.list())
    } catch (e) {
      reportError(e, 'Посмотреть позже')
      setItems([])
    }
  }

  useEffect(() => {
    void load()
  }, [])

  async function download(item: WishlistItem) {
    setBusy(item.topicId)
    try {
      const added = await tracker.download({ topicId: item.topicId, title: item.title })
      // It has moved from "planned" to "have it", so it leaves this list.
      await wishlistApi.remove(item.topicId)
      await Promise.all([load(), refreshAll()])
      toast(`Скачивается: ${added.name ?? item.title}`)
      onOpenGame(added.infoHash)
    } catch (e) {
      reportError(e, 'Скачивание')
    } finally {
      setBusy(null)
    }
  }

  async function remove(topicId: number) {
    try {
      await wishlistApi.remove(topicId)
      await load()
    } catch (e) {
      reportError(e, 'Удаление')
    }
  }

  if (!items || items.length === 0) return null

  return (
    <div className="strip-block">
      <div className="strip-head">
        <h2 className="strip-title">Посмотреть позже</h2>
        <span className="page-sub">{items.length}</span>
      </div>

      <div className="strip">
        {items.map((item) => (
          <div className="result-card" key={item.topicId}>
            <div className="result-art">
              {item.imageUrl ? (
                <img src={item.imageUrl} alt="" loading="lazy" />
              ) : (
                <div className="result-art-empty">🎬</div>
              )}
            </div>
            <div className="result-card-body">
              <div className="result-card-title" title={item.title}>
                {item.title}
              </div>
              <div className="result-card-meta">
                <span>{formatBytes(item.sizeBytes)}</span>
              </div>
              <button
                className="btn primary sm card-dl"
                onClick={() => download(item)}
                disabled={busy === item.topicId}
              >
                {busy === item.topicId ? <Spinner /> : '⬇'} Скачать
              </button>
              <button
                className="btn ghost sm"
                style={{ width: '100%', justifyContent: 'center', marginTop: 6 }}
                onClick={() => remove(item.topicId)}
              >
                Убрать
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
