// Preview artwork for search results.
//
// A preview costs a whole topic-page load through the browser transport, so
// three things keep the grid from hammering the tracker:
//
//   * only cards the user actually scrolls to ask for one;
//   * at most CONCURRENCY requests are in flight at a time;
//   * answers — including "this topic has no image" — are remembered, here for
//     the session and in SQLite across restarts.

import { useEffect, useRef, useState } from 'react'
import { tracker } from './api'

/** Requests in flight. The transport is a single browser window; flooding it
 *  just makes every card slower. */
const CONCURRENCY = 2

type Preview = string | null

const cache = new Map<number, Preview>()
const inflight = new Map<number, Promise<Preview>>()
const waiting: Array<() => void> = []
let active = 0

function acquire(): Promise<void> {
  if (active < CONCURRENCY) {
    active++
    return Promise.resolve()
  }
  return new Promise((resolve) => waiting.push(resolve))
}

function release() {
  const next = waiting.shift()
  if (next) {
    next()
  } else {
    active--
  }
}

export function getPreview(topicId: number): Promise<Preview> {
  const cached = cache.get(topicId)
  if (cached !== undefined) return Promise.resolve(cached)

  const existing = inflight.get(topicId)
  if (existing) return existing

  const task = acquire()
    .then(() => tracker.topicPreview(topicId))
    .then((url) => {
      cache.set(topicId, url ?? null)
      return url ?? null
    })
    .catch(() => {
      // Left out of the cache so a transient failure can be retried when the
      // card scrolls back into view.
      return null
    })
    .finally(() => {
      inflight.delete(topicId)
      release()
    })

  inflight.set(topicId, task)
  return task
}

/**
 * Loads a preview once the element is on screen.
 *
 * Returns the image URL, whether a request is running, and the ref to attach
 * to the card.
 */
export function usePreview(topicId: number) {
  const [url, setUrl] = useState<Preview>(() => cache.get(topicId) ?? null)
  const [loading, setLoading] = useState(false)
  const ref = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const known = cache.get(topicId)
    if (known !== undefined) {
      setUrl(known)
      return
    }
    setUrl(null)

    const node = ref.current
    if (!node) return

    let cancelled = false
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return
        observer.disconnect()
        setLoading(true)
        getPreview(topicId)
          .then((found) => {
            if (!cancelled) setUrl(found)
          })
          .finally(() => {
            if (!cancelled) setLoading(false)
          })
      },
      // Start a little before the card is visible so scrolling feels filled-in.
      { rootMargin: '300px' },
    )
    observer.observe(node)

    return () => {
      cancelled = true
      observer.disconnect()
    }
  }, [topicId])

  return { url, loading, ref }
}
