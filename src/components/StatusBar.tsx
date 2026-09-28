// The strip along the bottom: what the whole session is doing.
//
// Every torrent client has one, and for good reason — it answers "is anything
// happening at all" without opening a single row.

import { useEffect, useState, useSyncExternalStore } from 'react'

import * as activity from '../lib/activity'
import { torrents as torrentsApi } from '../lib/api'
import { formatSpeed } from '../lib/format'
import type { SessionSummary } from '../lib/types'

/** Seconds as `3 ч 14 мин`, or `14 мин` under an hour. */
function formatUptime(seconds: number): string {
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  return hours > 0 ? `${hours} ч ${minutes} мин` : `${minutes} мин`
}

/** How long a labelled call has to run before it is worth mentioning. */
const SHOW_AFTER_MS = 400

export function StatusBar() {
  const [stats, setStats] = useState<SessionSummary | null>(null)
  // Consecutive failures to reach the engine. One is a blip; three in a row
  // is the answer to "is it working at all".
  const [misses, setMisses] = useState(0)

  // What the backend is busy with, and for how long.
  useSyncExternalStore(activity.subscribe, activity.getSnapshot)
  const busy = activity.current()
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!busy) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [busy])
  const busyFor = busy ? now - busy.since : 0

  useEffect(() => {
    let stopped = false
    const tick = async () => {
      try {
        const next = await torrentsApi.sessionStats()
        if (!stopped) {
          setStats(next)
          setMisses(0)
        }
      } catch {
        // The engine may not be up yet; the next tick will find it.
        if (!stopped) setMisses((n) => n + 1)
      }
    }
    void tick()
    const timer = window.setInterval(tick, 2000)
    return () => {
      stopped = true
      window.clearInterval(timer)
    }
  }, [])

  if (!stats && misses < 3) return null

  return (
    <div className="status-bar">
      {stats ? (
        <>
          <span title="Скорость приёма">↓ {formatSpeed(stats.downloadSpeedBps)}</span>
          <span title="Скорость отдачи">↑ {formatSpeed(stats.uploadSpeedBps)}</span>
        </>
      ) : (
        <span className="status-warn" title="Движок не отвечает на запросы">
          ⚠ Движок не отвечает
        </span>
      )}

      {busy && busyFor >= SHOW_AFTER_MS && (
        <span className="status-activity" title="Что приложение делает прямо сейчас">
          <span className="status-spin">⟳</span> {busy.label}
          {busyFor >= 2000 ? ` · ${Math.floor(busyFor / 1000)} с` : ''}
        </span>
      )}

      <span className="spacer" />
      {stats && (
        <>
          <span title="Узлов в таблице DHT — чем больше, тем легче находить раздающих">
            DHT: {stats.dhtNodes}
          </span>
          <span title="Сколько работает движок">{formatUptime(stats.uptimeSeconds)}</span>
        </>
      )}
    </div>
  )
}
