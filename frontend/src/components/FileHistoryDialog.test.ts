// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import type {
  ArchiveEntryResponse,
  FileDiffResponse,
  FileHistoryResponse,
} from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), delete: vi.fn() },
}))

import { apiClient } from '../api/client'
import FileHistoryDialog from './FileHistoryDialog.vue'

const OPENED: ArchiveEntryResponse = {
  name: 'web-01-wed',
  start: '2026-10-15T03:00:00Z',
  hostname: 'web-01',
  comment: '',
  original_size: 0,
  deduplicated_size: 0,
  matched: true,
  agent_hostname: 'web-01',
  agent_domain: null,
}

function history(overrides: Partial<FileHistoryResponse> = {}): FileHistoryResponse {
  return {
    path: 'etc/nginx/nginx.conf',
    hostname: 'web-01',
    agent_online: true,
    entries: [
      {
        kind: 'version',
        archives: [
          { name: 'web-01-wed', start: '2026-10-15T03:00:00Z' },
          { name: 'web-01-tue', start: '2026-10-14T03:00:00Z' },
        ],
        size: 220,
        mtime: '2026-10-13T09:00:00',
      },
      {
        kind: 'version',
        archives: [{ name: 'web-01-mon', start: '2026-10-13T03:00:00Z' }],
        size: 200,
        mtime: '2026-10-01T09:00:00',
      },
      { kind: 'absent', archives: [{ name: 'web-01-sun', start: '2026-10-12T03:00:00Z' }] },
      { kind: 'not_indexed', archives: [{ name: 'web-01-sat', start: '2026-10-11T03:00:00Z' }] },
    ],
    ...overrides,
  }
}

const CHANGED: FileDiffResponse = {
  status: 'text',
  added: 1,
  removed: 1,
  hunks: [
    {
      base_start: 1,
      base_count: 1,
      target_start: 1,
      target_count: 1,
      lines: [
        {
          kind: 'removed',
          base_line: 1,
          target_line: null,
          segments: [{ text: 'worker_processes auto;', emphasized: false }],
        },
        {
          kind: 'added',
          base_line: null,
          target_line: 1,
          segments: [{ text: 'worker_processes 8;', emphasized: false }],
        },
      ],
    },
  ],
}

let wrapper: VueWrapper | null = null

/** Answers the history request with `response` and every diff with `diff`. */
function serve(response: FileHistoryResponse, diff: FileDiffResponse = CHANGED): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) =>
    Promise.resolve({ data: url.endsWith('/file-history') ? response : diff }),
  )
}

async function open(response: FileHistoryResponse = history()): Promise<VueWrapper> {
  serve(response)
  wrapper = mount(FileHistoryDialog, {
    props: { open: true, repoId: 3, archive: OPENED, path: 'etc/nginx/nginx.conf' },
    global: { stubs: { teleport: true } },
  })
  await flushPromises()
  return wrapper
}

function lastDiffCall(): unknown[] | undefined {
  return vi
    .mocked(apiClient.get)
    .mock.calls.filter(([url]) => !String(url).endsWith('/file-history'))
    .pop()
}

describe('FileHistoryDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.matchMedia = vi
      .fn()
      .mockReturnValue({ matches: true }) as unknown as typeof window.matchMedia
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    vi.useRealTimers()
  })

  it('lists every version, with the file now first', async () => {
    const dialog = await open()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/3/archives/web-01-wed/file-history', {
      params: { path: 'etc/nginx/nginx.conf' },
    })
    expect(dialog.find('[data-testid="file-history-live"]').text()).toContain(
      'over the encrypted agent channel',
    )
    expect(dialog.findAll('[data-testid="file-history-version"]')).toHaveLength(2)
    expect(dialog.find('[data-testid="file-history-version"]').text()).toContain('2 archives')
    expect(dialog.find('[data-testid="file-history-absent"]').text()).toContain(
      'Not in these archives',
    )
    expect(dialog.find('[data-testid="file-history-not_indexed"]').text()).toContain(
      'Not indexed yet',
    )
  })

  it('opens on the newest backup against the file now', async () => {
    const dialog = await open()

    expect(lastDiffCall()).toEqual([
      '/repos/3/archives/web-01-wed/file-diff-live',
      { params: { path: 'etc/nginx/nginx.conf', whole_file: undefined } },
    ])
    expect(dialog.get('[data-testid="file-history-to-label"]').text()).toBe('Now, on web-01')
    expect(dialog.get('[data-testid="file-history-stats"]').text()).toContain('-1')
    expect(dialog.find('[data-testid="file-diff-body"]').exists()).toBe(true)
  })

  it('compares two stored versions when picked', async () => {
    const dialog = await open()
    const [newest, older] = dialog.findAll('[data-testid="file-history-version"]')

    await older.findAll('button')[0].trigger('click')
    await newest.findAll('button')[1].trigger('click')
    await flushPromises()

    expect(lastDiffCall()).toEqual([
      '/repos/3/archives/web-01-wed/file-diff',
      expect.objectContaining({
        params: expect.objectContaining({ against: 'web-01-mon', path: 'etc/nginx/nginx.conf' }),
      }),
    ])
  })

  it('asks for two versions when one is picked as both sides', async () => {
    const dialog = await open()
    const [newest] = dialog.findAll('[data-testid="file-history-version"]')

    await newest.findAll('button')[0].trigger('click')
    await newest.findAll('button')[1].trigger('click')
    await flushPromises()

    expect(dialog.find('[data-testid="file-history-pick"]').exists()).toBe(true)
  })

  it('with the host offline, opens on the two newest backups', async () => {
    const dialog = await open(history({ agent_online: false }))

    expect(dialog.find('[data-testid="file-history-live"]').text()).toContain('web-01 is offline')
    expect(dialog.get('[data-testid="file-history-to-live"]').attributes('disabled')).toBeDefined()
    expect(lastDiffCall()).toEqual([
      '/repos/3/archives/web-01-wed/file-diff',
      expect.objectContaining({
        params: expect.objectContaining({ against: 'web-01-mon' }),
      }),
    ])
  })

  it('says why the file now could not be read', async () => {
    serve(history(), { status: 'live_unavailable', reason: 'not_in_sources' })
    wrapper = mount(FileHistoryDialog, {
      props: { open: true, repoId: 3, archive: OPENED, path: 'etc/nginx/nginx.conf' },
      global: { stubs: { teleport: true } },
    })
    await flushPromises()

    expect(wrapper.get('[data-testid="file-history-unavailable"]').text()).toContain(
      'outside every backup source configured for web-01',
    )
  })

  it('indexes the archives it could not look into, and reads the history again', async () => {
    vi.useFakeTimers()
    const dialog = await open()
    vi.mocked(apiClient.post).mockResolvedValue({ data: { queued: 1 } })
    serve(history({ entries: history().entries.slice(0, 3) }))

    await dialog.get('[data-testid="file-history-index"]').trigger('click')
    await flushPromises()
    expect(apiClient.post).toHaveBeenCalledWith('/repos/3/archives/web-01-wed/file-history/index')
    expect(dialog.get('[data-testid="file-history-index"]').text()).toBe('Indexing...')

    await vi.advanceTimersByTimeAsync(3000)
    await flushPromises()

    expect(dialog.find('[data-testid="file-history-not_indexed"]').exists()).toBe(false)
    expect(dialog.find('[data-testid="file-history-index"]').exists()).toBe(false)
  })

  it('offers the opened archive when the index cannot place it yet', async () => {
    const dialog = await open(
      history({
        entries: [
          {
            kind: 'not_indexed',
            archives: [{ name: 'web-01-wed', start: '2026-10-15T03:00:00Z' }],
          },
        ],
      }),
    )

    expect(dialog.text()).toContain('This archive, not indexed yet')
    expect(lastDiffCall()).toEqual([
      '/repos/3/archives/web-01-wed/file-diff-live',
      expect.anything(),
    ])
  })

  it('shows why the history could not be read', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('boom'))
    wrapper = mount(FileHistoryDialog, {
      props: { open: true, repoId: 3, archive: OPENED, path: 'etc/nginx/nginx.conf' },
      global: { stubs: { teleport: true } },
    })
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
  })
})
