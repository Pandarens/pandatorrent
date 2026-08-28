// Sign-in flow for RuTracker.
//
// There is no form to POST: Cloudflare answers every HTTP client that is not a
// real browser with a JavaScript challenge. So the app opens the tracker's own
// login page in a separate browser window and waits for the session to appear.
// The captcha, and anything the tracker adds later, just work — and the app
// never handles the password at all.
//
// Detection deliberately does not rely on the page-state event alone. While the
// dialog is waiting it polls `verify()`, which fetches a forum page and looks
// for the logout link. That is the definitive answer, it works even if the
// injected agent goes quiet, and a failure shows up as a message here instead
// of a dialog that waits forever.

import { useCallback, useEffect, useRef, useState } from 'react'

import { asAppError, onTrackerAuth, tracker } from '../lib/api'
import { useStore } from '../lib/store'
import type { TrackerStatus } from '../lib/types'
import { Modal, Spinner } from './ui'

/** How often to ask the tracker whether the session exists yet. */
const POLL_MS = 3000

export function TrackerLogin({ onClose }: { onClose: () => void }) {
  const { refreshTracker, toast, reportError } = useStore()
  const [waiting, setWaiting] = useState(false)
  const [checking, setChecking] = useState(false)
  const [probe, setProbe] = useState<TrackerStatus | null>(null)
  const [problem, setProblem] = useState<string | null>(null)

  // Guards against a poll and a manual check both "winning" at once.
  const settled = useRef(false)
  // A check can outlast the interval (Cloudflare, a slow page); overlapping
  // calls would queue up behind each other and make the dialog feel stuck.
  const inFlight = useRef(false)

  const succeed = useCallback(async () => {
    if (settled.current) return
    settled.current = true
    await tracker.hideLogin().catch(() => {})
    await refreshTracker()
    toast('Вход на RuTracker выполнен')
    onClose()
  }, [refreshTracker, toast, onClose])

  // The worker page also reports state changes on its own; if that path works,
  // this closes the dialog a beat sooner than the poll would. Held in a ref so
  // the subscription is set up once instead of churning on every poll render.
  const succeedRef = useRef(succeed)
  succeedRef.current = succeed

  useEffect(() => {
    const un = onTrackerAuth((loggedIn) => {
      if (loggedIn) void succeedRef.current()
    })
    return () => {
      void un.then((f) => f())
    }
  }, [])

  useEffect(() => {
    if (!waiting) return
    let cancelled = false

    const tick = async () => {
      if (inFlight.current || settled.current) return
      inFlight.current = true
      try {
        const status = await tracker.verify()
        if (cancelled) return
        setProbe(status)
        setProblem(null)
        if (status.hasSession) void succeedRef.current()
      } catch (e) {
        if (cancelled) return
        // Expected while the user is still on the login page or solving the
        // Cloudflare check; only worth showing, not worth aborting on.
        setProblem(asAppError(e).message)
      } finally {
        inFlight.current = false
      }
    }

    // Fires immediately too: the session may already be live when the dialog
    // is reopened after the user closed the tracker window by hand.
    void tick()
    const timer = window.setInterval(tick, POLL_MS)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [waiting])

  async function openWindow() {
    setWaiting(true)
    setProblem(null)
    try {
      await tracker.openLogin()
    } catch (e) {
      reportError(e, 'Не удалось открыть окно входа')
      setWaiting(false)
    }
  }

  async function checkNow() {
    setChecking(true)
    try {
      const status = await tracker.verify()
      setProbe(status)
      if (status.hasSession) {
        await succeed()
      } else {
        toast(
          status.hasCookie
            ? 'Кука сессии есть, но страница ещё не показывает вход — подождите пару секунд'
            : 'Активной сессии пока не видно — завершите вход в окне трекера',
          'warn',
        )
      }
    } catch (e) {
      reportError(e, 'Проверка сессии')
    } finally {
      setChecking(false)
    }
  }

  return (
    <Modal
      title="Вход на RuTracker"
      icon="🔐"
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            Закрыть
          </button>
          <div className="spacer" />
          {waiting && (
            <button className="btn" onClick={checkNow} disabled={checking}>
              {checking ? <Spinner /> : null} Проверить сейчас
            </button>
          )}
          <button className="btn primary" onClick={openWindow} disabled={waiting}>
            {waiting ? <Spinner /> : '🌐'} {waiting ? 'Окно открыто' : 'Открыть окно входа'}
          </button>
        </>
      }
    >
      <div className="banner info" style={{ marginBottom: 16 }}>
        <span>ℹ</span>
        <span>
          RuTracker закрыт проверкой Cloudflare, которую проходит только настоящий браузер.
          Поэтому вход выполняется на самой странице трекера, в отдельном окне.
        </span>
      </div>

      {waiting ? (
        <>
          <p style={{ marginTop: 0 }}>
            <strong>Окно трекера открыто.</strong> Введите логин и пароль прямо там.
            Если Cloudflare покажет проверку «я не робот» — пройдите её.
          </p>
          <p style={{ color: 'var(--text-dim)' }}>
            Приложение само проверяет сессию каждые 3 секунды и закроет окно, как
            только вход появится. Окно трекера можно не закрывать вручную.
          </p>

          <dl className="kv" style={{ marginTop: 16 }}>
            <dt>Сессия</dt>
            <dd>
              {probe?.hasSession ? 'найдена' : 'пока не видно'}
              {probe && !probe.hasSession && probe.hasCookie ? ' (кука уже есть)' : ''}
            </dd>
            <dt>Проверка Cloudflare</dt>
            <dd>{probe?.challenged ? 'идёт' : 'пройдена'}</dd>
            {problem && (
              <>
                <dt>Последняя ошибка</dt>
                <dd style={{ color: 'var(--warn)' }}>{problem}</dd>
              </>
            )}
          </dl>
        </>
      ) : (
        <>
          <p style={{ marginTop: 0 }}>
            Нажмите кнопку ниже — откроется страница входа RuTracker.
          </p>
          <ul style={{ color: 'var(--text-dim)', paddingLeft: 20, lineHeight: 1.7 }}>
            <li>Пароль вводится на сайте трекера, приложение его не видит и не хранит.</li>
            <li>Сессия запоминается браузерным движком и переживает перезапуск.</li>
            <li>После входа окно скрывается и используется в фоне для поиска и скачивания.</li>
          </ul>
        </>
      )}
    </Modal>
  )
}
