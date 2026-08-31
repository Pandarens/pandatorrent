// A horizontal row of cards with arrows at its edges.
//
// The rows are wider than the window and were scrollable only by wheel or
// trackpad, which leaves a mouse user dragging a thin scrollbar. Hovering an
// arrow scrolls steadily; clicking it moves a screenful.

import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'

/** Top speed of a hover-scroll, in pixels per millisecond. */
const MAX_SPEED = 0.85
/** How long it takes to reach that speed. Starting at full tilt lurches. */
const RAMP_MS = 260
/** How much of the visible width one click moves. */
const PAGE_FRACTION = 0.85

export function ScrollStrip({ children }: { children: ReactNode }) {
  const strip = useRef<HTMLDivElement>(null)
  const [atStart, setAtStart] = useState(true)
  const [atEnd, setAtEnd] = useState(true)

  const frame = useRef<number | null>(null)
  const direction = useRef(0)
  const startedAt = useRef(0)
  const lastFrame = useRef(0)

  const measure = useCallback(() => {
    const el = strip.current
    if (!el) return
    // A row that fits entirely reports both ends at once, which is how both
    // arrows end up hidden.
    setAtStart(el.scrollLeft <= 1)
    setAtEnd(el.scrollLeft + el.clientWidth >= el.scrollWidth - 1)
  }, [])

  useEffect(() => {
    const el = strip.current
    if (!el) return
    measure()

    // Three things change whether there is anything to scroll to, and all
    // three have to be watched. A ResizeObserver on the row alone was not
    // enough: the row keeps its own width while its *contents* grow, so the
    // arrows stayed hidden on the very rows that needed them.
    const onResize = new ResizeObserver(measure)
    onResize.observe(el)

    const onChildren = new MutationObserver(measure)
    onChildren.observe(el, { childList: true, subtree: true })

    // Cover art settles the card widths, and `load` does not bubble — so it
    // has to be caught on the way down.
    el.addEventListener('load', measure, true)

    // One more pass after layout has actually happened.
    const settle = window.setTimeout(measure, 250)

    return () => {
      onResize.disconnect()
      onChildren.disconnect()
      el.removeEventListener('load', measure, true)
      window.clearTimeout(settle)
    }
  }, [measure])

  const stopHold = useCallback(() => {
    direction.current = 0
    if (frame.current != null) {
      cancelAnimationFrame(frame.current)
      frame.current = null
    }
  }, [])

  useEffect(() => stopHold, [stopHold])

  const step = useCallback(
    (now: number) => {
      if (direction.current === 0) return
      const el = strip.current
      if (!el) return

      const delta = lastFrame.current === 0 ? 16 : Math.min(50, now - lastFrame.current)
      lastFrame.current = now

      // Ease up to speed rather than jumping to it.
      const ramp = Math.min(1, (now - startedAt.current) / RAMP_MS)
      el.scrollLeft += direction.current * MAX_SPEED * ramp * delta

      measure()
      frame.current = requestAnimationFrame(step)
    },
    [measure],
  )

  function startHold(towards: -1 | 1) {
    stopHold()
    direction.current = towards
    startedAt.current = performance.now()
    lastFrame.current = 0
    frame.current = requestAnimationFrame(step)
  }

  function page(towards: -1 | 1) {
    const el = strip.current
    if (!el) return
    stopHold()
    el.scrollBy({ left: towards * el.clientWidth * PAGE_FRACTION, behavior: 'smooth' })
  }

  return (
    <div className="strip-scroll">
      {!atStart && (
        <button
          className="strip-arrow left"
          title="Прокрутить назад"
          aria-label="Прокрутить назад"
          onMouseEnter={() => startHold(-1)}
          onMouseLeave={stopHold}
          onClick={() => page(-1)}
        >
          ‹
        </button>
      )}

      <div className="strip" ref={strip} onScroll={measure}>
        {children}
      </div>

      {!atEnd && (
        <button
          className="strip-arrow right"
          title="Прокрутить вперёд"
          aria-label="Прокрутить вперёд"
          onMouseEnter={() => startHold(1)}
          onMouseLeave={stopHold}
          onClick={() => page(1)}
        >
          ›
        </button>
      )}
    </div>
  )
}
