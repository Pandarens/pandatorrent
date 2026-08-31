// A horizontal row of cards with arrows at its edges.
//
// The rows are wider than the window and were scrollable only by wheel or
// trackpad, which leaves a mouse user dragging a thin scrollbar. Hovering an
// arrow scrolls steadily; clicking it jumps a screenful.

import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'

/** Pixels per frame while an arrow is hovered — steady, not a lurch. */
const HOVER_STEP = 9
/** How much of the visible width one click moves. */
const PAGE_FRACTION = 0.85

export function ScrollStrip({ children }: { children: ReactNode }) {
  const strip = useRef<HTMLDivElement>(null)
  const holding = useRef<number | null>(null)
  const [atStart, setAtStart] = useState(true)
  const [atEnd, setAtEnd] = useState(true)

  const measure = useCallback(() => {
    const el = strip.current
    if (!el) return
    // A row that fits entirely reports both ends at once, which is how both
    // arrows end up hidden.
    setAtStart(el.scrollLeft <= 1)
    setAtEnd(el.scrollLeft + el.clientWidth >= el.scrollWidth - 1)
  }, [])

  useEffect(() => {
    measure()
    const el = strip.current
    if (!el) return
    // Cards arrive after their pictures load, so the row's width changes
    // under us and the arrows have to be re-judged.
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [measure, children])

  const stopHold = useCallback(() => {
    if (holding.current != null) {
      window.clearInterval(holding.current)
      holding.current = null
    }
  }, [])

  // Stop scrolling if the row goes away mid-hover.
  useEffect(() => stopHold, [stopHold])

  function startHold(direction: -1 | 1) {
    stopHold()
    holding.current = window.setInterval(() => {
      strip.current?.scrollBy({ left: direction * HOVER_STEP })
    }, 16)
  }

  function page(direction: -1 | 1) {
    const el = strip.current
    if (!el) return
    el.scrollBy({ left: direction * el.clientWidth * PAGE_FRACTION, behavior: 'smooth' })
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
