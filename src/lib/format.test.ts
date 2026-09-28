// The display helpers, pinned. Every number the interface shows goes through
// these, so a wrong unit or a broken threshold would be everywhere at once.

import { describe, expect, it } from 'vitest'

import {
  formatBytes,
  formatEta,
  formatSpeed,
  progressPercent,
  sizeDelta,
  stateLabel,
} from './format'

describe('formatBytes', () => {
  it('picks the unit and keeps whole bytes whole', () => {
    expect(formatBytes(0)).toBe('0 Б')
    expect(formatBytes(512)).toBe('512 Б')
    expect(formatBytes(1536)).toBe('1.5 КБ')
    expect(formatBytes(2.5 * 1024 * 1024)).toBe('2.5 МБ')
    expect(formatBytes(3 * 1024 ** 3)).toBe('3.0 ГБ')
  })

  it('shows a dash for nothing sensible', () => {
    expect(formatBytes(null)).toBe('—')
    expect(formatBytes(undefined)).toBe('—')
    expect(formatBytes(-1)).toBe('—')
    expect(formatBytes(Number.NaN)).toBe('—')
  })
})

describe('formatSpeed', () => {
  it('is a dash at rest and a rate otherwise', () => {
    expect(formatSpeed(0)).toBe('—')
    expect(formatSpeed(2048)).toBe('2.0 КБ/с')
  })
})

describe('formatEta', () => {
  it('uses the two largest units that matter', () => {
    expect(formatEta(45)).toBe('45 с')
    expect(formatEta(125)).toBe('2 мин 5 с')
    expect(formatEta(5 * 3600 + 7 * 60)).toBe('5 ч 7 мин')
    expect(formatEta(2 * 86400 + 3 * 3600)).toBe('2 д 3 ч')
  })

  it('gives up on the unknowable', () => {
    expect(formatEta(null)).toBe('—')
    expect(formatEta(0)).toBe('—')
    expect(formatEta(31 * 86400)).toBe('∞')
  })
})

describe('progressPercent', () => {
  it('is clamped and survives an empty total', () => {
    expect(progressPercent(50, 200)).toBe(25)
    expect(progressPercent(5, 0)).toBe(0)
    expect(progressPercent(300, 200)).toBe(100)
  })
})

describe('stateLabel', () => {
  it('tells seeding, downloading, checking and pausing apart', () => {
    expect(stateLabel('live', false, false)).toBe('Загрузка')
    expect(stateLabel('live', true, false)).toBe('Раздаётся')
    expect(stateLabel('initializing', false, false)).toBe('Проверка файлов')
    expect(stateLabel('paused', false, false)).toBe('Пауза')
    expect(stateLabel('paused', true, false)).toBe('Остановлен')
  })

  it('lets an error win over everything', () => {
    expect(stateLabel('live', true, true)).toBe('Ошибка')
  })
})

describe('sizeDelta', () => {
  it('is signed, and honest about no change', () => {
    expect(sizeDelta(1024, 2048)).toBe('+1.0 КБ')
    expect(sizeDelta(2048, 1024)).toBe('−1.0 КБ')
    expect(sizeDelta(100, 100)).toBe('размер не изменился')
    expect(sizeDelta(null, 5)).toBe('—')
  })
})
