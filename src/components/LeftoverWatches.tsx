// What a previous run left behind in the watch cache.
//
// Streaming a film downloads it into a scratch folder that normally clears
// itself. A crash or a reboot skips all of that, and the part-downloaded film
// is exactly what somebody came back to finish — so it is offered back rather
// than quietly deleted.

import { useEffect, useState } from 'react'

import { leftovers as leftoversApi, player as playerApi } from '../lib/api'
import { formatBytes } from '../lib/format'
import { useStore } from '../lib/store'
import type { Leftover } from '../lib/types'
import { Spinner } from './ui'

export function LeftoverWatches() {
  const { toast, reportError } = useStore()
  const [items, setItems] = useState<Leftover[]>([])
  const [busy, setBusy] = useState<string | null>(null)

  async function load() {
    try {
      setItems(await leftoversApi.list())
    } catch {
      // A leftover nobody can list is one nobody has to decide about.
      setItems([])
    }
  }

  useEffect(() => {
    void load()
  }, [])

  async function act(item: Leftover, what: 'resume' | 'save' | 'drop') {
    setBusy(item.infoHash)
    try {
      if (what === 'resume') {
        await leftoversApi.resume(item.infoHash)
        const files = await playerApi.videoFiles(item.infoHash)
        if (files.length === 0) throw new Error('в раздаче нет видеофайлов')
        await playerApi.play(item.infoHash, files[0].index)
        toast('Продолжаю просмотр')
      } else if (what === 'save') {
        const where = await leftoversApi.save(item.infoHash)
        toast(`Сохранено в ${where}`)
      } else {
        await leftoversApi.drop(item.infoHash)
        toast('Место освобождено')
      }
      await load()
    } catch (e) {
      reportError(e, 'Незакрытый просмотр')
    } finally {
      setBusy(null)
    }
  }

  if (items.length === 0) return null

  return (
    <>
      {items.map((item) => (
        <div className="banner leftover" key={item.infoHash}>
          <span>🎬</span>
          <div className="leftover-text">
            <div className="leftover-title" title={item.title}>
              {item.title}
            </div>
            <div className="leftover-sub">
              Остался с прошлого раза: скачано {formatBytes(item.bytesOnDisk)}
              {item.totalBytes > 0 ? ` из ${formatBytes(item.totalBytes)}` : ''}. Продолжить
              просмотр, оставить себе или освободить место?
            </div>
          </div>

          <div className="spacer" />

          {busy === item.infoHash ? (
            <Spinner />
          ) : (
            <>
              <button className="btn primary sm" onClick={() => void act(item, 'resume')}>
                Досмотреть
              </button>
              <button
                className="btn sm"
                title="Перенести в папку загрузок и оставить насовсем"
                onClick={() => void act(item, 'save')}
              >
                Сохранить
              </button>
              <button className="btn ghost sm" onClick={() => void act(item, 'drop')}>
                Удалить
              </button>
            </>
          )}
        </div>
      ))}
    </>
  )
}
