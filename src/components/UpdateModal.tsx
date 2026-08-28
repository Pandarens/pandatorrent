// "The release was updated — want to update your copy?"
//
// This is the prompt the whole update watcher exists to show.

import { useState } from 'react'

import { updates as updatesApi } from '../lib/api'
import { formatBytes, formatDateTime, sizeDelta } from '../lib/format'
import { useStore } from '../lib/store'
import type { TopicUpdate } from '../lib/types'
import { Modal, Spinner } from './ui'

export function UpdateModal({
  update,
  onClose,
}: {
  update: TopicUpdate
  onClose: () => void
}) {
  const { refreshUpdates, refreshTorrents, refreshLibrary, toast, reportError } = useStore()
  const [busy, setBusy] = useState(false)

  async function apply() {
    setBusy(true)
    try {
      await updatesApi.apply(update.id)
      toast('Обновление скачивается — уже имеющиеся файлы будут использованы повторно')
      await Promise.all([refreshUpdates(), refreshTorrents(), refreshLibrary()])
      onClose()
    } catch (e) {
      reportError(e, 'Не удалось обновить')
    } finally {
      setBusy(false)
    }
  }

  async function dismiss() {
    setBusy(true)
    try {
      await updatesApi.dismiss(update.id)
      await Promise.all([refreshUpdates(), refreshLibrary()])
      onClose()
    } catch (e) {
      reportError(e, 'Не удалось скрыть')
    } finally {
      setBusy(false)
    }
  }

  async function stopTracking() {
    setBusy(true)
    try {
      await updatesApi.setTopicEnabled(update.topicId, false)
      await updatesApi.dismiss(update.id)
      toast('Слежение за этой раздачей отключено', 'warn')
      await Promise.all([refreshUpdates(), refreshLibrary()])
      onClose()
    } catch (e) {
      reportError(e, 'Не удалось отключить слежение')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal
      title="Раздача обновилась"
      icon="🔄"
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={stopTracking} disabled={busy}>
            Больше не следить
          </button>
          <div className="spacer" />
          <button className="btn" onClick={dismiss} disabled={busy}>
            Позже
          </button>
          <button className="btn primary" onClick={apply} disabled={busy}>
            {busy ? <Spinner /> : null}
            Обновить
          </button>
        </>
      }
    >
      <p style={{ marginTop: 0, fontSize: 15, fontWeight: 600 }}>
        {update.title ?? `Раздача ${update.topicId}`}
      </p>
      <p style={{ color: 'var(--text-dim)', marginTop: 6 }}>
        На трекере опубликована новая версия раздачи
        {update.newRegTime ? ` от ${formatDateTime(update.newRegTime)}` : ''}.
      </p>

      <div className="diff-row">
        <div className="diff-col">
          <div className="label">Сейчас у вас</div>
          <div className="value">{formatBytes(update.oldSizeBytes)}</div>
          <div className="mono" style={{ color: 'var(--muted)', marginTop: 4 }}>
            {update.oldInfoHash.slice(0, 16)}…
          </div>
        </div>
        <div className="diff-arrow">→</div>
        <div className="diff-col">
          <div className="label">Новая версия</div>
          <div className="value">{formatBytes(update.newSizeBytes)}</div>
          <div className="mono" style={{ color: 'var(--muted)', marginTop: 4 }}>
            {update.newInfoHash.slice(0, 16)}…
          </div>
        </div>
      </div>

      <div className="banner info" style={{ marginBottom: 0 }}>
        <span>ℹ</span>
        <span>
          Изменение объёма: <strong>{sizeDelta(update.oldSizeBytes, update.newSizeBytes)}</strong>.
          При обновлении файлы скачиваются в ту же папку, и всё, что не изменилось,
          берётся с диска — заново качается только разница.
        </span>
      </div>
    </Modal>
  )
}
