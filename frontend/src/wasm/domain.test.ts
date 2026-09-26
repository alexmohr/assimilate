// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { reactive } from 'vue'

import { TEMPLATE_PLACEHOLDERS } from '../utils/notificationTemplate'
import {
  maxHookCommandTimeoutSeconds,
  nextCronRuns,
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

describe('nextCronRuns', () => {
  const iso = (dates: Date[]): string[] => dates.map((d) => d.toISOString())

  it('lists consecutive runs in UTC', () => {
    const runs = nextCronRuns('0 */6 * * *', new Date('2026-01-01T10:00:00Z'), 'UTC', 3)
    expect(iso(runs)).toEqual([
      '2026-01-01T12:00:00.000Z',
      '2026-01-01T18:00:00.000Z',
      '2026-01-02T00:00:00.000Z',
    ])
  })

  it('rolls a run in a spring-forward gap to the end of the gap, like the scheduler', () => {
    const runs = nextCronRuns('30 2 * * *', new Date('2026-03-28T00:00:00Z'), 'Europe/Berlin', 3)
    expect(iso(runs)).toEqual([
      '2026-03-28T01:30:00.000Z',
      '2026-03-29T01:00:00.000Z',
      '2026-03-30T00:30:00.000Z',
    ])
  })

  it('runs a repeated fall-back time once, at its first occurrence', () => {
    const runs = nextCronRuns('30 2 * * *', new Date('2026-10-24T23:00:00Z'), 'Europe/Berlin', 2)
    expect(iso(runs)).toEqual(['2026-10-25T00:30:00.000Z', '2026-10-26T01:30:00.000Z'])
  })

  it('follows a non-European zone through its own transition', () => {
    const runs = nextCronRuns('15 2 * * *', new Date('2026-03-08T05:00:00Z'), 'America/New_York', 1)
    expect(iso(runs)).toEqual(['2026-03-08T07:00:00.000Z'])
  })

  it('rejects an unknown timezone', () => {
    expect(() => nextCronRuns('0 2 * * *', new Date(), 'Not/A/Zone', 3)).toThrow(RangeError)
  })

  it('rejects an invalid expression', () => {
    expect(() => nextCronRuns('nope', new Date(), 'UTC', 3)).toThrow()
  })
})
