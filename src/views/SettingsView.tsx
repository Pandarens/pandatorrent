// Settings. Engine-level options need a restart, and the UI says so rather
// than pretending the change took effect.

import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import {
  appUpdate as appUpdateApi,
  player as playerApi,
  settings as settingsApi,
  tracker,
} from '../lib/api'
import { useStore } from '../lib/store'
import type {
  AppConfig,
  AppInfo,
  AppUpdate,
  ForumEntry,
  PlayerStatus,
  SelfTest,
} from '../lib/types'
import { Spinner } from '../components/ui'
import { TrackerLogin } from '../components/TrackerLogin'

export function SettingsView() {
  const { config, trackerStatus, refreshConfig, refreshTracker, toast, reportError } = useStore()

  const [draft, setDraft] = useState<AppConfig | null>(null)
  const [mirrors, setMirrors] = useState<string[]>([])
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [saving, setSaving] = useState(false)
  const [needLogin, setNeedLogin] = useState(false)
  const [restartNeeded, setRestartNeeded] = useState(false)
  const [diag, setDiag] = useState<SelfTest | null>(null)
  const [diagBusy, setDiagBusy] = useState(false)
  const [allForums, setAllForums] = useState<ForumEntry[] | null>(null)
  const [forumFilter, setForumFilter] = useState('')
  const [playerState, setPlayerState] = useState<PlayerStatus | null>(null)
  const [update, setUpdate] = useState<AppUpdate | null>(null)
  const [updateBusy, setUpdateBusy] = useState(false)
  const [updateProgress, setUpdateProgress] = useState<number | null>(null)

  useEffect(() => {
    if (config) setDraft(structuredClone(config))
  }, [config])

  useEffect(() => {
    settingsApi.mirrors().then(setMirrors).catch(() => setMirrors([]))
    settingsApi.appInfo().then(setInfo).catch(() => setInfo(null))
    playerApi.status().then(setPlayerState).catch(() => setPlayerState(null))
  }, [])

  if (!draft) {
    return (
      <div className="page">
        <Spinner />
      </div>
    )
  }

  // Typed partial updates keep the nested config readable at call sites.
  function patch(fn: (d: AppConfig) => void) {
    setDraft((prev) => {
      if (!prev) return prev
      const next = structuredClone(prev)
      fn(next)
      return next
    })
  }

  async function save() {
    if (!draft) return
    setSaving(true)
    try {
      const res = await settingsApi.set(draft)
      setRestartNeeded(res.restartRequired)
      await refreshConfig()
      toast(
        res.restartRequired
          ? 'Сохранено. Часть настроек применится после перезапуска.'
          : 'Настройки сохранены',
      )
    } catch (e) {
      reportError(e, 'Сохранение')
    } finally {
      setSaving(false)
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, multiple: false })
    if (typeof picked === 'string') patch((d) => (d.downloadDir = picked))
  }

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">Настройки</h1>
        <div className="spacer" />
        <button className="btn primary" onClick={save} disabled={saving}>
          {saving ? <Spinner /> : null} Сохранить
        </button>
      </div>

      {restartNeeded && (
        <div className="banner warn">
          <span>⚠</span>
          <span>
            Параметры сети и папка загрузок применяются при следующем запуске приложения.
          </span>
        </div>
      )}

      <div className="card">
        <h3 className="card-title">Аккаунт RuTracker</h3>
        {trackerStatus?.hasSession ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span className="tag accent">
              Вход выполнен{trackerStatus.username ? `: ${trackerStatus.username}` : ''}
            </span>
            <div className="spacer" />
            <button
              className="btn danger sm"
              onClick={async () => {
                try {
                  await tracker.logout()
                  await refreshTracker()
                  toast('Выход выполнен')
                } catch (e) {
                  reportError(e, 'Выход')
                }
              }}
            >
              Выйти
            </button>
          </div>
        ) : (
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span className="page-sub">Вход не выполнен — поиск и скачивание недоступны.</span>
            <div className="spacer" />
            <button className="btn primary sm" onClick={() => setNeedLogin(true)}>
              Войти
            </button>
          </div>
        )}

        <div style={{ marginTop: 16, display: 'flex', alignItems: 'center', gap: 12 }}>
          <button
            className="btn sm"
            disabled={diagBusy}
            onClick={async () => {
              setDiagBusy(true)
              setDiag(null)
              try {
                setDiag(await tracker.selftest())
              } catch (e) {
                reportError(e, 'Диагностика')
              } finally {
                setDiagBusy(false)
              }
            }}
          >
            {diagBusy ? <Spinner /> : '🩺'} Проверить связь с трекером
          </button>
          {diag && (
            <span className={diag.ok ? 'tag accent' : 'tag warn'}>
              {diag.message}
              {diag.ok ? ` · ${(diag.bytes / 1024).toFixed(0)} КБ` : ''}
            </span>
          )}
        </div>
        {diagBusy && (
          <p className="hint" style={{ marginTop: 8 }}>
            Открывается фоновое окно браузера и проходится проверка Cloudflare — это
            может занять несколько секунд.
          </p>
        )}

        <div className="field" style={{ marginTop: 18 }}>
          <label>Зеркало трекера</label>
          <select
            className="select"
            value={draft.rutracker.host}
            onChange={(e) => patch((d) => (d.rutracker.host = e.target.value))}
          >
            {(mirrors.length ? mirrors : [draft.rutracker.host]).map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
          <span className="hint">
            Если основной домен заблокирован провайдером, выберите другое зеркало.
          </span>
        </div>

        <div className="field">
          <label>Прокси для трекера</label>
          <input
            className="input"
            placeholder="socks5://127.0.0.1:1080 или http://user:pass@host:port"
            value={draft.network.trackerProxy ?? ''}
            onChange={(e) =>
              patch((d) => (d.network.trackerProxy = e.target.value.trim() || null))
            }
          />
          <span className="hint">
            Применяется к API трекера и загрузке обложек. Страницы трекера открываются
            во встроенном браузере — он использует системный прокси Windows. Обмен с
            пирами идёт напрямую.
          </span>
        </div>
      </div>

      <div className="card">
        <h3 className="card-title">Загрузки</h3>
        <div className="field">
          <label>Папка для скачивания</label>
          <div style={{ display: 'flex', gap: 8 }}>
            <input className="input mono" value={draft.downloadDir} readOnly />
            <button className="btn" onClick={pickFolder}>
              Обзор…
            </button>
          </div>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <div className="field">
            <label>Ограничение скачивания, КБ/с</label>
            <input
              className="input"
              type="number"
              min={0}
              value={draft.network.downloadLimitKbps}
              onChange={(e) =>
                patch((d) => (d.network.downloadLimitKbps = Number(e.target.value) || 0))
              }
            />
            <span className="hint">0 — без ограничения</span>
          </div>
          <div className="field">
            <label>Ограничение раздачи, КБ/с</label>
            <input
              className="input"
              type="number"
              min={0}
              value={draft.network.uploadLimitKbps}
              onChange={(e) =>
                patch((d) => (d.network.uploadLimitKbps = Number(e.target.value) || 0))
              }
            />
            <span className="hint">0 — без ограничения</span>
          </div>
          <div className="field">
            <label>Порт (0 — выбрать автоматически)</label>
            <input
              className="input"
              type="number"
              min={0}
              max={65535}
              value={draft.network.listenPort}
              onChange={(e) => patch((d) => (d.network.listenPort = Number(e.target.value) || 0))}
            />
          </div>
          <div className="field">
            <label>Максимум пиров на торрент</label>
            <input
              className="input"
              type="number"
              min={10}
              max={500}
              value={draft.network.maxPeersPerTorrent}
              onChange={(e) =>
                patch((d) => (d.network.maxPeersPerTorrent = Number(e.target.value) || 100))
              }
            />
          </div>
        </div>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.network.enableDht}
            onChange={(e) => patch((d) => (d.network.enableDht = e.target.checked))}
          />
          <span>DHT — поиск пиров без трекера</span>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.network.enableUpnp}
            onChange={(e) => patch((d) => (d.network.enableUpnp = e.target.checked))}
          />
          <span>UPnP — автоматически пробрасывать порт на роутере</span>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.network.enableLsd}
            onChange={(e) => patch((d) => (d.network.enableLsd = e.target.checked))}
          />
          <span>Поиск пиров в локальной сети</span>
        </label>
      </div>

      <div className="card">
        <h3 className="card-title">Новинки на главном экране</h3>
        <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
          Выбранные разделы показываются лентами над библиотекой, отсортированные по
          свежести. Список кешируется на 30 минут, чтобы открытие библиотеки не было
          пачкой запросов к трекеру.
        </p>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.home.enabled}
            onChange={(e) => patch((d) => (d.home.enabled = e.target.checked))}
          />
          <span>Показывать новинки</span>
        </label>

        <div className="field" style={{ marginTop: 12, maxWidth: 260 }}>
          <label>Раздач в каждой ленте</label>
          <input
            className="input"
            type="number"
            min={1}
            max={50}
            value={draft.home.perForum}
            onChange={(e) =>
              patch((d) => (d.home.perForum = Math.min(50, Math.max(1, Number(e.target.value) || 8))))
            }
          />
        </div>

        <div className="field">
          <label>Закреплённые разделы</label>
          {draft.home.forums.length === 0 ? (
            <span className="hint">Ни одного раздела не выбрано — ленты не появятся.</span>
          ) : (
            <div className="pinned-list">
              {draft.home.forums.map((f, i) => (
                <div className="pinned-row" key={f.id}>
                  <span className="grow" title={f.title}>
                    {f.title}
                  </span>
                  <span className="page-sub mono">f={f.id}</span>
                  <button
                    className="btn ghost sm"
                    title="Выше"
                    disabled={i === 0}
                    onClick={() =>
                      patch((d) => {
                        const list = d.home.forums
                        ;[list[i - 1], list[i]] = [list[i], list[i - 1]]
                      })
                    }
                  >
                    ↑
                  </button>
                  <button
                    className="btn danger sm"
                    title="Убрать"
                    onClick={() =>
                      patch((d) => {
                        d.home.forums = d.home.forums.filter((x) => x.id !== f.id)
                      })
                    }
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="field">
          <label>Добавить раздел</label>
          <input
            className="input"
            placeholder="Начните вводить название, например «Новинки»"
            value={forumFilter}
            onFocus={() => {
              if (!allForums) {
                tracker
                  .allForums()
                  .then(setAllForums)
                  .catch((e) => reportError(e, 'Каталог разделов'))
              }
            }}
            onChange={(e) => setForumFilter(e.target.value)}
          />
          {forumFilter.trim().length > 0 && (
            <div className="forum-picker">
              {!allForums && (
                <div style={{ padding: 10 }}>
                  <Spinner /> Загрузка каталога…
                </div>
              )}
              {allForums
                ?.filter(
                  (f) =>
                    f.title.toLowerCase().includes(forumFilter.trim().toLowerCase()) &&
                    !draft.home.forums.some((p) => p.id === f.id),
                )
                .slice(0, 40)
                .map((f) => (
                  <button
                    key={f.id}
                    onClick={() => {
                      patch((d) => d.home.forums.push({ id: f.id, title: f.title }))
                      setForumFilter('')
                    }}
                  >
                    {f.title}
                  </button>
                ))}
            </div>
          )}
          <span className="hint">
            Каталог берётся с главной страницы трекера и включает подразделы вроде
            «Игры для Windows · Новинки».
          </span>
        </div>
      </div>

      <div className="card">
        <h3 className="card-title">Проверка обновлений раздач</h3>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.updates.enabled}
            onChange={(e) => patch((d) => (d.updates.enabled = e.target.checked))}
          />
          <span>Следить за обновлениями скачанных раздач</span>
        </label>

        <div className="field" style={{ marginTop: 12, maxWidth: 320 }}>
          <label>Интервал проверки</label>
          <select
            className="select"
            value={draft.updates.intervalMinutes}
            onChange={(e) => patch((d) => (d.updates.intervalMinutes = Number(e.target.value)))}
          >
            <option value={60}>Каждый час</option>
            <option value={180}>Каждые 3 часа</option>
            <option value={360}>Каждые 6 часов</option>
            <option value={720}>Каждые 12 часов</option>
            <option value={1440}>Раз в сутки</option>
          </select>
        </div>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.updates.checkOnStartup}
            onChange={(e) => patch((d) => (d.updates.checkOnStartup = e.target.checked))}
          />
          <span>Проверять при запуске приложения</span>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.updates.notifyDesktop}
            onChange={(e) => patch((d) => (d.updates.notifyDesktop = e.target.checked))}
          />
          <span>Показывать системные уведомления</span>
        </label>
      </div>

      <div className="card">
        <h3 className="card-title">Просмотр видео</h3>
        <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
          Фильм открывается во встроенном mpv и играет прямо во время скачивания:
          файл отдаётся локальным сервером, который ждёт нужные куски. mpv понимает
          MKV, HEVC, AC3/DTS — всё, что не умеет встроенный движок браузера.
        </p>

        {playerState && !playerState.available && (
          <div className="banner warn">
            <span>⚠</span>
            <span>
              Библиотека mpv не найдена, воспроизведение недоступно.{' '}
              {playerState.problem}
            </span>
          </div>
        )}
        {playerState?.available && (
          <span className="tag accent">mpv найден и готов</span>
        )}

        <div className="field" style={{ marginTop: 16, maxWidth: 360 }}>
          <label>Выравнивание громкости</label>
          <select
            className="select"
            value={draft.player.audioNormalize}
            onChange={(e) => patch((d) => (d.player.audioNormalize = e.target.value))}
          >
            <option value="off">Выключено</option>
            <option value="dynaudnorm">Динамическое (тихие диалоги громче)</option>
            <option value="loudnorm">По стандарту вещания (EBU R128)</option>
          </select>
          <span className="hint">
            Убирает перепад между тихими разговорами и громким экшеном.
          </span>
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <div className="field">
            <label>Громкость, %</label>
            <input
              className="input"
              type="number"
              min={0}
              max={150}
              value={draft.player.volume}
              onChange={(e) =>
                patch((d) => (d.player.volume = Math.min(150, Math.max(0, Number(e.target.value) || 0))))
              }
            />
            <span className="hint">Выше 100 — усиление</span>
          </div>
          <div className="field">
            <label>Языки дорожек и субтитров</label>
            <input
              className="input"
              value={draft.player.audioLang}
              onChange={(e) => patch((d) => (d.player.audioLang = e.target.value))}
            />
            <input
              className="input"
              style={{ marginTop: 6 }}
              value={draft.player.subtitleLang}
              onChange={(e) => patch((d) => (d.player.subtitleLang = e.target.value))}
            />
            <span className="hint">Порядок приоритета, например rus,eng</span>
          </div>
        </div>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.player.hardwareDecoding}
            onChange={(e) => patch((d) => (d.player.hardwareDecoding = e.target.checked))}
          />
          <span>Аппаратное декодирование (выключите, если видео рассыпается)</span>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.player.onScreenControls}
            onChange={(e) => patch((d) => (d.player.onScreenControls = e.target.checked))}
          />
          <span>Экранные кнопки управления в плеере</span>
        </label>

        <div className="field" style={{ marginTop: 12 }}>
          <label>Дополнительные параметры mpv</label>
          <input
            className="input"
            placeholder="например: sub-font-size=48, через запятую"
            value={draft.player.extraOptions.join(', ')}
            onChange={(e) =>
              patch(
                (d) =>
                  (d.player.extraOptions = e.target.value
                    .split(',')
                    .map((x) => x.trim())
                    .filter(Boolean)),
              )
            }
          />
          <span className="hint">
            Каждый параметр в виде имя=значение. Неизвестные mpv просто игнорирует.
          </span>
        </div>
      </div>

      <div className="card">
        <h3 className="card-title">Интерфейс</h3>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.ui.minimizeToTray}
            onChange={(e) => patch((d) => (d.ui.minimizeToTray = e.target.checked))}
          />
          <span>Сворачивать в трей вместо закрытия (раздача продолжается)</span>
        </label>
      </div>

      <div className="card">
        <h3 className="card-title">Раздача</h3>
        <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
          Скачанное раздаётся дальше — на трекерах от этого зависит рейтинг.
          Можно остановить раздачу, когда отдано достаточно. Ноль означает
          раздавать без ограничения.
        </p>
        <label className="field" style={{ maxWidth: 280 }}>
          <span>Остановить при рейтинге</span>
          <input
            type="number"
            min={0}
            max={100}
            step={0.1}
            value={draft.seeding.ratioLimit}
            onChange={(e) =>
              patch((d) => (d.seeding.ratioLimit = Math.max(0, Number(e.target.value) || 0)))
            }
          />
        </label>
      </div>

      <div className="card">
        <h3 className="card-title">Выключение компьютера</h3>
        <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
          Перед выключением появится обратный отсчёт — любая клавиша или клик его
          отменяют. По умолчанию выключено: приложение не должно гасить компьютер
          неожиданно.
        </p>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.power.afterPlayback}
            onChange={(e) => patch((d) => (d.power.afterPlayback = e.target.checked))}
          />
          <span>Выключать после того, как фильм или сезон досмотрен</span>
        </label>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.power.afterDownloads}
            onChange={(e) => patch((d) => (d.power.afterDownloads = e.target.checked))}
          />
          <span>Выключать после того, как все загрузки завершены</span>
        </label>

        <label className="field" style={{ maxWidth: 280, marginTop: 10 }}>
          <span>Сколько ждать перед выключением, секунд</span>
          <input
            type="number"
            min={10}
            max={600}
            value={draft.power.delaySeconds}
            onChange={(e) =>
              patch((d) => (d.power.delaySeconds = Math.max(10, Number(e.target.value) || 60)))
            }
          />
        </label>
      </div>

      <div className="card">
        <h3 className="card-title">Журнал работы</h3>
        <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
          Приложение записывает, что оно делает, в файл. Если что-то пошло не так,
          журнал показывает причину. Записи старше недели удаляются сами.
        </p>
        <button
          className="btn"
          onClick={() => void settingsApi.openLogs().catch((e) => reportError(e, 'Журнал'))}
        >
          Открыть папку журнала
        </button>
      </div>

      <div className="card">
        <h3 className="card-title">Обновление приложения</h3>
        <p style={{ color: 'var(--muted)', fontSize: 12.5, marginTop: 0 }}>
          Новые версии берутся из релизов на GitHub. Устанавливаются только сборки,
          подписанные ключом проекта.
        </p>

        <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
          <button
            className="btn"
            disabled={updateBusy}
            onClick={async () => {
              setUpdateBusy(true)
              try {
                setUpdate(await appUpdateApi.check())
              } catch (e) {
                reportError(e, 'Проверка обновлений')
              } finally {
                setUpdateBusy(false)
              }
            }}
          >
            {updateBusy && updateProgress == null ? <Spinner /> : '⟳'} Проверить обновления
          </button>

          {update && !update.available && !update.message && (
            <span className="tag accent">Установлена последняя версия {update.currentVersion}</span>
          )}
          {update?.message && <span className="tag">{update.message}</span>}
          {update?.available && (
            <span className="tag warn">Доступна версия {update.version}</span>
          )}
        </div>

        {update?.available && (
          <>
            {update.notes && (
              <p style={{ whiteSpace: 'pre-wrap', color: 'var(--text-dim)', fontSize: 13 }}>
                {update.notes}
              </p>
            )}
            <button
              className="btn primary"
              style={{ marginTop: 10 }}
              disabled={updateBusy}
              onClick={async () => {
                setUpdateBusy(true)
                setUpdateProgress(0)
                const stop = await appUpdateApi.onProgress(setUpdateProgress)
                try {
                  // Succeeds by restarting the app, so nothing after this runs.
                  await appUpdateApi.install()
                } catch (e) {
                  reportError(e, 'Установка обновления')
                  setUpdateBusy(false)
                  setUpdateProgress(null)
                  stop()
                }
              }}
            >
              {updateProgress != null
                ? `Загрузка… ${updateProgress}%`
                : 'Обновить и перезапустить'}
            </button>
            <p className="hint" style={{ marginTop: 8 }}>
              Загрузки будут остановлены, приложение перезапустится само.
            </p>
          </>
        )}
      </div>

      {info && (
        <div className="card">
          <h3 className="card-title">О программе</h3>
          <dl className="kv">
            <dt>Версия</dt>
            <dd>{info.version}</dd>
            <dt>Данные</dt>
            <dd className="mono">{info.dataDir}</dd>
            <dt>Обложки</dt>
            <dd className="mono">{info.coversDir}</dd>
          </dl>
        </div>
      )}

      {needLogin && <TrackerLogin onClose={() => setNeedLogin(false)} />}
    </div>
  )
}
