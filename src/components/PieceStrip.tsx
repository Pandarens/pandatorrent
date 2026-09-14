// The piece map: which parts of a download are here, drawn as a strip.
//
// A progress bar says how much; this says where. It is what shows that a
// film's first half has arrived, or that a stalled download is missing one
// stubborn stretch in the middle. Drawn on a canvas because a torrent-wide
// strip has hundreds of buckets and every open row redraws twice a second.

import { useEffect, useRef } from 'react'

export function PieceStrip({
  buckets,
  height = 8,
  title,
}: {
  /** Fill of each bucket, 0–100. */
  buckets: number[]
  height?: number
  title?: string
}) {
  const canvas = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const el = canvas.current
    if (!el) return
    const width = el.clientWidth || 200
    // Draw at device resolution so the strip stays crisp when scaled.
    const scale = window.devicePixelRatio || 1
    el.width = Math.round(width * scale)
    el.height = Math.round(height * scale)
    const ctx = el.getContext('2d')
    if (!ctx) return
    ctx.scale(scale, scale)

    ctx.fillStyle = 'rgba(255, 255, 255, 0.12)'
    ctx.fillRect(0, 0, width, height)

    const n = buckets.length
    if (n === 0) return
    const step = width / n
    // Read the accent once; the canvas cannot use CSS variables itself.
    const accent = getComputedStyle(el).getPropertyValue('--accent').trim() || '#57c26a'
    for (let i = 0; i < n; i++) {
      const fill = buckets[i] / 100
      if (fill <= 0) continue
      // A partly filled bucket is drawn paler rather than shorter: the eye
      // reads the strip as a whole, and gaps in colour are what it notices.
      ctx.globalAlpha = 0.35 + 0.65 * fill
      ctx.fillStyle = accent
      ctx.fillRect(i * step, 0, Math.ceil(step), height)
    }
    ctx.globalAlpha = 1
  }, [buckets, height])

  return <canvas ref={canvas} className="piece-strip" style={{ height }} title={title} />
}
