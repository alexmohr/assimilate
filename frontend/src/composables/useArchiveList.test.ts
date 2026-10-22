// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { ref, type Ref } from 'vue'
import { useArchiveList, ARCHIVE_SORT_OPTIONS, GROUP_COLLAPSE_THRESHOLD } from './useArchiveList'
import type { RepoArchive } from '../utils/repoArchive'

function archive(overrides: Partial<RepoArchive>): RepoArchive {
  return {
    repo_id: 1,
    repo_name: 'nightly',
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

  it('starts a short host list open and toggles groups one at a time', () => {
    // Two hosts: collapsing by default here would hide the whole list behind
    // a click for no benefit.
    const l = list()

    expect(l.isGroupCollapsed('web-01')).toBe(false)
    l.toggleGroup('web-01')
    expect(l.isGroupCollapsed('web-01')).toBe(true)
    expect(l.isGroupCollapsed('db-01')).toBe(false)
    l.toggleGroup('web-01')
    expect(l.isGroupCollapsed('web-01')).toBe(false)
  })

  it('starts every group collapsed once there are more hosts than the threshold', () => {
    const many = ref(
      Array.from({ length: GROUP_COLLAPSE_THRESHOLD + 1 }, (_, i) =>
        archive({ name: `host-${i}-jan`, hostname: `host-${i}`, agent_hostname: `host-${i}` }),
      ),
    )
    const l = useArchiveList(many)

    expect(l.grouped.value.length).toBe(GROUP_COLLAPSE_THRESHOLD + 1)
    expect(l.isGroupCollapsed('host-0')).toBe(true)
    l.toggleGroup('host-0')
    expect(l.isGroupCollapsed('host-0')).toBe(false)
    expect(l.isGroupCollapsed('host-1')).toBe(true)
  })

  it('clears the filter and collapse state on reset, keeping the chosen sort', () => {
    const l = list()
    l.filter.value = 'delta'
    l.sortMode.value = 'size-asc'
    l.toggleGroup('web-01')
    expect(l.isGroupCollapsed('web-01')).toBe(true)

    l.reset()

    expect(l.filter.value).toBe('')
    expect(l.isGroupCollapsed('web-01')).toBe(false)
    // The sort is a user preference, not per-repository state.
    expect(l.sortMode.value).toBe('size-asc')
  })

  describe('across several repositories', () => {
    const SPREAD = [
      archive({ name: 'web-01-a', repo_id: 1, repo_name: 'nightly' }),
      archive({ name: 'web-01-b', repo_id: 2, repo_name: 'offsite' }),
      archive({ name: 'web-01-c', repo_id: 2, repo_name: 'offsite' }),
    ]

    it('names each repository once, in list order, with its archive count', () => {
      expect(useArchiveList(ref(SPREAD)).repos.value).toEqual([
        { id: 1, name: 'nightly', count: 1 },
        { id: 2, name: 'offsite', count: 2 },
      ])
    })

    it('narrows to one repository', () => {
      const l = useArchiveList(ref(SPREAD))
      l.repoFilter.value = 2
      expect(l.ordered.value.map((a) => a.name).sort()).toEqual(['web-01-b', 'web-01-c'])
    })

    it('shows everything again when the picked repository leaves the list', () => {
      const archives = ref(SPREAD)
      const l = useArchiveList(archives)
      l.repoFilter.value = 2
      archives.value = SPREAD.filter((a) => a.repo_id === 1)
      expect(l.ordered.value.map((a) => a.name)).toEqual(['web-01-a'])
    })

    it('finds archives by repository name too', () => {
      const l = useArchiveList(ref(SPREAD))
      l.filter.value = 'OFFSITE'
      expect(l.ordered.value).toHaveLength(2)
    })

    it('forgets the repository on reset', () => {
      const l = useArchiveList(ref(SPREAD))
      l.repoFilter.value = 2
      l.reset()
      expect(l.repoFilter.value).toBeNull()
    })
  })

  describe('grouping', () => {
    it('groups one host by day and several by host, until the reader picks', () => {
      const oneHost = useArchiveList(ref(ARCHIVES.filter((a) => a.hostname === 'web-01')))
      expect(oneHost.groupMode.value).toBe('day')

      const twoHosts = list()
      expect(twoHosts.multiHost.value).toBe(true)
      expect(twoHosts.groupMode.value).toBe('host')
      twoHosts.groupMode.value = 'flat'
      expect(twoHosts.groupMode.value).toBe('flat')
    })

    it('never groups one host by host, even when the reader picked it', () => {
      const archives = ref(ARCHIVES)
      const l = useArchiveList(archives)
      l.groupMode.value = 'host'
      expect(l.groupMode.value).toBe('host')

      archives.value = ARCHIVES.filter((a) => a.hostname === 'web-01')
      expect(l.groupMode.value).toBe('day')
    })

    it('puts the archives under Today, Yesterday, This week, then one heading per month', () => {
      const now = Date.now()
      const ago = (days: number): string => new Date(now - days * 86_400_000).toISOString()
      const l = useArchiveList(
        ref([
          archive({ name: 'today', start: ago(0) }),
          archive({ name: 'yesterday', start: ago(1) }),
          archive({ name: 'week', start: ago(3) }),
          archive({ name: 'old', start: '2020-05-10T12:00:00Z' }),
          archive({ name: 'older', start: '2020-05-02T12:00:00Z' }),
        ]),
      )
      expect(l.dayGroups.value.map((g) => [g.key, g.archives.map((a) => a.name)])).toEqual([
        ['today', ['today']],
        ['yesterday', ['yesterday']],
        ['week', ['week']],
        ['month-2020-05', ['old', 'older']],
      ])
    })

    it('keeps the headings in date order when the list is sorted by size', () => {
      const now = Date.now()
      const ago = (days: number): string => new Date(now - days * 86_400_000).toISOString()
      const l = useArchiveList(
        ref([
          archive({ name: 'today', start: ago(0), original_size: 300 }),
          archive({ name: 'yesterday', start: ago(1), original_size: 100 }),
          archive({ name: 'old', start: '2020-05-10T12:00:00Z', original_size: 200 }),
        ]),
      )

      l.sortMode.value = 'size-asc'
      // Within a heading the archives keep the size order.
      expect(l.dayGroups.value.map((g) => g.key)).toEqual(['today', 'yesterday', 'month-2020-05'])

      l.sortMode.value = 'date-asc'
      expect(l.dayGroups.value.map((g) => g.key)).toEqual(['month-2020-05', 'yesterday', 'today'])
    })
  })
})
