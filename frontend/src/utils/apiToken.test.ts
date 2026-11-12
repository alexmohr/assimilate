// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  TOKEN_EXPIRY_CHOICES,
  formatNetworkList,
  parseNetworkList,
  tokenExpiresAt,
  tokenExpiryState,
  tokenScopeLabel,
  tokenScopeTone,
} from './apiToken'

const DAY = 86_400_000

describe('TOKEN_EXPIRY_CHOICES', () => {
  it('offers never, the preset lengths and a picked date, in that order', () => {
    expect(TOKEN_EXPIRY_CHOICES.map((c) => [c.value, c.label])).toEqual([
      ['never', 'Never'],
      ['30d', '30 days'],
      ['90d', '90 days'],
      ['365d', '1 year'],
      ['date', 'On a date'],
    ])
  })

  it('defaults to Never, the first choice', () => {
    expect(TOKEN_EXPIRY_CHOICES[0].value).toBe('never')
  })
})

describe('tokenExpiresAt', () => {
  const NOW = new Date('2026-06-01T12:00:00Z')

  it('sends no expiry for Never, whatever date is lying around', () => {
    expect(tokenExpiresAt('never', '', NOW)).toBeNull()
    expect(tokenExpiresAt('never', '2026-07-01', NOW)).toBeNull()
  })

  it.each([
    ['30d', 30],
    ['90d', 90],
    ['365d', 365],
  ] as const)('counts %s from now', (choice, days) => {
    expect(tokenExpiresAt(choice, '', NOW)).toBe(new Date(NOW.getTime() + days * DAY).toISOString())
  })

  it('ignores a picked date for a preset length', () => {
    expect(tokenExpiresAt('30d', '2030-01-01', NOW)).toBe('2026-07-01T12:00:00.000Z')
  })

  it('expires a picked date at the end of that day, local time', () => {
    const iso = tokenExpiresAt('date', '2026-07-15', NOW)
    expect(iso).toBe(new Date(2026, 6, 15, 23, 59, 59).toISOString())
    const local = new Date(iso!)
    expect([local.getFullYear(), local.getMonth(), local.getDate()]).toEqual([2026, 6, 15])
    expect([local.getHours(), local.getMinutes(), local.getSeconds()]).toEqual([23, 59, 59])
  })

  it('sends no expiry while no date has been picked yet', () => {
    expect(tokenExpiresAt('date', '', NOW)).toBeNull()
  })

  it('counts from the current time when none is given', () => {
    const before = Date.now()
    const at = Date.parse(tokenExpiresAt('30d', '')!)
    expect(at).toBeGreaterThanOrEqual(before + 30 * DAY)
    expect(at).toBeLessThanOrEqual(Date.now() + 30 * DAY)
  })
})

describe('tokenExpiryState', () => {
  const NOW = Date.parse('2026-06-01T12:00:00Z')

  it('has no state for a token that never expires', () => {
    expect(tokenExpiryState(null, NOW)).toBe('none')
    expect(tokenExpiryState(undefined, NOW)).toBe('none')
    expect(tokenExpiryState('', NOW)).toBe('none')
  })

  it('is expired once the moment has passed, including the moment itself', () => {
    expect(tokenExpiryState(new Date(NOW - 1).toISOString(), NOW)).toBe('expired')
    expect(tokenExpiryState(new Date(NOW).toISOString(), NOW)).toBe('expired')
  })

  it('is soon within two weeks, up to and including the fourteenth day', () => {
    expect(tokenExpiryState(new Date(NOW + 1).toISOString(), NOW)).toBe('soon')
    expect(tokenExpiryState(new Date(NOW + 14 * DAY).toISOString(), NOW)).toBe('soon')
  })

  it('is ok beyond two weeks', () => {
    expect(tokenExpiryState(new Date(NOW + 14 * DAY + 1).toISOString(), NOW)).toBe('ok')
    expect(tokenExpiryState(new Date(NOW + 365 * DAY).toISOString(), NOW)).toBe('ok')
  })

  it('reads against the current time when none is given', () => {
    expect(tokenExpiryState(new Date(Date.now() - DAY).toISOString())).toBe('expired')
    expect(tokenExpiryState(new Date(Date.now() + 100 * DAY).toISOString())).toBe('ok')
  })
})

describe('tokenScopeLabel', () => {
  it('names a monitoring token as read-only', () => {
    expect(tokenScopeLabel('monitoring')).toBe('Monitoring (read-only)')
  })

  it('treats a full token, and one listed without a scope, as full access', () => {
    expect(tokenScopeLabel('full')).toBe('Full access')
    expect(tokenScopeLabel(undefined)).toBe('Full access')
  })
})

describe('tokenScopeTone', () => {
  it('gives the read-only scope the info tone', () => {
    expect(tokenScopeTone('monitoring')).toBe('info')
    expect(tokenScopeTone('full')).toBe('neutral')
    expect(tokenScopeTone(undefined)).toBe('neutral')
  })
})

describe('parseNetworkList', () => {
  it('splits on line breaks, commas and spaces and drops blanks', () => {
    expect(parseNetworkList('10.0.0.0/8\n192.168.1.5, fd00::/8  \n\n')).toEqual([
      '10.0.0.0/8',
      '192.168.1.5',
      'fd00::/8',
    ])
  })

  it('reads an empty text area as no networks', () => {
    expect(parseNetworkList('  \n ')).toEqual([])
  })
})

describe('formatNetworkList', () => {
  it('puts one network per line', () => {
    expect(formatNetworkList(['10.0.0.0/8', '127.0.0.1/32'])).toBe('10.0.0.0/8\n127.0.0.1/32')
  })
})
