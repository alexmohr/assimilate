// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { ArchiveEntryResponse, FileDiffLine } from '../types/generated'
import {
  comparableArchives,
  defaultComparison,
  entryKind,
  toSplitRows,
  unavailableReason,
} from './fileDiff'

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

function line(kind: FileDiffLine['kind'], text: string): FileDiffLine {
  return {
    kind,
    base_line: kind === 'added' ? null : 1,
    target_line: kind === 'removed' ? null : 1,
    segments: [{ text, emphasized: false }],
  }
}

const MONDAY = archive('mon', '2026-10-13T03:00:00Z')
const TUESDAY = archive('tue', '2026-10-14T03:00:00Z')
const WEDNESDAY = archive('wed', '2026-10-15T03:00:00Z')

describe('entryKind', () => {
  it('only treats a regular file as having contents to compare', () => {
    expect(entryKind('-')).toBe('file')
    expect(entryKind('d')).toBe('other')
    expect(entryKind('l')).toBe('other')
  })
})

describe('comparableArchives', () => {
  it('lists every other archive, newest first', () => {
    expect(comparableArchives([MONDAY, WEDNESDAY, TUESDAY], TUESDAY)).toEqual([WEDNESDAY, MONDAY])
  })

  it("leaves out another machine's archives, where the same path is another file", () => {
    const otherHost = { ...archive('db-01-tue', '2026-10-14T04:00:00Z'), agent_hostname: 'db-01' }
    const otherDomain = { ...archive('web-01-dc2', '2026-10-14T05:00:00Z'), agent_domain: 'dc2' }

    expect(comparableArchives([MONDAY, otherHost, otherDomain, WEDNESDAY], TUESDAY)).toEqual([
      WEDNESDAY,
      MONDAY,
    ])
  })
})

describe('defaultComparison', () => {
  it('opens against the newest archive older than the current one', () => {
    expect(defaultComparison([MONDAY, TUESDAY, WEDNESDAY], WEDNESDAY)).toBe(TUESDAY)
    expect(defaultComparison([WEDNESDAY, MONDAY, TUESDAY], TUESDAY)).toBe(MONDAY)
  })

  it('falls back to the newest of the rest for the oldest archive', () => {
    expect(defaultComparison([MONDAY, TUESDAY, WEDNESDAY], MONDAY)).toBe(WEDNESDAY)
  })

  it('has nothing to compare an only archive with', () => {
    expect(defaultComparison([MONDAY], MONDAY)).toBeNull()
  })
})

describe('toSplitRows', () => {
  it('puts an unchanged line on both sides', () => {
    const same = line('context', 'user www;')
    expect(toSplitRows([same])).toEqual([{ base: same, target: same }])
  })

  it('pairs the removed and added lines of a change in order', () => {
    const removed = [line('removed', 'a'), line('removed', 'b')]
    const added = [line('added', 'A')]
    expect(toSplitRows([...removed, ...added])).toEqual([
      { base: removed[0], target: added[0] },
      { base: removed[1], target: null },
    ])
  })

  it('pads the base side of a pure addition', () => {
    const added = line('added', 'new')
    expect(toSplitRows([added])).toEqual([{ base: null, target: added }])
  })

  it('starts a new change when a removal follows additions', () => {
    const first = line('removed', 'a')
    const second = line('added', 'A')
    const third = line('removed', 'b')
    expect(toSplitRows([first, second, third])).toEqual([
      { base: first, target: second },
      { base: third, target: null },
    ])
  })
})

describe('unavailableReason', () => {
  const names = { base: 'Monday', target: 'Tuesday' }

  it('names the side that is not text', () => {
    expect(unavailableReason({ status: 'binary', side: 'target' }, names)).toBe(
      'The version in Tuesday is not text, so it cannot be compared line by line.',
    )
  })

  it('names the side that is too large and the limit', () => {
    expect(
      unavailableReason({ status: 'too_large', side: 'base', limit_bytes: 1048576 }, names),
    ).toBe('The version in Monday is larger than 1.0 MB, the most the server compares.')
  })

  it('says why the file could not be read on its host now', () => {
    const reasons = [
      ['not_in_sources', 'outside every backup source configured for web-01'],
      ['not_found', 'no longer exists on web-01'],
      ['not_regular_file', 'a directory, a link or another special file'],
      ['unreadable', 'could not read the file'],
      ['agent_offline', 'web-01 is offline'],
      ['timed_out', 'did not answer in time'],
    ] as const
    for (const [reason, sentence] of reasons) {
      expect(unavailableReason({ status: 'live_unavailable', reason }, names, 'web-01')).toContain(
        sentence,
      )
    }
  })
})
