// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { formatNetworkList, parseNetworkList, tokenScopeLabel, tokenScopeTone } from './apiToken'

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
