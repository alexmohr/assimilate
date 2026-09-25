// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { reactive } from 'vue'

import { parseFileChangePatterns, serializeFileChangePatterns } from './domain'

describe('domain WebAssembly module', () => {
  it('returns plain objects, not wrapper classes', () => {
    const [row] = parseFileChangePatterns('*/tmp* ignore')
    expect(Object.getPrototypeOf(row)).toBe(Object.prototype)
    expect(Object.keys(row ?? {}).sort()).toEqual(['action', 'path'])
  })

  it('serializes rows wrapped in a Vue reactive proxy', () => {
    const rows = reactive(parseFileChangePatterns('*/tmp* ignore\n*/etc*'))
    expect(serializeFileChangePatterns(rows)).toBe('*/tmp* ignore\n*/etc*')
  })

  it('throws on an unknown action instead of guessing', () => {
    const rows = JSON.parse('[{"path":"*/tmp*","action":"explode"}]')
    expect(() => serializeFileChangePatterns(rows)).toThrow()
  })

  it('preserves non-ASCII paths across the boundary', () => {
    const raw = '*/Dokumente/Übersicht* fatal'
    expect(serializeFileChangePatterns(parseFileChangePatterns(raw))).toBe(raw)
  })
})
