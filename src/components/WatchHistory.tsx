// What has been watched, including releases streamed without being kept.
//
// Those leave nothing behind on disk, so this row is the only trace of them —
// which is exactly why it belongs in the library next to what is installed.

import { useEffect, useState } from 'react'

import { history as historyApi, player as playerApi } from '../lib/api'
import { formatDateTime } from '../lib/format'
import { useStore } from '../lib/store'
import type { WatchHistoryItem } from '../lib/types'
import { Spinner } from './ui'
import { ConfirmDialog } from './ConfirmDialog'

export function WatchHistory() {
  const { toast, reportError } = useStore()
  const [items, setItems] = useState<WatchHistoryItem[] | null>(null)
  const [busy, setBusy] = useState<number | null>(null)
  const [confirmingClear, setConfirmingClear] = useState(false)

  async function load() {
    try {
      setItems(await historyApi.list())
    } catch (e) {
      reportError(e, 'История просмотров')
      setItems([])
    }
  }

  useEffect(() => {
    void load()
  }, [])

  async function again(item: WatchHistoryItem) {
    if (!item.topicId) return
    setBusy(item.id)
    try {
      await playerApi.watchTopic(item.topicId, item.title)
      toast('Открываю плеер')
      await load()
    } catch (e) {
      reportError(e, 'Просмотр')
    } finally {
      setBusy(null)
    }
  }

  if (!items || items.length === 0) return null

  return (
    <div className="strip-block">
      <div className="strip-head">
        <h2 className="strip-title">История просмотров</h2>
        <span className="page-sub">{items.length}</span>
        <div className="spacer" />
        <button
          className="btn ghost sm"
          onClick={() => setConfirmingClear(true)}
        >
          Очистить
        </button>
      </div>

      {confirmingClear && (
        <ConfirmDialog
          title="Очистить историю"
          icon="🧹"
          message="Список просмотренного будет очищен. Скачанные файлы это не затронет."
          choices={[{ label: 'Очистить', value: true, kind: 'danger' }]}
          onCancel={() => setConfirmingClear(false)}
          onPick={async () => {
            setConfirmingClear(false)
            try {
              await historyApi.clear()
              await load()
            } catch (e) {
              reportError(e, 'Очистка истории')
            }
          }}
        />
      )}

      <div className="strip">
        {items.map((item) => (
          <div className="result-card" key={item.id}>
            <div className="result-art">
              {item.imageUrl ? (
                <img src={item.imageUrl} alt="" loading="lazy" />
              ) : (
                <div className="result-art-empty">🎬</div>
              )}
              {item.temporary && <span className="art-badge temp">без загрузки</span>}
            </div>
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
                <span>{formatDateTime(item.watchedAt)}</span>
              </div>
              <button
                className="btn primary sm card-dl"
                onClick={() => again(item)}
                disabled={busy === item.id || !item.topicId}
                title={item.topicId ? 'Смотреть снова' : 'Раздача неизвестна'}
              >
                {busy === item.id ? <Spinner /> : '▶'} Смотреть снова
              </button>
              <button
                className="btn ghost sm"
                style={{ width: '100%', justifyContent: 'center', marginTop: 6 }}
                onClick={async () => {
                  try {
                    await historyApi.remove(item.id)
                    await load()
                  } catch (e) {
                    reportError(e, 'Удаление')
                  }
                }}
              >
                Убрать из истории
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
