// The files inside one download, with what has arrived so far.
//
// A download used to be a single bar and nothing else — no way to see what was
// inside, which part had landed, or to start watching the episode that had.
// This is the panel that opens underneath a row.

import { useEffect, useState } from 'react'

import { player as playerApi, torrents as torrentsApi } from '../lib/api'
import { formatBytes } from '../lib/format'
import { useStore } from '../lib/store'
import type { TorrentFileEntry } from '../lib/types'
import { Spinner } from './ui'

/** Extensions worth offering a play button for. */
const VIDEO = /\.(mkv|mp4|avi|mov|m4v|ts|webm|mpg|mpeg|wmv|flv|m2ts)$/i

export function TorrentFiles({
  infoHash,
  noSeeding,
}: {
  infoHash: string
  noSeeding: boolean
}) {
  const { toast, reportError, refreshAll } = useStore()
  const [selfish, setSelfish] = useState(noSeeding)
  const [files, setFiles] = useState<TorrentFileEntry[] | null>(null)
  const [busy, setBusy] = useState(false)

  async function load() {
    try {
      const details = await torrentsApi.details(infoHash)
      setFiles(details.files)
    } catch (e) {
      reportError(e, 'Файлы раздачи')
      setFiles([])
    }
  }

  useEffect(() => {
    void load()
    // The figures move while something is downloading, so the panel keeps up
    // rather than showing what was true when it was opened.
    const tick = window.setInterval(() => void load(), 2000)
    return () => window.clearInterval(tick)
  }, [infoHash])

  /** Turns one file on or off, keeping the rest as they are. */
  async function toggle(index: number, included: boolean) {
    if (!files) return
    const next = files
      .filter((f) => (f.index === index ? included : f.included))
      .map((f) => f.index)
    if (next.length === 0) {
      toast('Нельзя убрать все файлы сразу')
      return
    }
    setBusy(true)
    try {
      await torrentsApi.setFiles(infoHash, next)
      await load()
    } catch (e) {
      reportError(e, 'Выбор файлов')
    } finally {
      setBusy(false)
    }
  }

  async function play(index: number) {
    try {
      await playerApi.play(infoHash, index)
      toast('Открываю плеер')
    } catch (e) {
      reportError(e, 'Просмотр')
    }
  }

  if (files === null) {
    return (
      <div className="files-panel">
        <Spinner />
      </div>
    )
  }

  async function setSelfishness(on: boolean) {
    setSelfish(on)
    try {
      await torrentsApi.setNoSeeding(infoHash, on)
      await refreshAll()
    } catch (e) {
      setSelfish(!on)
      reportError(e, 'Раздача')
    }
  }

  return (
    <div className="files-panel">
      <div className="files-head">
        <label className="checkbox inline" title="Останавливать сразу после скачивания">
          <input
            type="checkbox"
            checked={selfish}
            onChange={(e) => void setSelfishness(e.target.checked)}
          />
          <span>Не раздавать эту раздачу</span>
        </label>
        <div className="spacer" />
        <span className="files-hint">
          Скорость задаётся общая — в настройках и по расписанию
        </span>
      </div>

      {files.map((f) => {
        const percent = f.length > 0 ? Math.min(100, (f.downloaded / f.length) * 100) : 0
        const ready = percent >= 99.9
        return (
          <div className={f.included ? 'file-row' : 'file-row off'} key={f.index}>
            <input
              type="checkbox"
              checked={f.included}
              disabled={busy}
              title={f.included ? 'Не скачивать этот файл' : 'Скачивать этот файл'}
              onChange={(e) => void toggle(f.index, e.target.checked)}
            />

            <span className="file-name" title={f.components.join('/')}>
              {f.name}
            </span>

            <span className="file-bar" title={`${percent.toFixed(1)}%`}>
              <span style={{ width: `${percent}%` }} />
            </span>

            <span className="file-size">
              {formatBytes(f.downloaded)} / {formatBytes(f.length)}
            </span>

            {VIDEO.test(f.name) && (
              <button
                className="btn ghost sm"
                title={ready ? 'Смотреть' : 'Смотреть, догружая по ходу'}
                onClick={() => void play(f.index)}
              >
                ▶
              </button>
            )}
          </div>
        )
      })}
    </div>
  )
}
