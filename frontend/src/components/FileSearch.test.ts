// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, type MockInstance } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { mockApiClientRead, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'
import type { RepoArchive } from '../utils/repoArchive'

vi.mock('../api/client', () => mockApiClientRead())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())

import { apiClient } from '../api/client'
import FileSearch from './FileSearch.vue'

const mockGet = apiClient.get as MockInstance

function archive(name: string, start: string, repo = { id: 1, name: 'nas' }): RepoArchive {
  return {
    name,
    start,
    hostname: 'web-server-01',
    comment: '',
    original_size: 0,
    deduplicated_size: 0,
    matched: true,
    agent_hostname: 'web-server-01',
    repo_id: repo.id,
    repo_name: repo.name,
  }
}

const NEWER = archive('web-server-01-2026-05-30T12:00:00', '2026-05-30T12:00:00Z')
const OLDER = archive('web-server-01-2026-05-29T12:00:00', '2026-05-29T12:00:00Z')
const ARCHIVES = [NEWER, OLDER]

function hit(path: string, archiveName: string, overrides: Record<string, unknown> = {}) {
  return {
    path,
    size: 1024,
    mtime: '2026-05-30T10:00:00',
    type: '-',
    archive_name: archiveName,
    ...overrides,
  }
}

function answer(items: unknown[], searched = 2) {
  return { data: { items, total_archives_searched: searched, limit: 200, offset: 0 } }
}

async function mountSearch(query = 'nginx', archives: RepoArchive[] = ARCHIVES) {
  const wrapper = mount(FileSearch, { props: { query, archives } })
  await flushPromises()
  return wrapper
}

function hitTitles(wrapper: Awaited<ReturnType<typeof mountSearch>>): string[] {
  return wrapper
    .findAll('[data-testid="file-search-hit"]')
    .map((row) => row.get('.browser-row-title').text())
}

/** The line under the heading, its spacing evened out. */
function facts(wrapper: Awaited<ReturnType<typeof mountSearch>>): string {
  return wrapper.get('.file-search-facts').text().replace(/\s+/g, ' ')
}

describe('FileSearch', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockGet.mockResolvedValue(answer([]))
  })

  it('names what it searched for in its heading', async () => {
    const wrapper = await mountSearch('nginx')
    expect(wrapper.get('h2').text()).toBe('Files matching “nginx”')
  })

  it('searches for names containing plain text, through every archive of the list', async () => {
    await mountSearch('nginx')
    expect(mockGet).toHaveBeenCalledWith('/repos/1/search', {
      params: { pattern: '**/*nginx*', max_archives: 2 },
    })
  })

  it('passes a glob on as it is', async () => {
    await mountSearch('*.conf')
    expect(mockGet).toHaveBeenCalledWith('/repos/1/search', {
      params: { pattern: '*.conf', max_archives: 2 },
    })
  })

  it('shows a spinner while searching', async () => {
    mockGet.mockReturnValue(new Promise(() => undefined))
    const wrapper = await mountSearch()
    expect(wrapper.find('[role="status"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('Searching 2 archives...')
  })

  it('lists each match by name with its folder, archive and size', async () => {
    mockGet.mockResolvedValue(
      answer([
        hit('etc/nginx/nginx.conf', NEWER.name),
        hit('etc/nginx', NEWER.name, { type: 'd', size: 0 }),
      ]),
    )
    const wrapper = await mountSearch()

    expect(facts(wrapper)).toBe('2 paths · 2 archives searched · nas')
    expect(hitTitles(wrapper)).toEqual(['nginx', 'nginx.conf'])
    const [folder, file] = wrapper.findAll('[data-testid="file-search-hit"]')
    expect(file!.text()).toContain('/etc/nginx')
    expect(file!.text()).toContain(`newest in ${NEWER.name}`)
    expect(file!.get('.browser-row-size').text()).toBe('1.0 KB')
    // A folder has no size of its own.
    expect(folder!.find('.browser-row-size').exists()).toBe(false)
    expect(folder!.text()).toContain('/etc')
  })

  it('names each path once, in the newest archive that holds it', async () => {
    mockGet.mockResolvedValue(
      answer([hit('etc/hosts', OLDER.name), hit('etc/hosts', NEWER.name, { size: 2048 })]),
    )
    const wrapper = await mountSearch('hosts')

    expect(hitTitles(wrapper)).toEqual(['hosts'])
    expect(wrapper.text()).toContain(`newest in ${NEWER.name}`)
    expect(facts(wrapper)).toBe('1 path · 2 archives searched · nas')
  })

  it('leaves out matches in archives the list does not hold', async () => {
    mockGet.mockResolvedValue(
      answer([hit('etc/hosts', 'other-host-2026-05-30T12:00:00'), hit('etc/motd', NEWER.name)]),
    )
    const wrapper = await mountSearch('e')

    expect(hitTitles(wrapper)).toEqual(['motd'])
  })

  it('asks each repository of a list spanning several, and names the repository', async () => {
    const offsite = archive('web-server-01-2026-05-31T12:00:00', '2026-05-31T12:00:00Z', {
      id: 2,
      name: 'offsite',
    })
    mockGet.mockImplementation((url: string) =>
      Promise.resolve(
        url === '/repos/2/search'
          ? answer([hit('etc/hosts', offsite.name)], 1)
          : answer([hit('etc/hosts', NEWER.name), hit('etc/fstab', NEWER.name)], 2),
      ),
    )
    const wrapper = await mountSearch('e', [...ARCHIVES, offsite])

    expect(mockGet).toHaveBeenCalledWith('/repos/1/search', {
      params: { pattern: '**/*e*', max_archives: 2 },
    })
    expect(mockGet).toHaveBeenCalledWith('/repos/2/search', {
      params: { pattern: '**/*e*', max_archives: 1 },
    })
    expect(facts(wrapper)).toBe('2 paths · 3 archives searched')
    const rows = wrapper.findAll('[data-testid="file-search-hit"]')
    expect(rows.map((r) => r.get('.browser-row-title').text())).toEqual(['fstab', 'hosts'])
    // The newer copy is in the other repository.
    expect(rows[1]!.text()).toContain(`newest in ${offsite.name}`)
    expect(rows[1]!.text()).toContain('offsite')
    expect(rows[0]!.text()).toContain('nas')
  })

  it('opens a match in the archive it was found in', async () => {
    const entry = hit('etc/nginx/nginx.conf', NEWER.name)
    mockGet.mockResolvedValue(answer([entry]))
    const wrapper = await mountSearch()

    await wrapper.get('[data-testid="file-search-hit"]').trigger('click')
    expect(wrapper.emitted('open')).toEqual([[NEWER, entry]])
  })

  it('says when no file name matches', async () => {
    const wrapper = await mountSearch('*.xyz')

    expect(wrapper.text()).toContain('No file names contain “*.xyz”')
    expect(facts(wrapper)).toBe('0 paths · 2 archives searched · nas')
  })

  it('searches again when the query changes', async () => {
    const wrapper = await mountSearch('nginx')
    await wrapper.setProps({ query: 'hosts' })
    await flushPromises()

    expect(mockGet).toHaveBeenLastCalledWith('/repos/1/search', {
      params: { pattern: '**/*hosts*', max_archives: 2 },
    })
  })

  it('keeps only the answer to the latest query', async () => {
    let answerFirst: (value: unknown) => void = () => undefined
    mockGet
      .mockReturnValueOnce(new Promise((resolve) => (answerFirst = resolve)))
      .mockResolvedValueOnce(answer([hit('etc/hosts', NEWER.name)]))
    const wrapper = await mountSearch('nginx')
    await wrapper.setProps({ query: 'hosts' })
    await flushPromises()
    answerFirst(answer([hit('etc/nginx/nginx.conf', NEWER.name)]))
    await flushPromises()

    expect(hitTitles(wrapper)).toEqual(['hosts'])
  })

  it('shows the error when the search fails', async () => {
    mockGet.mockRejectedValue(new Error('Network failure'))
    const wrapper = await mountSearch()

    expect(wrapper.get('.error-banner').text()).toBe('Network failure')
  })

  it('closes from its close button', async () => {
    const wrapper = await mountSearch()
    await wrapper.get('button[aria-label="Close search"]').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
  })
})
