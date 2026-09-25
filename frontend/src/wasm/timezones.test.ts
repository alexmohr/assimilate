// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

import { nextCronRuns } from './timezones'

const iso = (dates: Date[]): string[] => dates.map((d) => d.toISOString())

describe('timezone WebAssembly module', () => {
  it('lists consecutive runs in UTC', async () => {
    const runs = await nextCronRuns('0 */6 * * *', new Date('2026-01-01T10:00:00Z'), 'UTC', 3)
    expect(iso(runs)).toEqual([
      '2026-01-01T12:00:00.000Z',
      '2026-01-01T18:00:00.000Z',
      '2026-01-02T00:00:00.000Z',
    ])
  })

  it('rolls a run in a spring-forward gap to the end of the gap, like the scheduler', async () => {
    const runs = await nextCronRuns(
      '30 2 * * *',
      new Date('2026-03-28T00:00:00Z'),
      'Europe/Berlin',
      3,
    )
    expect(iso(runs)).toEqual([
      '2026-03-28T01:30:00.000Z',
      '2026-03-29T01:00:00.000Z',
      '2026-03-30T00:30:00.000Z',
    ])
  })

  it('rejects an unknown timezone', async () => {
    await expect(nextCronRuns('0 2 * * *', new Date(), 'Not/A/Zone', 3)).rejects.toThrow(
      'invalid timezone',
    )
  })

  it('rejects an invalid expression', async () => {
    await expect(nextCronRuns('nope', new Date(), 'UTC', 3)).rejects.toThrow()
  })
})
