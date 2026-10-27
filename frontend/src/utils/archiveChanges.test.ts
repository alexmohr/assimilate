// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { archiveSeries, changeOf, previousArchive } from './archiveChanges'
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

  it('skips an archive of another series on the same machine', () => {
    const nightlyMon = archive('web-01-nightly-2026-01-05T02:00:00', '2026-01-05T02:00:00Z')
    const extraTue = archive('web-01-extra-2026-01-06T03:30:00', '2026-01-06T03:30:00Z')
    const nightlyWed = archive('web-01-nightly-2026-01-07T02:00:00', '2026-01-07T02:00:00Z')

    expect(previousArchive([nightlyMon, extraTue, nightlyWed], nightlyWed)?.name).toBe(
      'web-01-nightly-2026-01-05T02:00:00',
    )
    expect(previousArchive([nightlyMon, extraTue, nightlyWed], extraTue)).toBeNull()
  })

  it('keeps to the schedule that created the archive when the server names it', () => {
    const nightlyMon = {
      ...archive('web-01-2026-01-05T02:00:00', '2026-01-05T02:00:00Z'),
      schedule_id: 1,
    }
    const extraTue = {
      ...archive('web-01-2026-01-06T03:30:00', '2026-01-06T03:30:00Z'),
      schedule_id: 2,
    }
    const nightlyWed = {
      ...archive('web-01-2026-01-07T02:00:00', '2026-01-07T02:00:00Z'),
      schedule_id: 1,
    }

    expect(previousArchive([nightlyMon, extraTue, nightlyWed], nightlyWed)?.name).toBe(
      'web-01-2026-01-05T02:00:00',
    )
    expect(previousArchive([nightlyMon, extraTue, nightlyWed], extraTue)).toBeNull()
  })

  it('falls back to the name series when an archive has no recorded schedule', () => {
    const before = archive('web-01-2026-01-05T02:00:00', '2026-01-05T02:00:00Z')
    const after = {
      ...archive('web-01-2026-01-06T02:00:00', '2026-01-06T02:00:00Z'),
      schedule_id: 1,
    }

    expect(previousArchive([before, after], after)?.name).toBe('web-01-2026-01-05T02:00:00')
  })
})

describe('archiveSeries', () => {
  it('is the name without its trailing start time', () => {
    expect(archiveSeries('web-01-nightly-2026-01-05T02:00:00')).toBe('web-01-nightly')
    expect(archiveSeries('web-01-2026-01-05T02:00:00.123456Z')).toBe('web-01')
  })

  it('is unknown for a name without a start time', () => {
    expect(archiveSeries('manual-before-upgrade')).toBeNull()
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
