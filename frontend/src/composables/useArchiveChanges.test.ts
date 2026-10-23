// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'
import { PREVIOUS_LISTING_LIMIT, useArchiveChanges } from './useArchiveChanges'
import type { ArchiveEntryResponse } from '../types/generated'

vi.mock('../api/archives', () => ({ getArchiveContents: vi.fn() }))

import { getArchiveContents } from '../api/archives'

function archive(name: string, start: string): ArchiveEntryResponse {
  return {
    name,
    start,
    hostname: 'web-01',
    comment: '',
    original_size: 0,
    deduplicated_size: 0,
    matched: true,
    agent_hostname: 'web-01',
    agent_domain: null,
  }
}

const MONDAY = archive('mon', '2026-01-05T02:00:00Z')
const TUESDAY = archive('tue', '2026-01-06T02:00:00Z')

function file(path: string, size: number) {
  return { type: '-', path, size, mtime: '2026-01-05T01:00:00', mode: '' }
}

function served(entries: ReturnType<typeof file>[], status = 'done') {
  return { index_status: status, entries }
}

describe('useArchiveChanges', () => {
  beforeEach(() => {
    vi.mocked(getArchiveContents).mockReset()
    vi.mocked(getArchiveContents).mockResolvedValue(
      served([{ ...file('etc', 0), type: 'd' }, file('etc/hosts', 10)]),
    )
  })

  it('compares a folder with the same folder in the previous archive', async () => {
    const changes = useArchiveChanges(
      () => 5,
      () => TUESDAY,
      () => [MONDAY, TUESDAY],
    )

    await changes.load('etc')
    await changes.load('etc')

    expect(changes.previous.value?.name).toBe('mon')
    expect(getArchiveContents).toHaveBeenCalledTimes(1)
    expect(getArchiveContents).toHaveBeenCalledWith(5, 'mon', {
      path: 'etc',
      limit: PREVIOUS_LISTING_LIMIT,
    })
    expect(changes.changeOf(file('etc/hosts', 10))).toBeNull()
    expect(changes.changeOf(file('etc/hosts', 11))).toBe('changed')
    expect(changes.changeOf(file('etc/motd', 1))).toBe('new')
    expect(changes.changeOf(file('var/log', 1))).toBeNull()
  })

  it('marks nothing in the oldest archive', async () => {
    const changes = useArchiveChanges(
      () => 5,
      () => MONDAY,
      () => [MONDAY, TUESDAY],
    )

    await changes.load('etc')

    expect(getArchiveContents).not.toHaveBeenCalled()
    expect(changes.changeOf(file('etc/motd', 1))).toBeNull()
  })

  it('leaves a folder unmarked until the previous index is done, then asks again', async () => {
    vi.mocked(getArchiveContents).mockResolvedValueOnce(served([], 'indexing'))
    const changes = useArchiveChanges(
      () => 5,
      () => TUESDAY,
      () => [MONDAY, TUESDAY],
    )

    await changes.load('etc')
    expect(changes.changeOf(file('etc/motd', 1))).toBeNull()

    await changes.load('etc')
    expect(changes.changeOf(file('etc/motd', 1))).toBe('new')
  })

  it('leaves a folder too large to read in one go unmarked', async () => {
    vi.mocked(getArchiveContents).mockResolvedValue(
      served(Array.from({ length: PREVIOUS_LISTING_LIMIT }, (_, i) => file(`etc/f${i}`, 1))),
    )
    const changes = useArchiveChanges(
      () => 5,
      () => TUESDAY,
      () => [MONDAY, TUESDAY],
    )

    await changes.load('etc')

    expect(changes.changeOf(file('etc/motd', 1))).toBeNull()
  })

  it('asks again after a failed read', async () => {
    vi.mocked(getArchiveContents).mockRejectedValueOnce(new Error('offline'))
    const changes = useArchiveChanges(
      () => 5,
      () => TUESDAY,
      () => [MONDAY, TUESDAY],
    )

    await changes.load('etc')
    await changes.load('etc')

    expect(getArchiveContents).toHaveBeenCalledTimes(2)
    expect(changes.changeOf(file('etc/motd', 1))).toBe('new')
  })

  it('forgets the comparison when another archive opens, dropping a late answer', async () => {
    const current = ref<ArchiveEntryResponse | null>(TUESDAY)
    let release: () => void = () => {}
    vi.mocked(getArchiveContents).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = () => resolve(served([file('etc/hosts', 10)]))
        }),
    )
    const changes = useArchiveChanges(
      () => 5,
      () => current.value,
      () => [MONDAY, TUESDAY],
    )

    const late = changes.load('etc')
    current.value = null
    await nextTick()
    release()
    await late

    expect(changes.previous.value).toBeNull()
    current.value = TUESDAY
    await nextTick()
    expect(changes.changeOf(file('etc/motd', 1))).toBeNull()
  })
})
