// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { mockApiClientRw, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

import { apiClient } from '../api/client'
import RestoreView from './RestoreView.vue'

const mockGet = apiClient.get as MockInstance
const mockPost = apiClient.post as MockInstance

const REPOS = [{ id: 1, name: 'server-daily', enabled: true }]

const ARCHIVES = [
  {
    name: 'web-01-old',
    start: '2026-05-29T02:00:00',
    hostname: 'web-01',
    comment: '',
    original_size: 1,
    deduplicated_size: 1,
    matched: true,
    agent_hostname: 'web-01',
    agent_domain: null,
  },
  {
    name: 'web-01-new',
    start: '2026-05-30T02:00:00',
    hostname: 'web-01',
    comment: '',
    original_size: 1,
    deduplicated_size: 1,
    matched: true,
    agent_hostname: 'web-01',
    agent_domain: null,
  },
]

function entry(path: string, type: string, size = 0): Record<string, unknown> {
  return { path, type, size, mtime: '2026-05-30T01:00:00', mode: '' }
}

const CONTENTS: Record<string, Record<string, unknown>[]> = {
  '': [entry('etc', 'd'), entry('var', 'd')],
  etc: [entry('etc/hosts', '-', 10), entry('etc/fstab', '-', 20)],
}

/** Serves the demo repository; `overrides` answers chosen URLs differently. */
function serve(overrides: Record<string, () => Promise<unknown>> = {}): void {
  mockGet.mockImplementation((url: string, config?: { params?: Record<string, unknown> }) => {
    const override = overrides[url]
    if (override !== undefined) return override()
    if (url === '/repos') return Promise.resolve({ data: REPOS })
    if (url === '/repos/1/archives') return Promise.resolve({ data: ARCHIVES })
    if (url.endsWith('/contents')) {
      const dir = typeof config?.params?.path === 'string' ? config.params.path : ''
      return Promise.resolve({ data: { index_status: 'done', entries: CONTENTS[dir] ?? [] } })
    }
    if (url.endsWith('/tree-search')) {
      return Promise.resolve({
        data: { index_status: 'done', entries: [entry('etc/hosts', '-', 10)], truncated: false },
      })
    }
    if (url === '/repos/1/search') {
      return Promise.resolve({
        data: {
          items: [{ ...entry('etc/hosts', '-', 10), archive_name: 'web-01-old' }],
          total_archives_searched: 2,
          limit: 200,
          offset: 0,
        },
      })
    }
    return Promise.reject(new Error(`unexpected GET ${url}`))
  })
  mockPost.mockImplementation((_url: string, body: { paths: string[] }) =>
    Promise.resolve({
      data: {
        index_status: 'done',
        totals: body.paths.map((path) => ({ path, files: 2, bytes: 30 })),
      },
    }),
  )
}

let router: Router

async function mountView(query: Record<string, string> = {}) {
  router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/restore', component: { template: '<div />' } }],
  })
  await router.push({ path: '/restore', query })
  const wrapper = mount(RestoreView, {
    global: {
      plugins: [createPinia(), router],
      stubs: { Teleport: true },
    },
  })
  await flushPromises()
  return wrapper
}

type View = Awaited<ReturnType<typeof mountView>>

function row(wrapper: View, path: string) {
  return wrapper.get(`[role="treeitem"][data-path="${path}"]`)
}

function treePaths(wrapper: View): (string | undefined)[] {
  return wrapper.findAll('[role="treeitem"]').map((r) => r.attributes('data-path'))
}

function basketPaths(wrapper: View): (string | undefined)[] {
  return wrapper
    .get('[data-testid="restore-basket"]')
    .findAll('.basket-item')
    .map((i) => i.attributes('data-path'))
}

/** A button of the tree panel's header, by its label. */
function treeButton(wrapper: View, label: string) {
  const button = wrapper
    .get('.restore-tree-panel')
    .findAll('button')
    .find((b) => b.text() === label)
  if (button === undefined) throw new Error(`no ${label} button`)
  return button
}

/** Lets the search debounce run out, and whatever it started settle. */
async function settleSearch(): Promise<void> {
  await vi.advanceTimersByTimeAsync(400)
  await flushPromises()
}

function treeSearches(): unknown[] {
  return mockGet.mock.calls.filter(([url]) => String(url).endsWith('/tree-search'))
}

describe('RestoreView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    serve()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('opens the newest archive and keeps it in the URL', async () => {
    const wrapper = await mountView()

    const select = wrapper.get('[data-testid="restore-archive-select"]')
    expect((select.element as HTMLSelectElement).value).toBe('web-01-new')
    expect(router.currentRoute.value.query).toMatchObject({ repo: '1', archive: 'web-01-new' })
    expect(wrapper.findAll('[role="treeitem"]').map((r) => r.attributes('data-path'))).toEqual([
      'etc',
      'var',
    ])
  })

  it('opens the archive a link names', async () => {
    const wrapper = await mountView({ repo: '1', archive: 'web-01-old' })

    const select = wrapper.get('[data-testid="restore-archive-select"]')
    expect((select.element as HTMLSelectElement).value).toBe('web-01-old')
  })

  it('puts a ticked folder in the basket as one entry with its totals', async () => {
    const wrapper = await mountView()

    await row(wrapper, 'etc').get('input').trigger('click')
    await flushPromises()

    const basket = wrapper.get('[data-testid="restore-basket"]')
    expect(basket.findAll('.basket-item').map((i) => i.attributes('data-path'))).toEqual(['etc'])
    expect(basket.get('[data-testid="basket-total"]').text()).toContain('2 files')
  })

  it('splits a ticked folder when one file inside it is unticked', async () => {
    const wrapper = await mountView()
    await row(wrapper, 'etc').get('input').trigger('click')
    await row(wrapper, 'etc').get('.tree-chevron').trigger('click')
    await flushPromises()

    await row(wrapper, 'etc/fstab').get('input').trigger('click')
    await flushPromises()

    const basket = wrapper.get('[data-testid="restore-basket"]')
    expect(basket.findAll('.basket-item').map((i) => i.attributes('data-path'))).toEqual([
      'etc/hosts',
    ])
    expect(row(wrapper, 'etc').attributes('aria-checked')).toBe('mixed')
  })

  it('filters the tree to search matches as the user types', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()

    await wrapper.get('[data-testid="restore-search"]').setValue('hosts')
    await vi.advanceTimersByTimeAsync(400)
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/repos/1/archives/web-01-new/tree-search', {
      params: expect.objectContaining({ q: 'hosts' }),
      signal: expect.any(AbortSignal),
    })
    expect(wrapper.findAll('[role="treeitem"]').map((r) => r.attributes('data-path'))).toEqual([
      'etc',
      'etc/hosts',
    ])
    expect(row(wrapper, 'etc/hosts').find('mark').text()).toBe('hosts')
  })

  it('switches to the archive a cross-archive hit came from', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()

    await wrapper.get('[aria-label="Search all archives"]').trigger('click')
    await wrapper.get('[data-testid="restore-search"]').setValue('hosts')
    await vi.advanceTimersByTimeAsync(400)
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/repos/1/search', {
      params: { pattern: '**/*hosts*', max_archives: undefined },
    })
    await wrapper.get('.cross-hit').trigger('click')
    await vi.advanceTimersByTimeAsync(400)
    await flushPromises()

    const select = wrapper.get('[data-testid="restore-archive-select"]')
    expect((select.element as HTMLSelectElement).value).toBe('web-01-old')
    expect((wrapper.get('[data-testid="restore-search"]').element as HTMLInputElement).value).toBe(
      'etc/hosts',
    )
  })

  it('shows why the repositories could not be loaded', async () => {
    serve({ '/repos': () => Promise.reject(new Error('session expired')) })

    const wrapper = await mountView()

    expect(wrapper.get('.error-banner').text()).toBe('session expired')
  })

  it('shows why the archives could not be loaded', async () => {
    serve({ '/repos/1/archives': () => Promise.reject(new Error('repository locked')) })

    const wrapper = await mountView()

    expect(wrapper.get('.error-banner').text()).toBe('repository locked')
    expect(wrapper.find('[data-testid="restore-tree"]').exists()).toBe(false)
  })

  it('lists the archives of another repository when it is picked', async () => {
    const other = { ...ARCHIVES[0], name: 'db-01-new', hostname: 'db-01', agent_hostname: 'db-01' }
    serve({
      '/repos': () => Promise.resolve({ data: [...REPOS, { id: 2, name: 'db', enabled: true }] }),
      '/repos/2/archives': () => Promise.resolve({ data: [other] }),
    })
    const wrapper = await mountView()

    await wrapper.get('[data-testid="restore-repo-select"]').setValue('2')
    await flushPromises()

    const select = wrapper.get('[data-testid="restore-archive-select"]')
    expect((select.element as HTMLSelectElement).value).toBe('db-01-new')
    expect(router.currentRoute.value.query).toMatchObject({ repo: '2', archive: 'db-01-new' })
  })

  it('opens another archive picked from the list', async () => {
    const wrapper = await mountView()

    await wrapper.get('[data-testid="restore-archive-select"]').setValue('web-01-old')
    await flushPromises()

    expect(router.currentRoute.value.query).toMatchObject({ archive: 'web-01-old' })
    expect(mockGet).toHaveBeenCalledWith('/repos/1/archives/web-01-old/contents', {
      params: { limit: 200 },
    })
    expect(wrapper.get('#restore-tree-title').text()).toBe('web-01-old')
  })

  it('asks for a file name before searching every archive', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()

    await wrapper.get('[aria-label="Search all archives"]').trigger('click')
    await settleSearch()

    expect(wrapper.get('[data-testid="restore-cross-results"]').text()).toContain(
      'Type a file name to find the archives that hold it.',
    )
    expect(mockGet).not.toHaveBeenCalledWith('/repos/1/search', expect.anything())
  })

  it('shows why a search across archives failed', async () => {
    vi.useFakeTimers()
    serve({ '/repos/1/search': () => Promise.reject(new Error('borg timed out')) })
    const wrapper = await mountView()

    await wrapper.get('[aria-label="Search all archives"]').trigger('click')
    await wrapper.get('[data-testid="restore-search"]').setValue('hosts')
    await settleSearch()

    const results = wrapper.get('[data-testid="restore-cross-results"]')
    expect(results.get('.error-banner').text()).toBe('borg timed out')
    expect(results.find('.cross-hit').exists()).toBe(false)
  })

  it('lists a ticked folder it has not opened before unticking a match inside it', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()
    await wrapper.get('[data-testid="restore-search"]').setValue('hosts')
    await settleSearch()
    await row(wrapper, 'etc').get('input').trigger('click')
    await flushPromises()

    await row(wrapper, 'etc/hosts').get('input').trigger('click')
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/repos/1/archives/web-01-new/contents', {
      params: { path: 'etc', limit: 100_000 },
    })
    expect(basketPaths(wrapper)).toEqual(['etc/fstab'])
  })

  it('selects the whole archive, or only the matches while searching, and clears', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()

    await treeButton(wrapper, 'Select all').trigger('click')
    await flushPromises()
    expect(basketPaths(wrapper)).toEqual(['etc', 'var'])

    await treeButton(wrapper, 'Clear').trigger('click')
    expect(basketPaths(wrapper)).toEqual([])

    await wrapper.get('[data-testid="restore-search"]').setValue('hosts')
    await settleSearch()
    await treeButton(wrapper, 'Select matches').trigger('click')
    await flushPromises()
    expect(basketPaths(wrapper)).toEqual(['etc/hosts'])
  })

  it('takes an entry out of the basket', async () => {
    const wrapper = await mountView()
    await row(wrapper, 'etc').get('input').trigger('click')
    await flushPromises()

    await wrapper.get('[aria-label="Remove etc"]').trigger('click')

    expect(basketPaths(wrapper)).toEqual([])
    expect(row(wrapper, 'etc').attributes('aria-checked')).toBe('false')
  })

  it('narrows the search with filters and resets them', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()

    await wrapper.get('.filter-toggle').trigger('click')
    const filters = wrapper.get('.restore-filters')
    await filters.get('[role="radio"][aria-checked="false"]:nth-child(3)').trigger('click')
    const [minSize, maxSize] = filters.findAll('input[type="number"]')
    const [from, to] = filters.findAll('input[type="date"]')
    await minSize!.setValue('1')
    await maxSize!.setValue('2.5')
    await from!.setValue('2026-01-01')
    await to!.setValue('2026-01-31')
    await settleSearch()

    expect(mockGet).toHaveBeenLastCalledWith('/repos/1/archives/web-01-new/tree-search', {
      params: {
        q: undefined,
        kind: 'dir',
        min_size: 1024 * 1024,
        max_size: 2.5 * 1024 * 1024,
        modified_from: '2026-01-01',
        modified_to: '2026-01-31',
        limit: undefined,
      },
      signal: expect.any(AbortSignal),
    })
    expect(wrapper.find('.filter-badge').exists()).toBe(true)

    const reset = filters.findAll('button').find((b) => b.text() === 'Reset filters')!
    await reset.trigger('click')
    await settleSearch()

    expect(wrapper.find('.filter-badge').exists()).toBe(false)
    expect(reset.attributes('disabled')).toBeDefined()
    expect((minSize!.element as HTMLInputElement).value).toBe('')
    expect((from!.element as HTMLInputElement).value).toBe('')
    expect(treePaths(wrapper)).toEqual(['etc', 'var'])
  })

  it('drops a search still waiting on its debounce when the page is left', async () => {
    vi.useFakeTimers()
    const wrapper = await mountView()

    await wrapper.get('[data-testid="restore-search"]').setValue('hosts')
    wrapper.unmount()
    await settleSearch()

    expect(treeSearches()).toEqual([])
  })
})
