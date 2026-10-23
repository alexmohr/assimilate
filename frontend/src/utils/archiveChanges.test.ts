// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { changeOf, previousArchive } from './archiveChanges'
import type { ArchiveEntryResponse } from '../types/generated'

function archive(name: string, start: string, hostname = 'web-01'): ArchiveEntryResponse {
  return {
    name,
    start,
    hostname,
    comment: '',
    original_size: 0,
    deduplicated_size: 0,
    matched: true,
    agent_hostname: hostname,
    agent_domain: null,
  }
}

function file(path: string, size: number, mtime = '2026-01-10T02:00:00') {
  return { type: '-', path, size, mtime, mode: '' }
}

describe('previousArchive', () => {
  const monday = archive('mon', '2026-01-05T02:00:00Z')
  const tuesday = archive('tue', '2026-01-06T02:00:00Z')
  const wednesday = archive('wed', '2026-01-07T02:00:00Z')
  const otherHost = archive('db-tue', '2026-01-06T12:00:00Z', 'db-01')

  it('is the newest older archive of the same machine', () => {
    expect(previousArchive([monday, wednesday, otherHost, tuesday], wednesday)?.name).toBe('tue')
  })

  it('is none for the oldest archive', () => {
    expect(previousArchive([monday, tuesday], monday)).toBeNull()
  })
})

describe('changeOf', () => {
  const before = new Map([
    ['etc/hosts', file('etc/hosts', 10)],
    ['etc/fstab', file('etc/fstab', 20)],
  ])

  it('marks a file the previous folder did not hold as new', () => {
    expect(changeOf(file('etc/motd', 5), before)).toBe('new')
  })

  it('marks a file whose size or modification time differs as changed', () => {
    expect(changeOf(file('etc/hosts', 11), before)).toBe('changed')
    expect(changeOf(file('etc/hosts', 10, '2026-01-11T02:00:00'), before)).toBe('changed')
  })

  it('leaves unchanged files, folders and unknown folders unmarked', () => {
    expect(changeOf(file('etc/fstab', 20), before)).toBeNull()
    expect(changeOf({ ...file('etc/new-dir', 0), type: 'd' }, before)).toBeNull()
    expect(changeOf(file('etc/motd', 5), undefined)).toBeNull()
  })
})
