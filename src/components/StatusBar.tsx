// The strip along the bottom: what the whole session is doing.
//
// Every torrent client has one, and for good reason — it answers "is anything
// happening at all" without opening a single row.

import { useEffect, useState } from 'react'

import { torrents as torrentsApi } from '../lib/api'
import { formatSpeed } from '../lib/format'
import type { SessionSummary } from '../lib/types'

/** Seconds as `3 ч 14 мин`, or `14 мин` under an hour. */
function formatUptime(seconds: number): string {
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  return hours > 0 ? `${hours} ч ${minutes} мин` : `${minutes} мин`
}

export function StatusBar() {
  const [stats, setStats] = useState<SessionSummary | null>(null)

  useEffect(() => {
    let stopped = false
    const tick = async () => {
      try {
        const next = await torrentsApi.sessionStats()
        if (!stopped) setStats(next)
      } catch {
        // The engine may not be up yet; the next tick will find it.
      }
    }
    void tick()
    const timer = window.setInterval(tick, 2000)
    return () => {
      stopped = true
      window.clearInterval(timer)
    }
  }, [])

  if (!stats) return null

  return (
    <div className="status-bar">
      <span title="Скорость приёма">↓ {formatSpeed(stats.downloadSpeedBps)}</span>
      <span title="Скорость отдачи">↑ {formatSpeed(stats.uploadSpeedBps)}</span>
      <span className="spacer" />
      <span title="Узлов в таблице DHT — чем больше, тем легче находить раздающих">
        DHT: {stats.dhtNodes}
      </span>
      <span title="Сколько работает движок">{formatUptime(stats.uptimeSeconds)}</span>
    </div>
  )
}
