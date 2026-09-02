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
    props: ['repoId', 'repoName', 'archives', 'loading', 'error', 'isAdmin', 'reload'],
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
})
