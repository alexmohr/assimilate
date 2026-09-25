// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { reactive } from 'vue'

import { TEMPLATE_PLACEHOLDERS } from '../utils/notificationTemplate'
import {
  maxHookCommandTimeoutSeconds,
  notificationTemplatePlaceholderKeys,
  parseFileChangePatterns,
  serializeFileChangePatterns,
  validateCron,
} from './domain'

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

describe('validateCron', () => {
  it('accepts weekday and month names the scheduler understands', () => {
    expect(validateCron('0 2 * * MON-FRI')).toBeUndefined()
    expect(validateCron('0 2 1 JAN *')).toBeUndefined()
  })

  it('returns the server error message for an out-of-range field', () => {
    expect(validateCron('60 2 * * *')).toMatch(/^invalid cron expression: /)
  })

  it('rejects an expression with too few fields', () => {
    expect(validateCron('0 2 * *')).toBeDefined()
  })
})

describe('notification template placeholders', () => {
  it('offers exactly the keys the Rust renderer substitutes, in the same order', () => {
    expect(TEMPLATE_PLACEHOLDERS.map((p) => p.key)).toEqual(notificationTemplatePlaceholderKeys())
  })
})

describe('maxHookCommandTimeoutSeconds', () => {
  it('is the 24-hour server limit', () => {
    expect(maxHookCommandTimeoutSeconds()).toBe(86_400)
  })
})
