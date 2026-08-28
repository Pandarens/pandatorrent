// Pending re-uploads, plus the list of topics being watched.

import { useEffect, useState } from 'react'

import { updates as updatesApi } from '../lib/api'
import { formatBytes, formatDateTime, sizeDelta } from '../lib/format'
import { useStore } from '../lib/store'
import type { TopicUpdate, TrackedTopic } from '../lib/types'
import { Empty, Spinner } from '../components/ui'
import { UpdateModal } from '../components/UpdateModal'

export function UpdatesView() {
  const { pendingUpdates, checkingUpdates, refreshUpdates, config, toast, reportError } =
    useStore()
  const [topics, setTopics] = useState<TrackedTopic[] | null>(null)
  const [active, setActive] = useState<TopicUpdate | null>(null)

  async function loadTopics() {
    try {
      setTopics(await updatesApi.trackedTopics())
    } catch (e) {
      reportError(e, 'Отслеживаемые раздачи')
    }
  }

  useEffect(() => {
    void loadTopics()
    // Re-read the list whenever the pending set changes, so a just-applied
    // update is reflected in the baselines below.
  }, [pendingUpdates.length])

  async function checkNow() {
    try {
      const outcome = await updatesApi.checkNow()
      await refreshUpdates()
      await loadTopics()
      // The note explains a fallback run; without it a short `checked` count
      // would look like the check simply did not work.
      if (outcome.note) toast(outcome.note, 'warn')
      const tail = outcome.deferred ? ` Отложено на следующий проход: ${outcome.deferred}.` : ''
      toast(
        outcome.newUpdates.length
          ? `Найдено обновлений: ${outcome.newUpdates.length}.${tail}`
          : `Проверено раздач: ${outcome.checked}. Обновлений нет.${tail}`,
      )
    } catch (e) {
      reportError(e, 'Проверка обновлений')
    }
  }

  const interval = config?.updates.intervalMinutes ?? 0

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">Обновления раздач</h1>
        <span className="page-sub">
          {config?.updates.enabled
            ? `Автопроверка каждые ${interval >= 60 ? `${Math.round(interval / 60)} ч` : `${interval} мин`}`
            : 'Автопроверка выключена'}
        </span>
        <div className="spacer" />
        <button className="btn primary sm" onClick={checkNow} disabled={checkingUpdates}>
          {checkingUpdates ? <Spinner /> : '🔄'} Проверить сейчас
        </button>
      </div>

      <div className="banner info">
        <span>ℹ</span>
        <span>
          Сначала используется открытый JSON-API трекера — он быстрый и не требует входа.
          Пока трекер держит его выключенным, проверка идёт по страницам тем: сравнивается
          info hash раздачи. Этот способ медленнее и требует активной сессии, поэтому за
          один проход проверяется до 40 тем, начиная с самых давно не проверявшихся.
        </span>
      </div>

      <h3 className="card-title" style={{ fontSize: 16 }}>
        Доступные обновления
      </h3>

      {pendingUpdates.length === 0 ? (
        <Empty icon="✅" title="Всё актуально" hint="Ни одна отслеживаемая раздача не обновлялась." />
      ) : (
        pendingUpdates.map((u) => (
          <div className="torrent-row" key={u.id}>
            <div className="torrent-head">
              <span className="torrent-name">{u.title ?? `Раздача ${u.topicId}`}</span>
              <span className="tag warn">
                {sizeDelta(u.oldSizeBytes, u.newSizeBytes)}
              </span>
              <div className="torrent-actions">
                <button className="btn primary sm" onClick={() => setActive(u)}>
                  Обновить
                </button>
              </div>
            </div>
            <div className="torrent-stats">
              <span>Было: {formatBytes(u.oldSizeBytes)}</span>
              <span>Стало: {formatBytes(u.newSizeBytes)}</span>
              <span>Новая версия от {formatDateTime(u.newRegTime)}</span>
              <span>Обнаружено {formatDateTime(u.detectedAt)}</span>
            </div>
          </div>
        ))
      )}

      <h3 className="card-title" style={{ fontSize: 16, marginTop: 30 }}>
        Отслеживаемые раздачи ({topics?.length ?? 0})
      </h3>

      {!topics ? (
        <Spinner />
      ) : topics.length === 0 ? (
        <Empty
          icon="👀"
          title="Пока нечего отслеживать"
          hint="Раздачи попадают сюда автоматически, когда вы скачиваете их через поиск."
        />
      ) : (
        <table className="result-table">
          <thead>
            <tr>
              <th>Раздача</th>
              <th className="num">Размер</th>
              <th className="num">Версия от</th>
              <th className="num">Проверено</th>
              <th className="num">Слежение</th>
            </tr>
          </thead>
          <tbody>
            {topics.map((t) => (
              <tr key={t.topicId}>
                <td>
                  <span className="result-title">{t.title ?? `#${t.topicId}`}</span>
                  <span className="result-forum mono">{t.infoHash.slice(0, 24)}…</span>
                </td>
                <td className="num">{formatBytes(t.sizeBytes)}</td>
                <td className="num">{formatDateTime(t.regTime)}</td>
                <td className="num">{formatDateTime(t.lastCheckedAt)}</td>
                <td className="num">
                  <button
                    className={t.enabled ? 'btn sm' : 'btn sm ghost'}
                    onClick={async () => {
                      try {
                        await updatesApi.setTopicEnabled(t.topicId, !t.enabled)
                        await loadTopics()
                      } catch (e) {
                        reportError(e)
                      }
                    }}
                  >
                    {t.enabled ? 'Включено' : 'Выключено'}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {active && <UpdateModal update={active} onClose={() => setActive(null)} />}
    </div>
  )
}
