// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { ArchiveEntryResponse } from '../types/generated'
import { archiveKey, inRepo, reposOf, sameArchive } from './repoArchive'

const ENTRY: ArchiveEntryResponse = {
  name: 'web-01-2026-10-20',
  start: '2026-10-20T03:00:00Z',
  hostname: 'web-01',
  comment: '',
  original_size: 1,
  deduplicated_size: 1,
  matched: true,
  agent_hostname: 'web-01',
  agent_domain: null,
}

describe('repoArchive', () => {
  it('tags a repository list with its repository', () => {
    expect(inRepo([ENTRY], { id: 3, name: 'nightly' })).toEqual([
      { ...ENTRY, repo_id: 3, repo_name: 'nightly' },
    ])
  })

  it('tells two copies of one name apart', () => {
    const [a] = inRepo([ENTRY], { id: 3, name: 'nightly' })
    const [b] = inRepo([ENTRY], { id: 4, name: 'offsite' })
    expect(archiveKey(a!)).not.toBe(archiveKey(b!))
    expect(sameArchive(a, b)).toBe(false)
    expect(sameArchive(a, { ...a! })).toBe(true)
    expect(sameArchive(null, a)).toBe(false)
  })

  it('counts the archives of each repository, in list order', () => {
    const list = [
      ...inRepo([ENTRY], { id: 4, name: 'offsite' }),
      ...inRepo([ENTRY, { ...ENTRY, name: 'other' }], { id: 3, name: 'nightly' }),
    ]
    expect(reposOf(list)).toEqual([
      { id: 4, name: 'offsite', count: 1 },
      { id: 3, name: 'nightly', count: 2 },
    ])
  })
})
