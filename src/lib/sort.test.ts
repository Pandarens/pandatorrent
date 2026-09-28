// The download list order. Sorting by progress or speed reads through the live
// stats, and a row without stats must sort sensibly rather than crash.

import { describe, expect, it } from 'vitest'

import { compareRows, type SortableRow } from './sort'

function row(
  name: string,
  addedAt: number,
  totalBytes: number,
  live?: { done: number; speed: number },
): SortableRow {
  return {
    t: { name, addedAt, totalBytes },
    p: live
      ? { totalBytes, progressBytes: live.done, downloadSpeedBps: live.speed }
      : null,
  }
}

const rows = [
  row('Беталь', 20, 1000, { done: 500, speed: 10 }),
  row('Альфа', 30, 4000, { done: 4000, speed: 0 }),
  row('Гамма', 10, 2000),
]

const order = (key: Parameters<typeof compareRows>[0], desc: boolean) =>
  [...rows].sort(compareRows(key, desc)).map((r) => r.t.name)

describe('compareRows', () => {
  it('orders by when it was added, newest first by default', () => {
    expect(order('added', true)).toEqual(['Альфа', 'Беталь', 'Гамма'])
    expect(order('added', false)).toEqual(['Гамма', 'Беталь', 'Альфа'])
  })

  it('orders names the Russian way', () => {
    expect(order('name', false)).toEqual(['Альфа', 'Беталь', 'Гамма'])
  })

  it('orders by size using the live total when there is one', () => {
    expect(order('size', true)).toEqual(['Альфа', 'Гамма', 'Беталь'])
  })

  it('treats a row without stats as zero progress and zero speed', () => {
    expect(order('progress', true)).toEqual(['Альфа', 'Беталь', 'Гамма'])
    expect(order('speed', true)[0]).toBe('Беталь')
  })
})
