// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { ref, type Ref } from 'vue'
import { useArchiveList, ARCHIVE_SORT_OPTIONS } from './useArchiveList'
import type { ArchiveEntry } from './useArchiveBrowser'

function archive(overrides: Partial<ArchiveEntry>): ArchiveEntry {
  return {
    name: 'web-01-2026-01-01',
    start: '2026-01-01T00:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 100,
    deduplicated_size: 10,
    matched: true,
    agent_hostname: 'web-01',
    ...overrides,
  }
}

const ARCHIVES = [
  archive({ name: 'alpha-jan', start: '2026-01-01T00:00:00Z', original_size: 100 }),
  archive({
    name: 'charlie-mar',
    start: '2026-03-01T00:00:00Z',
    original_size: 300,
    deduplicated_size: 30,
  }),
  archive({
    name: 'bravo-feb',
    start: '2026-02-01T00:00:00Z',
    original_size: 200,
    deduplicated_size: 20,
  }),
  archive({
    name: 'delta-feb',
    hostname: 'db-01',
    agent_hostname: null,
    start: '2026-02-15T00:00:00Z',
    original_size: 250,
    deduplicated_size: 25,
    matched: false,
  }),
]

function list() {
  return useArchiveList(ref(ARCHIVES))
}

describe('useArchiveList', () => {
  it('offers both directions for each sortable column', () => {
    expect(ARCHIVE_SORT_OPTIONS.map((o) => o.value)).toEqual([
      'date-desc',
      'date-asc',
      'size-desc',
      'size-asc',
      'dedup-desc',
      'dedup-asc',
    ])
    for (const option of ARCHIVE_SORT_OPTIONS) {
      expect(option.label.length).toBeGreaterThan(0)
    }
  })

  it('sorts newest first by default', () => {
    expect(list().ordered.value.map((a) => a.name)).toEqual([
      'charlie-mar',
      'delta-feb',
      'bravo-feb',
      'alpha-jan',
    ])
  })

  it('sorts by date, size and deduplicated size in both directions', () => {
    const l = list()

    l.sortMode.value = 'date-asc'
    expect(l.ordered.value.map((a) => a.name)).toEqual([
      'alpha-jan',
      'bravo-feb',
      'delta-feb',
      'charlie-mar',
    ])

    l.sortMode.value = 'size-desc'
    expect(l.ordered.value.map((a) => a.original_size)).toEqual([300, 250, 200, 100])

    l.sortMode.value = 'size-asc'
    expect(l.ordered.value.map((a) => a.original_size)).toEqual([100, 200, 250, 300])

    l.sortMode.value = 'dedup-desc'
    expect(l.ordered.value.map((a) => a.deduplicated_size)).toEqual([30, 25, 20, 10])

    l.sortMode.value = 'dedup-asc'
    expect(l.ordered.value.map((a) => a.deduplicated_size)).toEqual([10, 20, 25, 30])
  })

  // The default arm is the safety net for a value the union does not cover -
  // a sort persisted by an older build, or an option added to the picker
  // without a case here. It has to leave the list intact, not empty it.
  it('leaves the order alone for a sort mode it does not know', () => {
    const l = list()
    ;(l.sortMode as unknown as Ref<string>).value = 'colour-asc'
    expect(l.ordered.value.map((a) => a.name)).toEqual(ARCHIVES.map((a) => a.name))
  })

  it('filters case-insensitively on the archive name', () => {
    const l = list()
    l.filter.value = 'DELTA'
    expect(l.ordered.value.map((a) => a.name)).toEqual(['delta-feb'])
  })

  it('also filters on the hostname, so a host can be isolated by name', () => {
    const l = list()
    l.filter.value = 'db-01'
    expect(l.ordered.value.map((a) => a.name)).toEqual(['delta-feb'])
  })

  it('groups matched archives under their agent and unmatched under the hostname borg recorded', () => {
    const l = list()

    expect(l.grouped.value.map((g) => g.hostname)).toEqual(['db-01', 'web-01'])

    const web = l.grouped.value.find((g) => g.hostname === 'web-01')
    expect(web?.matched).toBe(true)
    expect(web?.agentHostname).toBe('web-01')
    expect(web?.archives).toHaveLength(3)

    const db = l.grouped.value.find((g) => g.hostname === 'db-01')
    expect(db?.matched).toBe(false)
    expect(db?.agentHostname).toBeNull()
  })

  it('counts unmatched archives and lists their hostnames', () => {
    const l = list()
    expect(l.unmatchedCount.value).toBe(1)
    expect(l.unmatchedHostnames.value).toEqual(['db-01'])
  })

  it('starts every group collapsed and toggles them one at a time', () => {
    const l = list()

    expect(l.isGroupCollapsed('web-01')).toBe(true)
    l.toggleGroup('web-01')
    expect(l.isGroupCollapsed('web-01')).toBe(false)
    expect(l.isGroupCollapsed('db-01')).toBe(true)
    l.toggleGroup('web-01')
    expect(l.isGroupCollapsed('web-01')).toBe(true)
  })

  it('clears the filter and collapse state on reset, keeping the chosen sort', () => {
    const l = list()
    l.filter.value = 'delta'
    l.sortMode.value = 'size-asc'
    l.toggleGroup('web-01')

    l.reset()

    expect(l.filter.value).toBe('')
    expect(l.isGroupCollapsed('web-01')).toBe(true)
    // The sort is a user preference, not per-repository state.
    expect(l.sortMode.value).toBe('size-asc')
  })
})
