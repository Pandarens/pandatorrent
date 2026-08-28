// Display helpers. Everything user-facing is Russian.

const UNITS = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ', 'ПБ']

export function formatBytes(bytes: number | null | undefined, digits = 1): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes === 0) return '0 Б'
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1)
  const value = bytes / 1024 ** i
  // Whole bytes never need a fractional part.
  return `${value.toFixed(i === 0 ? 0 : digits)} ${UNITS[i]}`
}

export function formatSpeed(bytesPerSecond: number): string {
  if (!bytesPerSecond) return '—'
  return `${formatBytes(bytesPerSecond)}/с`
}

export function formatEta(seconds: number | null): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return '—'
  if (seconds > 60 * 60 * 24 * 30) return '∞'

  const d = Math.floor(seconds / 86400)
  const h = Math.floor((seconds % 86400) / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)

  if (d > 0) return `${d} д ${h} ч`
  if (h > 0) return `${h} ч ${m} мин`
  if (m > 0) return `${m} мин ${s} с`
  return `${s} с`
}

export function formatDate(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return '—'
  return new Date(unixSeconds * 1000).toLocaleDateString('ru-RU', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
  })
}

export function formatDateTime(unixSeconds: number | null | undefined): string {
  if (!unixSeconds) return '—'
  return new Date(unixSeconds * 1000).toLocaleString('ru-RU', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function formatPlaytime(seconds: number): string {
  if (!seconds) return 'Ещё не запускали'
  const hours = seconds / 3600
  if (hours < 1) return `${Math.round(seconds / 60)} мин`
  return `${hours.toFixed(1)} ч`
}

export function progressPercent(done: number, total: number): number {
  if (!total) return 0
  return Math.min(100, Math.max(0, (done / total) * 100))
}

/** Human label for the torrent state shown next to the progress bar. */
export function stateLabel(
  state: string,
  finished: boolean,
  hasError: boolean,
): string {
  if (hasError) return 'Ошибка'
  if (finished && state !== 'paused') return 'Раздаётся'
  switch (state) {
    case 'initializing':
      return 'Проверка файлов'
    case 'live':
      return 'Загрузка'
    case 'paused':
      return finished ? 'Остановлен' : 'Пауза'
    case 'error':
      return 'Ошибка'
    default:
      return state
  }
}

/** Difference between two sizes, as a signed human string. */
export function sizeDelta(oldBytes: number | null, newBytes: number | null): string {
  if (oldBytes == null || newBytes == null) return '—'
  const delta = newBytes - oldBytes
  if (delta === 0) return 'размер не изменился'
  const sign = delta > 0 ? '+' : '−'
  return `${sign}${formatBytes(Math.abs(delta))}`
}
