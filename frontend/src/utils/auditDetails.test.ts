// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { hasAuditDetails } from './auditDetails'

describe('hasAuditDetails', () => {
  it('is true for an action that recorded details', () => {
    expect(hasAuditDetails({ details: { archive: 'nightly' } })).toBe(true)
  })

  it('is false for an action that records none', () => {
    expect(hasAuditDetails({ details: {} })).toBe(false)
  })
})
