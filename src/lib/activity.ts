// What the application is busy with right now.
//
// A search behind a Cloudflare check can take twenty seconds, and for all
// that time the interface showed a spinner and nothing else — so it was
// impossible to tell "working" from "stuck". Every backend call passes through
// here on its way out; the ones that can take a while carry a label, and the
// status line shows the oldest of them with a running clock.

type Listener = () => void

/** Commands worth naming while they run. Anything else is over in a blink. */
const LABELS: Record<string, string> = {
  rutracker_search: 'Ищу на трекере',
  rutracker_catalog: 'Открываю раздел',
  rutracker_all_forums: 'Загружаю каталог',
  home_new_releases: 'Загружаю новинки',
  rutracker_topic: 'Открываю раздачу',
  rutracker_topic_preview: 'Открываю раздачу',
  rutracker_download: 'Забираю торрент с трекера',
  rutracker_verify: 'Проверяю вход на трекер',
  rutracker_selftest: 'Проверяю связь с трекером',
  player_watch_topic: 'Готовлю просмотр',
  player_play: 'Открываю плеер',
  torrent_recheck: 'Проверяю файлы',
  torrent_redownload_file: 'Перекачиваю файл',
  torrent_create: 'Собираю торрент',
  torrent_add_url: 'Добавляю торрент',
  torrent_add_file: 'Добавляю торрент',
  updates_check_now: 'Проверяю обновления раздач',
  app_update_check: 'Проверяю обновление приложения',
  app_update_install: 'Устанавливаю обновление',
  leftover_save: 'Переношу просмотр в загрузки',
  settings_import: 'Загружаю настройки',
}

const inflight = new Map<number, { label: string; since: number }>()
const listeners = new Set<Listener>()
let sequence = 0
// A stable snapshot, so subscribers only re-render when something changed.
let snapshot = ''

function publish() {
  const oldest = [...inflight.values()].sort((a, b) => a.since - b.since)[0]
  const next = oldest ? `${oldest.label}|${oldest.since}` : ''
  if (next === snapshot) return
  snapshot = next
  listeners.forEach((l) => l())
}

/** Marks a command as running; call the result when it is done. */
export function begin(command: string): () => void {
  const label = LABELS[command]
  if (!label) return () => {}
  const id = ++sequence
  inflight.set(id, { label, since: Date.now() })
  publish()
  return () => {
    inflight.delete(id)
    publish()
  }
}

export function subscribe(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** The oldest labelled call in flight, or null when idle. */
export function current(): { label: string; since: number } | null {
  if (!snapshot) return null
  const at = snapshot.lastIndexOf('|')
  return { label: snapshot.slice(0, at), since: Number(snapshot.slice(at + 1)) }
}

export function getSnapshot(): string {
  return snapshot
}
