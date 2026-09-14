// The notice that the tracker window is waiting on a person.
//
// Cloudflare sometimes wants a click to confirm there is a human here. The
// window is put on screen for that, but a window can end up behind others —
// so this stays at the top of the app until the check is passed, with a
// button to bring the window back in front.

import { tracker as trackerApi } from '../lib/api'
import { useStore } from '../lib/store'

export function TrackerAttention() {
  const { attention, dismissAttention, reportError } = useStore()
  if (!attention) return null

  return (
    <div className="banner warn attention">
      <span>🛡</span>
      <span className="attention-text">{attention}</span>
      <div className="spacer" />
      <button
        className="btn primary sm"
        onClick={() => void trackerApi.showWindow().catch((e) => reportError(e, 'Окно трекера'))}
      >
        Показать окно
      </button>
      <button className="btn ghost sm" onClick={dismissAttention} title="Убрать уведомление">
        ✕
      </button>
    </div>
  )
}
