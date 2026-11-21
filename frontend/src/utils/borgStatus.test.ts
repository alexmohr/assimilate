// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { borgProblem, borgVersion } from './borgStatus'

describe('borgProblem', () => {
  it('has nothing to say about a host that has not reported or has borg', () => {
    expect(borgProblem(null)).toBeNull()
    expect(borgProblem(undefined)).toBeNull()
    expect(borgProblem({ state: 'available', version: '1.4.1' })).toBeNull()
  })

  it('names a missing borg and what the agent looked for', () => {
    const problem = borgProblem({ state: 'missing', binary: '/opt/borg' })
    expect(problem?.chip).toBe('Borg not installed')
    expect(problem?.detail).toContain('/opt/borg')
  })

  it('says why a borg that does not run could not be used', () => {
    const problem = borgProblem({ state: 'broken', binary: 'borg', reason: 'permission denied' })
    expect(problem?.chip).toBe('Borg not working')
    expect(problem?.detail).toContain('permission denied')
  })
})

describe('borgVersion', () => {
  it('is the version of a working borg only', () => {
    expect(borgVersion({ borg: { state: 'available', version: '1.2.8' } })).toBe('1.2.8')
    expect(borgVersion({ borg: { state: 'missing', binary: 'borg' } })).toBeNull()
    expect(borgVersion({ borg: null })).toBeNull()
  })
})
