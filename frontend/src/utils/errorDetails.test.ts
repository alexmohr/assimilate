// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, beforeEach } from 'vitest'
import { consumeErrorDetails, storeErrorDetails } from './errorDetails'

describe('errorDetails', () => {
  beforeEach(() => {
    sessionStorage.clear()
  })

  it('returns undefined when nothing was stored', () => {
    expect(consumeErrorDetails()).toBeUndefined()
  })

  it('round-trips stored details', () => {
    storeErrorDetails({
      source: 'frontend',
      name: 'TypeError',
      message: 'Cannot read properties of undefined',
      stack: 'TypeError: ...\n    at foo',
    })

    expect(consumeErrorDetails()).toEqual({
      source: 'frontend',
      name: 'TypeError',
      message: 'Cannot read properties of undefined',
      stack: 'TypeError: ...\n    at foo',
    })
  })

  it('clears details after consuming them once', () => {
    storeErrorDetails({ source: 'backend', message: 'Internal server error' })

    expect(consumeErrorDetails()).toBeDefined()
    expect(consumeErrorDetails()).toBeUndefined()
  })

  it('returns undefined for malformed stored data', () => {
    sessionStorage.setItem('assimilate:last-error', 'not json')
    expect(consumeErrorDetails()).toBeUndefined()
  })
})
