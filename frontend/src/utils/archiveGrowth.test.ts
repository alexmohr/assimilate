// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { ArchiveEntryResponse, GrowthNode } from '../types/generated'
import {
  USUAL_GROWTH_WINDOW,
  folderAt,
  growthFactor,
  shareLabel,
  usualGrowth,
} from './archiveGrowth'

function archive(
  name: string,
  start: string,
  deduplicated_size: number,
  host = 'web-01',
  domain: string | null = null,
): ArchiveEntryResponse {
  return {
    name,
    start,
    hostname: host,
    comment: '',
    original_size: 0,
    deduplicated_size,
    matched: true,
    agent_hostname: host,
    agent_domain: domain,
  }
}

function folder(name: string, bytes: number, children: GrowthNode[] = []): GrowthNode {
  return { name, kind: 'directory', bytes, files: 1, children }
}

function file(name: string, bytes: number): GrowthNode {
  return { name, kind: 'file', bytes, files: 1, children: [] }
}

describe('usualGrowth', () => {
  it("averages the same host's earlier archives in the repository", () => {
    const current = archive('web-01-27', '2026-10-27T02:00:00Z', 3000)
    const archives = [
      current,
      archive('web-01-26', '2026-10-26T02:00:00Z', 400),
      archive('web-01-25', '2026-10-25T02:00:00Z', 200),
      archive('db-01-26', '2026-10-26T03:00:00Z', 90_000, 'db-01'),
      archive('web-01-28', '2026-10-28T02:00:00Z', 9_000),
    ]

    expect(usualGrowth(current, archives)).toBe(300)
  })

  it('tells hosts sharing a hostname apart by domain', () => {
    const current = archive('web-27', '2026-10-27T02:00:00Z', 3000, 'web', 'eu')
    const archives = [
      current,
      archive('web-26-eu', '2026-10-26T02:00:00Z', 100, 'web', 'eu'),
      archive('web-26-us', '2026-10-26T02:00:00Z', 50_000, 'web', 'us'),
    ]

    expect(usualGrowth(current, archives)).toBe(100)
  })

  it('takes only the most recent archives into account', () => {
    const current = archive('now', '2026-10-30T00:00:00Z', 1)
    const earlier = Array.from({ length: USUAL_GROWTH_WINDOW + 3 }, (_, i) =>
      archive(`a${i}`, `2026-10-${String(20 - i).padStart(2, '0')}T00:00:00Z`, i < 7 ? 70 : 7000),
    )

    expect(usualGrowth(current, [current, ...earlier])).toBe(70)
  })

  it('has nothing to compare the first archive of a host with', () => {
    const current = archive('first', '2026-10-27T02:00:00Z', 3000)

    expect(usualGrowth(current, [current])).toBeNull()
  })
})

describe('growthFactor', () => {
  it('rounds to one decimal', () => {
    expect(growthFactor(3120, 392)).toBe(8)
    expect(growthFactor(500, 400)).toBe(1.3)
  })

  it('has no factor without a usual growth', () => {
    expect(growthFactor(100, null)).toBeNull()
    expect(growthFactor(100, 0)).toBeNull()
  })
})

describe('folderAt', () => {
  const tree = [
    folder('home/alex', 300, [folder('Downloads', 200, [file('ubuntu.iso', 200)]), file('a', 1)]),
    file('notes', 5),
  ]

  it('opens the top level for an empty path', () => {
    const { entries, trail } = folderAt(tree, [])

    expect(entries).toBe(tree)
    expect(trail).toEqual([])
  })

  it('walks down by folder names', () => {
    const { entries, trail } = folderAt(tree, ['home/alex', 'Downloads'])

    expect(entries.map((node) => node.name)).toEqual(['ubuntu.iso'])
    expect(trail.map((node) => node.name)).toEqual(['home/alex', 'Downloads'])
  })

  it('stops at the deepest folder that exists, and never opens a file', () => {
    expect(folderAt(tree, ['home/alex', 'gone']).trail.map((node) => node.name)).toEqual([
      'home/alex',
    ])
    expect(folderAt(tree, ['notes']).trail).toEqual([])
  })
})

describe('shareLabel', () => {
  it('rounds to whole percent and marks tiny shares', () => {
    expect(shareLabel(740, 1000)).toBe('74%')
    expect(shareLabel(4, 1000)).toBe('<1%')
    expect(shareLabel(0, 1000)).toBe('0%')
    expect(shareLabel(10, 0)).toBe('0%')
  })
})
