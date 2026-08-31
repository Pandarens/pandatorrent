// The player interface, drawn over the video.
//
// This page runs in its own frameless window with a transparent background;
// mpv renders the picture into the same window, stacked underneath. Every
// control talks to mpv through the app's commands, so nothing here depends on
// mpv's own on-screen controller — which is why it can be in Russian.
//
// Smoothness note: the state is polled, but a poll must never yank a control
// out from under the user. Anything the user just touched keeps its local value
// for a moment before the reported one is trusted again; without that the
// volume slider jumps back on every tick.

import { useCallback, useEffect, useRef, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { player as playerApi, settings as settingsApi } from './lib/api'
import { formatBytes, formatSpeed } from './lib/format'
import type { AppConfig, Playback } from './lib/types'

/** Fast enough for a smooth progress bar, cheap enough to leave running. */
const POLL_MS = 400
// A picture that has not moved for this long is waiting on data. Whether that
// is a fault depends on whether data is still arriving — see below.
const BUFFER_MS = 2500
// Frozen this long with nothing arriving at all: the stream really has stopped.
const STALL_MS = 20000
/** How long the mouse must be still before the controls fade away. */
const IDLE_MS = 2600
/** How long a control the user touched keeps its own value. */
const HOLD_MS = 900
/** Volume step for one wheel notch. */
const WHEEL_STEP = 5

const appWindow = getCurrentWindow()

function clock(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return '--:--'
  const total = Math.floor(seconds)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const mm = String(m).padStart(2, '0')
  const ss = String(s).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`
}

export function PlayerWindow() {
  const [state, setState] = useState<Playback | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [active, setActive] = useState(true)
  const [hovering, setHovering] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [frozenFor, setFrozenFor] = useState(0)
  const progressRef = useRef({ position: -1, since: 0 })
  const [onTop, setOnTop] = useState(false)
  const [fullscreen, setFullscreen] = useState(false)

  // Values the user is driving right now. `null` means "trust the player".
  const [volume, setVolume] = useState<number | null>(null)
  const [scrub, setScrub] = useState<number | null>(null)
  const [paused, setPaused] = useState<boolean | null>(null)

  const heldUntil = useRef<{ volume: number; paused: number }>({ volume: 0, paused: 0 })
  const idleTimer = useRef<number | null>(null)
  const volumeSend = useRef<number | null>(null)

  const wake = useCallback(() => {
    setActive(true)
    if (idleTimer.current) window.clearTimeout(idleTimer.current)
    idleTimer.current = window.setTimeout(() => setActive(false), IDLE_MS)
  }, [])

  useEffect(() => {
    wake()
    window.addEventListener('mousemove', wake)
    window.addEventListener('keydown', wake)
    return () => {
      window.removeEventListener('mousemove', wake)
      window.removeEventListener('keydown', wake)
      if (idleTimer.current) window.clearTimeout(idleTimer.current)
    }
  }, [wake])

  useEffect(() => {
    let cancelled = false
    const tick = async () => {
      try {
        const next = await playerApi.playback()
        if (cancelled) return
        setState(next)
        setError(null)
        const now = Date.now()

        // Playback is judged by the clock moving, not by mpv reporting a
        // fault: when a torrent stream dies it simply stops delivering, and
        // nothing else says so.
        const running = next && !next.paused && (next.duration ?? 0) > 0
        const position = next?.position ?? 0
        if (!running || Math.abs(position - progressRef.current.position) > 0.25) {
          progressRef.current = { position, since: now }
          setFrozenFor(0)
        } else {
          setFrozenFor(now - progressRef.current.since)
        }

        // Adopt the reported values only once the user has let go.
        if (next && now > heldUntil.current.volume) setVolume(null)
        if (next && now > heldUntil.current.paused) setPaused(null)
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e))
      }
    }
    void tick()
    const timer = window.setInterval(tick, POLL_MS)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [])

  useEffect(() => {
    settingsApi.get().then(setConfig).catch(() => setConfig(null))
    appWindow.isFullscreen().then(setFullscreen).catch(() => {})
  }, [])

  /** Sets the volume optimistically, rate-limiting what reaches the player. */
  const changeVolume = useCallback((next: number) => {
    const clamped = Math.max(0, Math.min(150, Math.round(next)))
    setVolume(clamped)
    heldUntil.current.volume = Date.now() + HOLD_MS
    // One in-flight call at a time: a slider drag fires far more events than
    // the player needs, and queuing them all is what made it feel sticky.
    if (volumeSend.current) return
    volumeSend.current = window.setTimeout(() => {
      volumeSend.current = null
      void playerApi.setVolume(clamped).catch(() => {})
    }, 40)
  }, [])

  const togglePause = useCallback(() => {
    setPaused((current) => {
      const next = !(current ?? state?.paused ?? false)
      heldUntil.current.paused = Date.now() + HOLD_MS
      void playerApi.togglePause().catch(() => {})
      return next
    })
  }, [state?.paused])

  const toggleFullscreen = useCallback(async () => {
    const next = !fullscreen
    setFullscreen(next)
    await appWindow.setFullscreen(next).catch(() => {})
  }, [fullscreen])

  // Keyboard shortcuts, the ones people expect from a player.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      switch (e.key) {
        case ' ':
        case 'k':
          e.preventDefault()
          togglePause()
          break
        case 'ArrowRight':
          void playerApi.seekBy(e.shiftKey ? 60 : 10)
          break
        case 'ArrowLeft':
          void playerApi.seekBy(e.shiftKey ? -60 : -10)
          break
        case 'ArrowUp':
          e.preventDefault()
          changeVolume((volume ?? state?.volume ?? 100) + WHEEL_STEP)
          break
        case 'ArrowDown':
          e.preventDefault()
          changeVolume((volume ?? state?.volume ?? 100) - WHEEL_STEP)
          break
        case 'f':
          void toggleFullscreen()
          break
        case 'm':
          void playerApi.toggleMute()
          break
        case 'Escape':
          if (fullscreen) void toggleFullscreen()
          else void appWindow.close()
          break
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [togglePause, toggleFullscreen, changeVolume, volume, state?.volume, fullscreen])

  const duration = state?.duration ?? 0
  // Until mpv knows the length there is nothing to show but a blank window, so
  // the placeholder stands in for it rather than leaving an empty rectangle.
  // Seeking to where we already are makes the player re-request the stream,
  // which is exactly what is needed after it has gone quiet.
  const retry = useCallback(async () => {
    const at = Math.max(0, (state?.position ?? 0) - 1)
    setFrozenFor(0)
    progressRef.current = { position: -1, since: Date.now() }
    await playerApi.seekTo(at).catch(() => {})
  }, [state])

  // Standing still while pieces are still arriving is buffering, and saying
  // "the stream stopped" there is simply wrong — it is the normal cost of
  // jumping to a part of the film that has not been downloaded yet.
  const downloading = (state?.downloadSpeedBps ?? 0) > 0
  const buffering = frozenFor > BUFFER_MS && downloading
  const stalled = frozenFor > STALL_MS && !downloading

  const loading = !state || duration <= 0
  // mpv reports the stream URL as its media title, so the readable name comes
  // from the app instead; the episode file name is added only when there is a
  // season to tell apart.
  const heading = state?.title ?? 'Загрузка…'
  const position = scrub ?? state?.position ?? 0
  const shownVolume = volume ?? Math.round(state?.volume ?? 100)
  const isPaused = paused ?? state?.paused ?? false
  const episodes = state?.playlistCount ?? 0
  const episode = (state?.playlistPos ?? 0) + 1
  // Only worth naming the file when there is a season to tell apart.
  const subheading = episodes > 1 ? (state?.episode ?? null) : null
  const progress = duration > 0 ? (position / duration) * 100 : 0
  const normalize = config?.player.audioNormalize ?? 'off'
  const speed = state?.downloadSpeedBps ?? null
  const fileDone = state?.fileDownloaded ?? null
  const fileTotal = state?.fileTotal ?? null

  async function setNormalize(value: string) {
    if (!config) return
    const next = { ...config, player: { ...config.player, audioNormalize: value } }
    setConfig(next)
    // Saving applies it to the running player, so the change is audible at once.
    await settingsApi.set(next).catch(() => {})
  }

  // Hiding the controls out from under a cursor that is resting on a button is
  // the one case where the idle timer is wrong; the same goes for a paused
  // film, where the controls are exactly what the viewer is looking at.
  const visible = active || hovering || isPaused

  return (
    <div
      className={visible ? 'pw' : 'pw idle'}
      onDoubleClick={() => void toggleFullscreen()}
      // The wheel is the natural way to change volume over a video.
      onWheel={(e) => {
        changeVolume(shownVolume + (e.deltaY < 0 ? WHEEL_STEP : -WHEEL_STEP))
        wake()
      }}
    >
      {/* Frameless window: these strips ask Windows to move and resize it. */}
      {!fullscreen && (
        <>
          <div className="pw-resize n" onMouseDown={() => appWindow.startResizeDragging('North')} />
          <div className="pw-resize s" onMouseDown={() => appWindow.startResizeDragging('South')} />
          <div className="pw-resize w" onMouseDown={() => appWindow.startResizeDragging('West')} />
          <div className="pw-resize e" onMouseDown={() => appWindow.startResizeDragging('East')} />
          <div
            className="pw-resize se"
            onMouseDown={() => appWindow.startResizeDragging('SouthEast')}
          />
          <div
            className="pw-resize sw"
            onMouseDown={() => appWindow.startResizeDragging('SouthWest')}
          />
        </>
      )}

      <div
        className="pw-top"
        onMouseEnter={() => setHovering(true)}
        onMouseLeave={() => setHovering(false)}
      >
        <div
          className="pw-drag"
          onMouseDown={(e) => {
            // Only a plain drag on empty space moves the window.
            if (e.button === 0 && e.target === e.currentTarget) void appWindow.startDragging()
          }}
        >
          <span className="pw-name" title={heading}>
            {heading}
          </span>
          {subheading && (
            <span className="pw-episode" title={subheading}>
              {subheading}
            </span>
          )}
          {episodes > 1 && (
            <span className="pw-episode-count">
              Серия {episode} из {episodes}
            </span>
          )}
          {/* Speed matters while streaming, so it sits with the title and is
              visible exactly when the rest of the interface is. */}
          {speed != null && speed > 0 && (
            <span className="pw-speed" title="Скорость загрузки">
              ↓ {formatSpeed(speed)}
              {state?.peers ? ` · ${state.peers} пир.` : ''}
            </span>
          )}
          {fileDone != null && fileTotal ? (
            <span className="pw-episode" title="Загружено из этой серии">
              {Math.min(100, Math.round((fileDone / fileTotal) * 100))}% ·{' '}
              {formatBytes(fileDone)}
            </span>
          ) : null}
          {state?.nextReady && episodes > 1 && (
            <span className="pw-episode ready" title="Следующая серия уже скачана">
              Следующая готова
            </span>
          )}
        </div>

        <button
          className={onTop ? 'pw-btn on' : 'pw-btn'}
          title="Поверх всех окон"
          onClick={async () => {
            const next = !onTop
            setOnTop(next)
            await appWindow.setAlwaysOnTop(next).catch(() => {})
          }}
        >
          📌
        </button>
        <button className="pw-btn" title="Свернуть" onClick={() => void appWindow.minimize()}>
          −
        </button>
        <button className="pw-btn danger" title="Закрыть (Esc)" onClick={() => void appWindow.close()}>
          ✕
        </button>
      </div>

      {/* Solid backing while the picture is missing: the window is transparent
          so that video shows through the interface, which without this leaves
          the desktop visible until the first frame arrives. */}
      {loading && <div className="pw-backdrop" />}

      {loading && !error && (
        <div className="pw-loading">
          <div className="pw-loading-art">
            <span />
            <span />
            <span />
            <span />
          </div>
          <div className="pw-loading-title">{heading}</div>
          {subheading && <div className="pw-loading-episode">{subheading}</div>}
          <div className="pw-loading-hint">
            Фильм начнёт играть, как только скачается начало — обычно несколько секунд
          </div>
        </div>
      )}
      {error && <div className="pw-center pw-error">{error}</div>}

      {buffering && !error && !loading && (
        <div className="pw-center pw-stall buffering">
          <div className="pw-loading-art">
            <span />
            <span />
            <span />
            <span />
          </div>
          <div className="pw-stall-title">Догружаем эту часть</div>
          <div className="pw-stall-hint">
            {formatSpeed(state?.downloadSpeedBps ?? 0)}
            {state?.peers ? ` · ${state.peers} раздающих` : ''}
          </div>
        </div>
      )}

      {stalled && !error && (
        <div className="pw-center pw-stall">
          <div className="pw-stall-title">Поток остановился</div>
          <div className="pw-stall-hint">
            Данные перестали приходить — возможно, нет раздающих или пропала сеть.
          </div>
          <button className="btn primary" onClick={() => void retry()}>
            Повторить
          </button>
        </div>
      )}

      <div
        className="pw-bottom"
        onMouseEnter={() => setHovering(true)}
        onMouseLeave={() => setHovering(false)}
      >
        <div className="pw-seek">
          <span className="pw-time">{clock(position)}</span>
          <div className="pw-bar">
            <div className="pw-bar-fill" style={{ width: `${progress}%` }} />
            <input
              type="range"
              min={0}
              max={Math.max(1, Math.floor(duration))}
              value={Math.floor(position)}
              disabled={duration <= 0}
              onChange={(e) => setScrub(Number(e.target.value))}
              onMouseUp={() => {
                if (scrub != null) {
                  void playerApi.seekTo(scrub)
                  setScrub(null)
                }
              }}
            />
          </div>
          <span className="pw-time">{clock(duration)}</span>
        </div>

        <div className="pw-row">
          {episodes > 1 && (
            <button
              className="pw-btn"
              title="Предыдущая серия"
              onClick={() => void playerApi.prevEpisode()}
            >
              ⏮
            </button>
          )}
          <button
            className="pw-btn"
            title="Назад 10 секунд (←)"
            onClick={() => void playerApi.seekBy(-10)}
          >
            ⏪
          </button>
          <button
            className="pw-btn pw-play"
            title={isPaused ? 'Продолжить (пробел)' : 'Пауза (пробел)'}
            onClick={togglePause}
          >
            {isPaused ? '▶' : '⏸'}
          </button>
          <button
            className="pw-btn"
            title="Вперёд 30 секунд (→)"
            onClick={() => void playerApi.seekBy(30)}
          >
            ⏩
          </button>
          {episodes > 1 && (
            <button
              className="pw-btn"
              title="Следующая серия"
              onClick={() => void playerApi.nextEpisode()}
            >
              ⏭
            </button>
          )}

          <div className="pw-grow" />

          <button
            className="pw-btn"
            title={state?.muted ? 'Включить звук (M)' : 'Выключить звук (M)'}
            onClick={() => void playerApi.toggleMute()}
          >
            {state?.muted ? '🔇' : '🔊'}
          </button>
          <input
            className="pw-volume"
            type="range"
            min={0}
            max={150}
            value={shownVolume}
            title={`Громкость ${shownVolume}%`}
            onChange={(e) => changeVolume(Number(e.target.value))}
          />
          <span className="pw-time pw-vol-value">{shownVolume}%</span>

          <select
            className="pw-select"
            value={normalize}
            title="Выравнивание громкости"
            onChange={(e) => void setNormalize(e.target.value)}
          >
            <option value="off">Звук: как есть</option>
            <option value="dynaudnorm">Звук: выровнять</option>
            <option value="loudnorm">Звук: по стандарту</option>
          </select>

          <button
            className="pw-btn"
            title={fullscreen ? 'Выйти из полноэкранного (F)' : 'Во весь экран (F)'}
            onClick={() => void toggleFullscreen()}
          >
            {fullscreen ? '🗗' : '⛶'}
          </button>
        </div>
      </div>
    </div>
  )
}
