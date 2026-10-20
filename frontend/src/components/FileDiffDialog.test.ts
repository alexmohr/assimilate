// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import type { ArchiveEntryResponse, FileDiffResponse } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), delete: vi.fn() },
}))

import { apiClient } from '../api/client'
import FileDiffDialog from './FileDiffDialog.vue'

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

const MONDAY = archive('web-01-mon', '2026-10-13T03:00:00Z')
const TUESDAY = archive('web-01-tue', '2026-10-14T03:00:00Z')
const WEDNESDAY = archive('web-01-wed', '2026-10-15T03:00:00Z')

const CHANGED: FileDiffResponse = {
  status: 'text',
  added: 1,
  removed: 1,
  hunks: [
    {
      base_start: 1,
      base_count: 2,
      target_start: 1,
      target_count: 2,
      lines: [
        {
          kind: 'context',
          base_line: 1,
          target_line: 1,
          segments: [{ text: 'user www;', emphasized: false }],
        },
        {
          kind: 'removed',
          base_line: 2,
          target_line: null,
          segments: [
            { text: 'worker_processes ', emphasized: false },
            { text: 'auto;', emphasized: true },
          ],
        },
        {
          kind: 'added',
          base_line: null,
          target_line: 2,
          segments: [
            { text: 'worker_processes ', emphasized: false },
            { text: '4;', emphasized: true },
          ],
        },
      ],
    },
  ],
}

let wrapper: VueWrapper | null = null

function wide(matches: boolean): void {
  window.matchMedia = vi.fn().mockReturnValue({ matches }) as unknown as typeof window.matchMedia
}

async function open(response: FileDiffResponse = CHANGED): Promise<VueWrapper> {
  vi.mocked(apiClient.get).mockResolvedValue({ data: response })
  wrapper = mount(FileDiffDialog, {
    props: {
      open: true,
      repoId: 3,
      archive: TUESDAY,
      archives: [MONDAY, TUESDAY, WEDNESDAY],
      path: 'etc/nginx/nginx.conf',
    },
    global: { stubs: { teleport: true } },
  })
  await flushPromises()
  return wrapper
}

function lastParams(): Record<string, unknown> {
  const call = vi.mocked(apiClient.get).mock.calls.at(-1)
  return (call?.[1] as { params: Record<string, unknown> }).params
}

describe('FileDiffDialog', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    wide(true)
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
  })

  it('compares against the archive before this one by default', async () => {
    const w = await open()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/3/archives/web-01-tue/file-diff', {
      params: {
        path: 'etc/nginx/nginx.conf',
        against: 'web-01-mon',
        against_repo_id: undefined,
        whole_file: undefined,
      },
    })
    const select = w.get<HTMLSelectElement>('[data-testid="file-diff-against"]')
    expect(select.element.value).toBe('web-01-mon')
    expect(w.text()).toContain('Compare nginx.conf')
  })

  it('offers every other archive, newest first', async () => {
    const w = await open()

    const options = w.findAll('[data-testid="file-diff-against"] option')
    expect(options.map((o) => o.attributes('value'))).toEqual(['web-01-wed', 'web-01-mon'])
  })

  it('shows the line counts and the changed lines with the changed words emphasized', async () => {
    const w = await open()

    expect(w.get('[data-testid="file-diff-stats"]').text()).toContain('-1')
    expect(w.get('[data-testid="file-diff-stats"]').text()).toContain('+1')
    const emphasized = w.findAll('.file-diff-em').map((e) => e.text())
    expect(emphasized).toEqual(['auto;', '4;'])
    expect(w.get('.file-diff-hunk').text()).toBe('Lines 1-2')
  })

  it('opens side by side when there is room, with the change on one row', async () => {
    const w = await open()

    expect(w.find('.file-diff-body--split').exists()).toBe(true)
    // The context line, then the removed/added pair on one row.
    expect(w.findAll('.file-diff-row')).toHaveLength(2)
  })

  it('opens inline on a narrow screen', async () => {
    wide(false)
    const w = await open()

    expect(w.find('.file-diff-body--inline').exists()).toBe(true)
    expect(w.findAll('.file-diff-line')).toHaveLength(3)
  })

  it('switches between the two layouts', async () => {
    const w = await open()

    await w.get('[role="radio"]:not([aria-checked="true"])').trigger('click')

    expect(w.find('.file-diff-body--inline').exists()).toBe(true)
  })

  it('asks again when another archive is picked', async () => {
    const w = await open()

    await w.get('[data-testid="file-diff-against"]').setValue('web-01-wed')
    await flushPromises()

    expect(lastParams().against).toBe('web-01-wed')
  })

  it('asks for the whole file when it is switched on', async () => {
    const w = await open()

    await w.get('[role="switch"]').trigger('click')
    await flushPromises()

    expect(lastParams().whole_file).toBe(true)
  })

  it('says when the two versions are the same', async () => {
    const w = await open({ status: 'text', added: 0, removed: 0, hunks: [] })

    expect(w.text()).toContain('No differences')
    expect(w.text()).toContain('nginx.conf is the same in both archives.')
  })

  it('says which version is not text', async () => {
    const w = await open({ status: 'binary', side: 'target' })

    expect(w.get('[data-testid="file-diff-unavailable"]').text()).toContain(
      'The version in this archive is not text',
    )
  })

  it('says which version is too large', async () => {
    const w = await open({ status: 'too_large', side: 'base', limit_bytes: 1048576 })

    expect(w.get('[data-testid="file-diff-unavailable"]').text()).toContain('larger than 1.0 MB')
  })

  it('shows the server error when the comparison fails', async () => {
    vi.mocked(apiClient.get).mockRejectedValue({
      isAxiosError: true,
      response: { status: 404, data: { error: 'path not found in archive' } },
    })
    wrapper = mount(FileDiffDialog, {
      props: {
        open: true,
        repoId: 3,
        archive: TUESDAY,
        archives: [MONDAY, TUESDAY],
        path: 'etc/nginx/nginx.conf',
      },
      global: { stubs: { teleport: true } },
    })
    await flushPromises()

    expect(wrapper.get('.error-banner').text()).toContain('path not found in archive')
  })

  it('closes on the dialog close', async () => {
    const w = await open()

    await w.get('button[aria-label="Close"]').trigger('click')

    expect(w.emitted('close')).toHaveLength(1)
  })
})
