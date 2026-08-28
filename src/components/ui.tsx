// Small presentational primitives shared by every view.

import { useEffect, type ReactNode } from 'react'
import { useStore } from '../lib/store'

export function Spinner() {
  return <span className="spinner" aria-label="Загрузка" />
}

export function Modal({
  title,
  icon,
  wide,
  onClose,
  children,
  footer,
}: {
  title: string
  icon?: string
  wide?: boolean
  onClose: () => void
  children: ReactNode
  footer?: ReactNode
}) {
  // Escape closes, which is what every desktop dialog does.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        // Only a click that both starts and ends on the backdrop dismisses;
        // dragging a text selection out of the dialog must not close it.
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className={wide ? 'modal wide' : 'modal'} role="dialog" aria-modal="true">
        <div className="modal-head">
          {icon && <span style={{ fontSize: 20 }}>{icon}</span>}
          <h2 className="modal-title">{title}</h2>
          <div className="spacer" />
          <button className="btn ghost sm" onClick={onClose} aria-label="Закрыть">
            ✕
          </button>
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </div>
    </div>
  )
}

export function Empty({
  icon,
  title,
  hint,
  action,
}: {
  icon: string
  title: string
  hint?: string
  action?: ReactNode
}) {
  return (
    <div className="empty">
      <div className="empty-icon">{icon}</div>
      <div style={{ fontSize: 15, color: 'var(--text-dim)', marginBottom: 6 }}>{title}</div>
      {hint && <div style={{ fontSize: 13 }}>{hint}</div>}
      {action && <div style={{ marginTop: 18 }}>{action}</div>}
    </div>
  )
}

export function ProgressBar({
  done,
  total,
  variant,
}: {
  done: number
  total: number
  variant?: 'paused' | 'done' | 'error'
}) {
  const pct = total > 0 ? Math.min(100, (done / total) * 100) : 0
  return (
    <div className="progress-track">
      <div
        className={variant ? `progress-fill ${variant}` : 'progress-fill'}
        style={{ width: `${pct}%` }}
      />
    </div>
  )
}

export function Toasts() {
  const { toasts } = useStore()
  if (toasts.length === 0) return null
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          {t.text}
        </div>
      ))}
    </div>
  )
}
