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
    props: ['repoId', 'repoName', 'archives', 'loading', 'error', 'isAdmin', 'reload', 'selected'],
    emits: ['update:selected'],
    template:
      '<div class="stub-explorer" :data-repo="repoName"><span class="stub-count">{{ archives.length }}</span></div>',
    methods: {
      onArchiveDeleted: vi.fn(),
      onDataChanged: vi.fn(),
      onRepoIdle: vi.fn(),
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

  it('renders one archive section per repository, filtered to this agent', async () => {
    mockListRepoArchives.mockResolvedValue([
      archive({ name: 'bell-1' }),
      archive({ name: 'other-1', hostname: 'other-host', agent_hostname: 'other-host' }),
    ])
    const wrapper = mount()
    await flushPromises()

    expect(mockListRepoArchives).toHaveBeenCalledWith(1)
    const section = wrapper.find('.stub-explorer')
    expect(section.attributes('data-repo')).toBe('Inhouse Global')
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

  it('surfaces a load failure as the section error', async () => {
    mockListRepoArchives.mockRejectedValue(new Error('repo unreachable'))
    const wrapper = mount()
    await flushPromises()

    const explorer = wrapper.findComponent({ name: 'ArchiveExplorer' })
    expect(explorer.props('error')).toBe('repo unreachable')
    expect(explorer.props('loading')).toBe(false)
  })

  it('forwards ArchiveDeleted to the matching repo explorer only', async () => {
    mockListRepoArchives.mockResolvedValue([])
    const wrapper = mount({
      repos: [repo({ id: 1, name: 'Inhouse Global' }), repo({ id: 2, name: 'Photos Offsite' })],
    })
    await flushPromises()

    const explorers = wrapper.findAllComponents({ name: 'ArchiveExplorer' })
    const onDeleted1 = vi.spyOn(explorers[0]!.vm, 'onArchiveDeleted')
    const onDeleted2 = vi.spyOn(explorers[1]!.vm, 'onArchiveDeleted')

    wsHandlers.ArchiveDeleted({ repo_id: 2, archive_name: 'bell-1' })

    expect(onDeleted1).not.toHaveBeenCalled()
    expect(onDeleted2).toHaveBeenCalledWith('bell-1')
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
