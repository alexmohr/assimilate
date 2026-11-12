// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { filesLabel, formatBytes, formatDuration, listSeparator } from './format'

describe('formatBytes', () => {
  it('returns "0 B" for zero', () => {
    expect(formatBytes(0)).toBe('0 B')
  })

  it('returns "0 B" for null or undefined', () => {
    expect(formatBytes(null as unknown as number)).toBe('0 B')
    expect(formatBytes(undefined as unknown as number)).toBe('0 B')
  })

  it('formats bytes correctly', () => {
    expect(formatBytes(512)).toBe('512.0 B')
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(1048576)).toBe('1.0 MB')
    expect(formatBytes(1073741824)).toBe('1.0 GB')
  })

  it('formats fractional sizes', () => {
    expect(formatBytes(1536)).toBe('1.5 KB')
  })
})

describe('formatDuration', () => {
  it('formats seconds only', () => {
    expect(formatDuration(45)).toBe('45s')
  })

  it('formats minutes and seconds', () => {
    expect(formatDuration(125)).toBe('2m 5s')
  })

  it('formats hours and minutes', () => {
    expect(formatDuration(3661)).toBe('1h 1m')
  })
})

describe('filesLabel', () => {
  it('counts files in words', () => {
    expect(filesLabel(1)).toBe('1 file')
    expect(filesLabel(1284)).toBe(`${(1284).toLocaleString()} files`)
  })
})

describe('listSeparator', () => {
  const sentence = (items: string[], end?: string): string =>
    items.map((item, i) => item + listSeparator(i, items.length, end)).join('')

  it('joins items with commas and " and " before the last', () => {
    expect(sentence(['a'])).toBe('a')
    expect(sentence(['a', 'b'])).toBe('a and b')
    expect(sentence(['a', 'b', 'c'])).toBe('a, b and c')
  })

  it('ends with the given text after the last item', () => {
    expect(sentence(['a'], '.')).toBe('a.')
    expect(sentence(['a', 'b', 'c'], '.')).toBe('a, b and c.')
  })
})
