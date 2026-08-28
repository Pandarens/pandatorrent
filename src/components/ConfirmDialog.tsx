// In-app confirmation, used instead of `window.confirm`.
//
// Tauri's webview blocks the browser's native dialogs, so `window.confirm`
// never returns a decision and the calling handler quietly stops — which is why
// none of the delete buttons appeared to do anything. Rendering the dialog
// ourselves also allows more than two answers, which "delete, and the files
// too?" genuinely needs.

import type { ReactNode } from 'react'

import { Modal } from './ui'

export interface ConfirmChoice<T> {
  label: string
  value: T
  /** `danger` for destructive answers, `primary` for the expected one. */
  kind?: 'danger' | 'primary' | 'ghost'
}

export function ConfirmDialog<T>({
  title,
  icon,
  message,
  choices,
  onPick,
  onCancel,
}: {
  title: string
  icon?: string
  message: ReactNode
  choices: Array<ConfirmChoice<T>>
  onPick: (value: T) => void
  onCancel: () => void
}) {
  return (
    <Modal
      title={title}
      icon={icon}
      onClose={onCancel}
      footer={
        <>
          <button className="btn ghost" onClick={onCancel}>
            Отмена
          </button>
          <div className="spacer" />
          {choices.map((choice) => (
            <button
              key={choice.label}
              className={`btn ${choice.kind ?? ''}`.trim()}
              onClick={() => onPick(choice.value)}
            >
              {choice.label}
            </button>
          ))}
        </>
      }
    >
      {typeof message === 'string' ? <p style={{ margin: 0 }}>{message}</p> : message}
    </Modal>
  )
}
