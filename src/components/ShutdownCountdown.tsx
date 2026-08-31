// The warning before the computer turns itself off.
//
// Shutting a machine down is not something to do quietly, so it is announced,
// counted down, and called off by anything at all — a click, a key, the mouse
// moving. Doing nothing is the only way it goes through.

import { useEffect, useRef, useState } from 'react'

import { power as powerApi } from '../lib/api'

export function ShutdownCountdown({
  seconds,
  reason,
  onCancel,
}: {
  seconds: number
  /** What finished, so the message says why this is happening. */
  reason: string
  onCancel: () => void
}) {
  const [left, setLeft] = useState(seconds)
  const fired = useRef(false)

  useEffect(() => {
    const tick = window.setInterval(() => setLeft((n) => n - 1), 1000)
    return () => window.clearInterval(tick)
  }, [])

  useEffect(() => {
    if (left > 0 || fired.current) return
    fired.current = true
    void powerApi.shutdown().catch(() => onCancel())
  }, [left, onCancel])

  // Any sign of life calls it off. Somebody at the keyboard did not mean this.
  useEffect(() => {
    const stop = () => {
      if (!fired.current) onCancel()
    }
    window.addEventListener('keydown', stop)
    window.addEventListener('mousedown', stop)
    return () => {
      window.removeEventListener('keydown', stop)
      window.removeEventListener('mousedown', stop)
    }
  }, [onCancel])

  return (
    <div className="shutdown-veil">
      <div className="shutdown-box">
        <div className="shutdown-title">Выключение компьютера</div>
        <div className="shutdown-reason">{reason}</div>
        <div className="shutdown-count">{Math.max(0, left)}</div>
        <div className="shutdown-hint">
          Нажмите любую клавишу или кнопку мыши, чтобы отменить
        </div>
        <button className="btn primary" onClick={onCancel}>
          Не выключать
        </button>
      </div>
    </div>
  )
}
