// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  headersForRequest,
  headersNeedingReentry,
  keepsSavedValue,
  newHeaderRow,
  rowsFromSaved,
  type WebhookHeaderRow,
} from './webhookHeaders'

function row(name: string, value: string, savedName: string | null = null): WebhookHeaderRow {
  return { ...newHeaderRow(), name, value, savedName }
}

describe('rowsFromSaved', () => {
  it('starts every saved header with a blank value and unique ids', () => {
    const rows = rowsFromSaved([
      { name: 'Authorization', has_value: true },
      { name: 'X-Empty', has_value: false },
    ])
    expect(rows.map((r) => [r.name, r.value, r.savedName])).toEqual([
      ['Authorization', '', 'Authorization'],
      ['X-Empty', '', null],
    ])
    expect(new Set(rows.map((r) => r.id)).size).toBe(2)
  })
})

describe('keepsSavedValue', () => {
  it('holds while the name still matches, ignoring case and whitespace', () => {
    expect(keepsSavedValue(row(' authorization ', '', 'Authorization'))).toBe(true)
  })

  it('stops once the row is renamed or was never saved with a value', () => {
    expect(keepsSavedValue(row('X-Other', '', 'Authorization'))).toBe(false)
    expect(keepsSavedValue(row('Authorization', ''))).toBe(false)
  })
})

describe('headersForRequest', () => {
  it('sends blank values as null and drops rows without a name', () => {
    expect(
      headersForRequest([
        row(' Authorization ', ''),
        row('X-Team', 'ops'),
        row('  ', 'orphan value'),
      ]),
    ).toEqual({ Authorization: null, 'X-Team': 'ops' })
  })

  it('is empty for no rows, which removes every saved header', () => {
    expect(headersForRequest([])).toEqual({})
  })
})

describe('headersNeedingReentry', () => {
  const saved = [row('Authorization', '', 'Authorization'), row('X-Empty', '')]
  const base = 'https://hooks.example.com/notify'

  it('asks for nothing while creating or on the same origin', () => {
    expect(headersNeedingReentry(undefined, 'https://elsewhere.example', saved)).toEqual([])
    expect(headersNeedingReentry(base, 'https://HOOKS.example.com:443/other', saved)).toEqual([])
  })

  it('names the saved values a new scheme, host or port needs again', () => {
    for (const url of [
      'https://hooks.attacker.example/notify',
      'http://hooks.example.com/notify',
      'https://hooks.example.com:8443/notify',
      'not a url',
      'ftp://hooks.example.com/notify',
    ]) {
      expect(headersNeedingReentry(base, url, saved)).toEqual(['Authorization'])
    }
  })

  it('is satisfied once the value is typed again', () => {
    const retyped = [row('Authorization', 'Bearer new', 'Authorization')]
    expect(headersNeedingReentry(base, 'https://other.example/notify', retyped)).toEqual([])
  })
})
