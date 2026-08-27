// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { domainParams } from './agent'

describe('domainParams', () => {
  it('omits the domain entirely when unset', () => {
    expect(domainParams(undefined)).toEqual({})
    expect(domainParams(null)).toEqual({})
  })

  it('omits the domain when it is an empty string', () => {
    expect(domainParams('')).toEqual({})
  })

  it('includes the domain when set', () => {
    expect(domainParams('lab.example.com')).toEqual({ domain: 'lab.example.com' })
  })
})
