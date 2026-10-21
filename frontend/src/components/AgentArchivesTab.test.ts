// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { mockWebSocket, resetWsHandlers, wsHandlers } from '../test-utils/sharedMocks'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { Repo } from '../types/repo'

const mockListRepoArchives = vi.fn()
vi.mock('../api/archives', () => ({
  listRepoArchives: (repoId: number) => mockListRepoArchives(repoId),
}))

vi.mock('../composables/useWebSocket', () => mockWebSocket())

vi.mock('./ArchiveExplorer.vue', () => ({
  default: {
    name: 'ArchiveExplorer',
    props: [
      'archives',
      'loading',
      'error',
      'isAdmin',
      'reload',
      'selected',
      'pathQueryKey',
      'archiveQueryKey',
    ],
    emits: ['update:selected'],
    template:
      '<div class="stub-explorer"><span class="stub-count">{{ archives.length }}</span></div>',
    methods: {
      onArchiveDeleted: vi.fn(),
      onDataChanged: vi.fn(),
      onRepoIdle: vi.fn(),
      resetList: vi.fn(),
    },
  },
}))

import AgentArchivesTab from './AgentArchivesTab.vue'

function archive(overrides: Partial<ArchiveEntry>): ArchiveEntry {
  return {
    name: 'bell-2026-08-30T20:00:01',
    start: '2026-08-30T20:00:01Z',
    hostname: 'bell',
    comment: '',
    original_size: 1000,
    deduplicated_size: 100,
    matched: true,
    agent_hostname: 'bell',
    agent_domain: null,
    ...overrides,
  }
}

function repo(overrides: Partial<Repo> = {}): Repo {
  return { id: 1, name: 'Inhouse Global', ...overrides } as unknown as Repo
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentArchivesTab, {
    props: {
      hostname: 'bell',
      repos: [repo()],
      isAdmin: true,
      ...props,
    },
  })
}

describe('AgentArchivesTab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetWsHandlers()
  })

  it('shows a message when the agent backs up to no repository yet', () => {
    const wrapper = mount({ repos: [] })
    expect(wrapper.text()).toContain('No repositories yet')
  })

  it("lists this agent's archives, each tagged with its repository", async () => {
    mockListRepoArchives.mockResolvedValue([
      archive({ name: 'bell-1' }),
      archive({ name: 'other-1', hostname: 'other-host', agent_hostname: 'other-host' }),
    ])
    const wrapper = mount()
    await flushPromises()

    expect(mockListRepoArchives).toHaveBeenCalledWith(1)
    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    expect(explorer.props('archives')).toEqual([
      expect.objectContaining({ name: 'bell-1', repo_id: 1, repo_name: 'Inhouse Global' }),
    ])
  })

  // One list for every repository, not one explorer per repository stacked
  // under the next - which on a phone pushed the second repository's
  // archives screens below the first's.
  it("puts every repository's archives into one explorer, in repository order", async () => {
    mockListRepoArchives.mockImplementation((repoId: number) =>
      Promise.resolve([archive({ name: `bell-in-${repoId}` })]),
    )
    const wrapper = mount({
      repos: [repo({ id: 2, name: 'Photos Offsite' }), repo({ id: 1, name: 'Inhouse Global' })],
    })
    await flushPromises()

    const explorers = wrapper.findAllComponents({ name: 'ArchiveExplorer' })
    expect(explorers).toHaveLength(1)
    expect(explorers[0]!.props('archives').map((a: { repo_name: string }) => a.repo_name)).toEqual([
      'Photos Offsite',
      'Inhouse Global',
    ])
  })

  // One explorer browses every repository, so it is the page's only browser
  // and keeps its archive and directory under the default query keys; the
  // per-repository keys were for one browser per repository section.
  it('keeps the one explorer under the default query keys', async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount({ repos: [repo({ id: 1 }), repo({ id: 2, name: 'Offsite' })] })
    await flushPromises()

    const explorers = wrapper.findAllComponents({ name: 'ArchiveExplorer' })
    expect(explorers).toHaveLength(1)
    expect(explorers[0]?.props('pathQueryKey')).toBeUndefined()
    expect(explorers[0]?.props('archiveQueryKey')).toBeUndefined()
  })

  // Regression test: two agents can share a hostname and be told apart only
  // by domain (`repos.spec.ts` e2e coverage). `listRepoArchives` returns
  // every archive in the repo regardless of which agent made it, so this tab
  // must filter by domain too, not just the ambiguous bare hostname, or one
  // agent's archives leak onto the other's page.
  it('disambiguates by domain when two agents share a hostname', async () => {
    mockListRepoArchives.mockResolvedValue([
      archive({ name: 'bell-dc1', agent_domain: 'dc1.example.com' }),
      archive({ name: 'bell-dc2', agent_domain: 'dc2.example.com' }),
    ])
    const wrapper = mount({ domain: 'dc1.example.com' })
    await flushPromises()

    const section = wrapper.find('.stub-explorer')
    expect(section.find('.stub-count').text()).toBe('1')
  })

  it('matches archives from an agent with no domain when none is given', async () => {
    mockListRepoArchives.mockResolvedValue([
      archive({ name: 'bell-none', agent_domain: null }),
      archive({ name: 'bell-dc1', agent_domain: 'dc1.example.com' }),
    ])
    const wrapper = mount()
    await flushPromises()

    const section = wrapper.find('.stub-explorer')
    expect(section.find('.stub-count').text()).toBe('1')
  })

  it('fetches archives for every repository the agent backs up to', async () => {
    mockListRepoArchives.mockResolvedValue([])
    mount({
      repos: [repo({ id: 1, name: 'Inhouse Global' }), repo({ id: 2, name: 'Photos Offsite' })],
    })
    await flushPromises()

    expect(mockListRepoArchives).toHaveBeenCalledWith(1)
    expect(mockListRepoArchives).toHaveBeenCalledWith(2)
  })

  it('reloads every section on DataChanged', async () => {
    mockListRepoArchives.mockResolvedValue([])
    mount()
    await flushPromises()
    mockListRepoArchives.mockClear()

    wsHandlers.DataChanged({})
    await flushPromises()

    expect(mockListRepoArchives).toHaveBeenCalledWith(1)
  })

  it('names the repository that failed to load, and still lists the others', async () => {
    mockListRepoArchives.mockImplementation((repoId: number) =>
      repoId === 1
        ? Promise.reject(new Error('repo unreachable'))
        : Promise.resolve([archive({ name: 'bell-offsite' })]),
    )
    const wrapper = mount({
      repos: [repo({ id: 1, name: 'Inhouse Global' }), repo({ id: 2, name: 'Photos Offsite' })],
    })
    await flushPromises()

    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    expect(explorer.props('error')).toBe('Inhouse Global: repo unreachable')
    expect(explorer.props('loading')).toBe(false)
    expect(explorer.props('archives')).toHaveLength(1)
  })

  it('drops a repository error once it loads again', async () => {
    mockListRepoArchives.mockRejectedValueOnce(new Error('repo unreachable'))
    const wrapper = mount()
    await flushPromises()
    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    expect(explorer.props('error')).toBe('Inhouse Global: repo unreachable')

    mockListRepoArchives.mockResolvedValue([archive({ name: 'bell-back' })])
    await explorer.props('reload')(true)
    await flushPromises()

    expect(explorer.props('error')).toBeNull()
    expect(explorer.props('archives')).toHaveLength(1)
  })

  it("forwards ArchiveDeleted for this agent's repositories only", async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount({
      repos: [repo({ id: 1, name: 'Inhouse Global' }), repo({ id: 2, name: 'Photos Offsite' })],
    })
    await flushPromises()

    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    const onDeleted = vi.spyOn(explorer.vm, 'onArchiveDeleted')

    wsHandlers.ArchiveDeleted({ repo_id: 2, archive_name: 'bell-1' })
    wsHandlers.ArchiveDeleted({ repo_id: 99, archive_name: 'bell-1' })

    expect(onDeleted).toHaveBeenCalledTimes(1)
    expect(onDeleted).toHaveBeenCalledWith('bell-1', 2)
  })

  it('forwards RepoOpChanged to the matching explorer, except for archive/compact ops', async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount()
    await flushPromises()

    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    const onIdle = vi.spyOn(explorer.vm, 'onRepoIdle')

    wsHandlers.RepoOpChanged({ repo_id: 1, op: { kind: 'delete_archive' } })
    expect(onIdle).not.toHaveBeenCalled()

    wsHandlers.RepoOpChanged({ repo_id: 1, op: { kind: 'compact_repo' } })
    expect(onIdle).not.toHaveBeenCalled()

    wsHandlers.RepoOpChanged({ repo_id: 1, op: null })
    expect(onIdle).toHaveBeenCalledTimes(1)
  })

  // Regression test: `AgentDetailView`'s background refresh (DataChanged,
  // reconnect, AgentDisconnected) reassigns `repos` to a brand new array on
  // every run even when this agent's actual repositories are unchanged. The
  // repo-list watcher used to fire on that reference change alone, wiping
  // every section's selection and flashing its loading state back on - on
  // top of the DataChanged handler right below it already doing a silent,
  // selection-preserving refresh. It must stay quiet unless the repo *set*
  // actually changed.
  it('does not reset sections when the repos prop is reassigned with the same ids', async () => {
    mockListRepoArchives.mockResolvedValue([archive({ name: 'bell-1' })])
    const wrapper = mount()
    await flushPromises()
    mockListRepoArchives.mockClear()

    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    await explorer.vm.$emit('update:selected', archive({ name: 'bell-1' }))
    expect(explorer.props('selected')).not.toBeNull()

    // A new array, same repo id - the shape of a background refetch.
    await wrapper.setProps({ repos: [repo({ id: 1, name: 'Inhouse Global' })] })
    await flushPromises()

    expect(mockListRepoArchives).not.toHaveBeenCalled()
    expect(wrapper.findComponent({ name: 'ArchiveExplorer' }).props('selected')).not.toBeNull()
  })

  it('reloads all sections when the set of repos actually changes', async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount({ repos: [repo({ id: 1, name: 'Inhouse Global' })] })
    await flushPromises()
    mockListRepoArchives.mockClear()

    await wrapper.setProps({
      repos: [repo({ id: 1, name: 'Inhouse Global' }), repo({ id: 2, name: 'Photos Offsite' })],
    })
    await flushPromises()

    expect(mockListRepoArchives).toHaveBeenCalledWith(1)
    expect(mockListRepoArchives).toHaveBeenCalledWith(2)
  })

  it("clears the list's search and filters when the set of repos changes, and only then", async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount({ repos: [repo({ id: 1, name: 'Inhouse Global' })] })
    await flushPromises()
    const resetList = vi.spyOn(wrapper.findComponent({ name: 'ArchiveExplorer' }).vm, 'resetList')

    await wrapper.setProps({ repos: [repo({ id: 1, name: 'Inhouse Global' })] })
    await flushPromises()
    expect(resetList).not.toHaveBeenCalled()

    await wrapper.setProps({
      repos: [repo({ id: 1, name: 'Inhouse Global' }), repo({ id: 2, name: 'Photos Offsite' })],
    })
    await flushPromises()
    expect(resetList).toHaveBeenCalledTimes(1)
  })

  it('the reload prop and selected v-model reach their own repo section', async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount()
    await flushPromises()
    mockListRepoArchives.mockClear()

    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    await explorer.props('reload')(true)
    expect(mockListRepoArchives).toHaveBeenCalledWith(1)

    const picked = archive({ name: 'bell-2' })
    await explorer.vm.$emit('update:selected', picked)
    expect(explorer.props('selected')).toEqual(picked)
  })
})
