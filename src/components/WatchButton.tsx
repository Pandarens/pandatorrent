// "Watch" for a torrent that may still be downloading.
//
// Playback goes through the local streaming server, so it works long before the
// download finishes — mpv just waits for the pieces it needs.

import { useState } from 'react'

import { player as playerApi } from '../lib/api'
import { formatBytes } from '../lib/format'
import { useStore } from '../lib/store'
import type { TorrentFileEntry } from '../lib/types'
import { Empty, Modal, Spinner } from './ui'

export function WatchButton({ infoHash }: { infoHash: string }) {
  const { toast, reportError } = useStore()
  const [busy, setBusy] = useState(false)
  const [choices, setChoices] = useState<TorrentFileEntry[] | null>(null)

  async function start() {
    setBusy(true)
    try {
      const status = await playerApi.status()
      if (!status.available) {
        toast(status.problem ?? 'Проигрыватель недоступен', 'error')
        return
      }

      const files = await playerApi.videoFiles(infoHash)
      if (files.length === 0) {
        toast('В этой раздаче нет видеофайлов', 'warn')
        return
      }
      // One film: no reason to ask. A season: let the user pick an episode.
      if (files.length === 1) {
        await playerApi.play(infoHash, files[0].index)
      } else {
        setChoices(files)
      }
    } catch (e) {
      reportError(e, 'Воспроизведение')
    } finally {
      setBusy(false)
    }
  }

  async function playFile(index: number) {
    setChoices(null)
    try {
      await playerApi.play(infoHash, index)
    } catch (e) {
      reportError(e, 'Воспроизведение')
    }
  }

  return (
    <>
      <button className="btn" onClick={start} disabled={busy}>
        {busy ? <Spinner /> : '▶'} Смотреть
      </button>

      {choices && (
        <Modal title="Что включить" icon="🎬" wide onClose={() => setChoices(null)}>
          {choices.length === 0 ? (
            <Empty icon="🎬" title="Видеофайлов не найдено" />
          ) : (
            <div className="exe-list">
              {choices.map((f) => (
                <button key={f.index} className="exe-item" onClick={() => playFile(f.index)}>
                  <span style={{ fontSize: 16 }}>🎬</span>
                  <span style={{ flex: 1, minWidth: 0 }}>
                    <span className="exe-name">{f.name}</span>
                  </span>
                  <span style={{ color: 'var(--muted)', fontSize: 12 }}>
                    {formatBytes(f.length)}
                  </span>
                </button>
              ))}
            </div>
          )}
        </Modal>
      )}
    </>
  )
}
