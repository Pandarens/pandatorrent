// Making a .torrent out of a file or a folder.
//
// The last thing a torrent client is expected to do that this one could not:
// share something of your own rather than only fetch other people's.

import { useState } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'

import { torrents as torrentsApi } from '../lib/api'
import { useStore } from '../lib/store'
import { Modal, Spinner } from './ui'

/** Trackers a new torrent gets unless the author names their own. */
const SUGGESTED = ['udp://tracker.opentrackr.org:1337/announce', 'udp://open.demonii.com:1337/announce']

export function CreateTorrentModal({ onClose }: { onClose: () => void }) {
  const { toast, reportError } = useStore()
  const [source, setSource] = useState('')
  const [name, setName] = useState('')
  const [trackers, setTrackers] = useState(SUGGESTED.join('\n'))
  const [busy, setBusy] = useState(false)

  async function pick(directory: boolean) {
    const picked = await open({
      title: directory ? 'Папка для раздачи' : 'Файл для раздачи',
      directory,
      multiple: false,
    })
    if (typeof picked === 'string') setSource(picked)
  }

  async function build() {
    if (!source) {
      toast('Сначала выберите, что раздавать')
      return
    }
    const saveTo = await save({
      title: 'Куда сохранить .torrent',
      defaultPath: `${name.trim() || source.split(/[\\/]/).pop() || 'torrent'}.torrent`,
      filters: [{ name: 'Торрент', extensions: ['torrent'] }],
    })
    if (!saveTo) return

    setBusy(true)
    try {
      await torrentsApi.create(
        source,
        saveTo,
        name.trim() || null,
        trackers
          .split('\n')
          .map((t) => t.trim())
          .filter(Boolean),
      )
      toast('Торрент создан')
      onClose()
    } catch (e) {
      reportError(e, 'Создание торрента')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal title="Создать торрент" onClose={onClose}>
      <div className="field">
        <label>Что раздаём</label>
        <div style={{ display: 'flex', gap: 8 }}>
          <input className="input" value={source} readOnly placeholder="Файл или папка" />
          <button className="btn" disabled={busy} onClick={() => void pick(false)}>
            Файл
          </button>
          <button className="btn" disabled={busy} onClick={() => void pick(true)}>
            Папка
          </button>
        </div>
      </div>

      <div className="field" style={{ marginTop: 12 }}>
        <label>Название</label>
        <input
          className="input"
          value={name}
          disabled={busy}
          placeholder="По умолчанию — имя файла или папки"
          onChange={(e) => setName(e.target.value)}
        />
      </div>

      <div className="field" style={{ marginTop: 12 }}>
        <label>Трекеры, по одному в строке</label>
        <textarea
          className="input"
          rows={4}
          value={trackers}
          disabled={busy}
          onChange={(e) => setTrackers(e.target.value)}
        />
        <span className="hint">
          Без трекеров раздачу найдут только через DHT — медленнее, но работает.
        </span>
      </div>

      <div style={{ display: 'flex', gap: 10, marginTop: 16 }}>
        <button className="btn primary" disabled={busy || !source} onClick={() => void build()}>
          {busy ? <Spinner /> : null} Собрать
        </button>
        <button className="btn" disabled={busy} onClick={onClose}>
          Отмена
        </button>
      </div>

      {busy && (
        <p className="hint" style={{ marginTop: 10 }}>
          Считаем контрольные суммы — для большой папки это займёт время.
        </p>
      )}
    </Modal>
  )
}
