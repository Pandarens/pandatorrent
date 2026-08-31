// Films and episodes left part-way through.
//
// The viewing position has been recorded for a while, but only the history row
// showed it — so picking a half-watched film back up meant hunting for it. This
// is the shelf that makes "continue" the obvious thing to do.

import { useEffect, useState } from 'react'

import { history as historyApi, onHistoryUpdated, player as playerApi } from '../lib/api'
import { useStore } from '../lib/store'
import type { WatchHistoryItem } from '../lib/types'
import { ScrollStrip } from './ScrollStrip'
import { Spinner } from './ui'

/** Below this it is the beginning, and there is nothing to continue. */
const MIN_SECONDS = 30
/** This close to the end counts as watched. */
const TAIL_SECONDS = 60

/** Seconds as `1:23:45`, or `4:07` for anything under an hour. */
function formatClock(seconds: number): string {
  const total = Math.floor(seconds)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const pad = (n: number) => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`
}

/** Whether this row is genuinely unfinished. */
function unfinished(item: WatchHistoryItem): boolean {
  const at = item.positionSeconds
  if (at == null || at < MIN_SECONDS) return false
  if (item.durationSeconds != null && at > item.durationSeconds - TAIL_SECONDS) return false
  return item.topicId != null
}

export function ContinueWatching() {
  const { toast, reportError } = useStore()
  const [items, setItems] = useState<WatchHistoryItem[] | null>(null)
  const [busy, setBusy] = useState<number | null>(null)

  async function load() {
    try {
      setItems((await historyApi.list()).filter(unfinished))
    } catch {
      // The shelf is a convenience; a failure here should not shout at anyone.
      setItems([])
    }
  }

  useEffect(() => {
    void load()
    const un = onHistoryUpdated(() => void load())
    return () => {
      void un.then((f) => f())
    }
  }, [])

  async function resume(item: WatchHistoryItem) {
    if (!item.topicId) return
    setBusy(item.id)
    try {
      await playerApi.watchTopic(item.topicId, item.title)
      toast('Продолжаю с того места')
    } catch (e) {
      reportError(e, 'Продолжить просмотр')
    } finally {
      setBusy(null)
    }
  }

  // Nothing half-watched is not worth a heading of its own.
  if (!items || items.length === 0) return null

  return (
    <div className="strip-block">
      <div className="strip-head">
        <h3 className="card-title">Продолжить смотреть</h3>
        <div className="spacer" />
      </div>

      <ScrollStrip>
        {items.map((item) => {
          const at = item.positionSeconds ?? 0
          const total = item.durationSeconds ?? 0
          const percent = total > 0 ? Math.min(100, (at / total) * 100) : 0
          return (
            <div className="result-card" key={item.id}>
              <button
                className="result-art as-button"
                title={`Продолжить с ${formatClock(at)}`}
                onClick={() => void resume(item)}
                disabled={busy === item.id}
              >
                {item.imageUrl ? (
                  <img src={item.imageUrl} alt="" loading="lazy" />
                ) : (
                  <div className="result-art-empty">🎬</div>
                )}
                <span className="art-play">{busy === item.id ? <Spinner /> : '▶'}</span>
                {percent > 0 && (
                  <span className="art-progress">
                    <span style={{ width: `${percent}%` }} />
                  </span>
                )}
              </button>

              <div className="result-card-body">
                <div className="result-card-title" title={item.title}>
                  {item.title}
                </div>
                {item.fileName && (
                  <div className="result-card-meta muted-line" title={item.fileName}>
                    {item.fileName}
                  </div>
                )}
                <div className="result-card-meta">
                  <span>
                    {formatClock(at)}
                    {total > 0 ? ` из ${formatClock(total)}` : ''}
                  </span>
                </div>
              </div>
            </div>
          )
        })}
      </ScrollStrip>
    </div>
  )
}
