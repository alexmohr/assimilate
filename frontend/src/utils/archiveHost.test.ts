// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { resolveArchiveHost } from './archiveHost'

describe('resolveArchiveHost', () => {
  it('prefers the agent hostname on a matched archive', () => {
    expect(
      resolveArchiveHost({ matched: true, agent_hostname: 'web-01.internal', hostname: 'web-01' }),
    ).toBe('web-01.internal')
  })

  it('falls back to what borg recorded when a matched archive has no agent hostname', () => {
    expect(resolveArchiveHost({ matched: true, agent_hostname: null, hostname: 'web-01' })).toBe(
      'web-01',
    )
  })

  it('ignores the agent hostname on an unmatched archive', () => {
    // The whole point of the guard: on an unmatched archive `agent_hostname` is
    // whichever agent the name happened to resemble, not where the backup came
    // from. Trusting it labels and links the row to the wrong machine.
    expect(
      resolveArchiveHost({ matched: false, agent_hostname: 'web-01', hostname: 'legacy-nas' }),
    ).toBe('legacy-nas')
  })

  it('treats a missing matched flag as unmatched', () => {
    expect(
      resolveArchiveHost({ matched: null, agent_hostname: 'web-01', hostname: 'legacy-nas' }),
    ).toBe('legacy-nas')
  })
})
