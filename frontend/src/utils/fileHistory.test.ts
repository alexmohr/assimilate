// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { FileHistoryResponse } from '../types/generated'
import {
  LIVE,
  archiveCount,
  archiveSpan,
  defaultPair,
  listedEntries,
  versionChoices,
} from './fileHistory'

function history(agentOnline: boolean): FileHistoryResponse {
  return {
    path: 'etc/hosts',
    hostname: 'web-01',
    agent_online: agentOnline,
    entries: [
      {
        kind: 'version',
        archives: [
          { name: 'b', start: '2026-10-15T03:00:00Z' },
          { name: 'a', start: '2026-10-14T03:00:00Z' },
        ],
        size: 2,
        mtime: 't2',
      },
      { kind: 'absent', archives: [{ name: 'gap', start: '2026-10-13T03:00:00Z' }] },
      {
        kind: 'version',
        archives: [{ name: 'z', start: '2026-10-12T03:00:00Z' }],
        size: 1,
        mtime: 't1',
      },
    ],
  }
}

describe('fileHistory', () => {
  it('keys each entry by its newest archive', () => {
    expect(listedEntries(history(true)).map((entry) => entry.key)).toEqual(['b', 'gap', 'z'])
  })

  it('offers the file now, then every stored version', () => {
    expect(versionChoices(history(true), 'a')).toEqual([
      { key: LIVE, archiveName: null },
      { key: 'b', archiveName: 'b' },
      { key: 'z', archiveName: 'z' },
    ])
    expect(versionChoices(history(false), 'a').map((choice) => choice.key)).toEqual(['b', 'z'])
  })

  it('adds the opened archive when no version holds it', () => {
    expect(versionChoices(history(false), 'gap').map((choice) => choice.key)).toEqual([
      'b',
      'z',
      'gap',
    ])
  })

  it('opens on the newest two choices, the newer as the To side', () => {
    expect(defaultPair(versionChoices(history(true), 'a'))).toEqual({ from: 'b', to: LIVE })
    expect(defaultPair([{ key: 'only', archiveName: 'only' }])).toEqual({ from: 'only', to: null })
    expect(defaultPair([])).toEqual({ from: null, to: null })
  })

  it('describes a run of archives', () => {
    const [run, gap] = history(true).entries
    expect(archiveCount(run)).toBe('2 archives')
    expect(archiveCount(gap)).toBe('1 archive')
    expect(archiveSpan(run)).toContain(' to ')
    expect(archiveSpan(gap)).not.toContain(' to ')
    expect(archiveSpan({ kind: 'absent', archives: [] })).toBe('')
  })
})
