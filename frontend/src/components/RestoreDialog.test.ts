// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import type { AgentResponse, ArchiveEntryResponse } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), delete: vi.fn() },
}))

import { apiClient } from '../api/client'
import RestoreDialog from './RestoreDialog.vue'

const ARCHIVE: ArchiveEntryResponse = {
  name: 'web-server-01-backup-2026-06-05T02:00:00',
  start: '2026-06-05T02:00:00Z',
  hostname: 'web-server-01',
  comment: '',
  original_size: 2048,
  deduplicated_size: 1024,
  matched: true,
  agent_hostname: 'web-server-01',
  agent_domain: null,
}

const RESTORE_URL = '/repos/5/archives/web-server-01-backup-2026-06-05T02%3A00%3A00/restore'

function agent(
  id: number,
  hostname: string,
  overrides: Partial<AgentResponse> = {},
): AgentResponse {
  return {
    id,
    hostname,
    display_name: null,
    domain: null,
    is_connected: true,
    is_hidden: false,
    ...overrides,
  } as AgentResponse
}

const AGENTS = [
  agent(2, 'db-server-01', { is_connected: false }),
  agent(1, 'web-server-01'),
  agent(3, 'media-store-01'),
]

let wrapper: VueWrapper | null = null

async function open(paths: string[] = ['etc/nginx/nginx.conf']): Promise<VueWrapper> {
  vi.mocked(apiClient.get).mockResolvedValue({ data: AGENTS })
  wrapper = mount(RestoreDialog, {
    props: { open: true, repoId: 5, archive: ARCHIVE, paths },
    global: { stubs: { teleport: true } },
  })
  await flushPromises()
  return wrapper
}

async function submit(w: VueWrapper): Promise<void> {
  await w.get('form').trigger('submit')
  await flushPromises()
}

describe('RestoreDialog', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
  })

  it('opens on the machine the archive came from, listed first', async () => {
    const w = await open()

    const names = w.findAll('input[name="restore-agent"]').map((i) => i.element as HTMLInputElement)
    expect(names[0]?.checked).toBe(true)
    expect(w.findAll('.restore-choice-name')[0]?.text()).toBe('web-server-01')
    expect(w.text()).toContain('the backup came from here')
    expect(w.text()).toContain('starts when it next connects')
  })

  it('restores the picked paths to their original locations on the matched host', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { id: 'run-1', status: 'pending' } })
    const w = await open()

    await submit(w)

    expect(apiClient.post).toHaveBeenCalledWith(RESTORE_URL, {
      paths: ['etc/nginx/nginx.conf'],
      target_path: '/',
      hostname: 'web-server-01',
      domain: null,
    })
    expect(w.emitted('restored')?.[0]).toEqual([{ id: 'run-1', status: 'pending' }])
    expect(w.emitted('close')).toHaveLength(1)
  })

  it('restores the whole archive when no path is picked', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { id: 'run-2' } })
    const w = await open([])

    expect(w.get('[data-testid="restore-preview"]').text()).toContain(
      'every path back where it was backed up from',
    )
    await submit(w)

    expect(apiClient.post).toHaveBeenCalledWith(
      RESTORE_URL,
      expect.objectContaining({ paths: [], target_path: '/' }),
    )
  })

  it('restores into a folder on another agent, and says where each path lands', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { id: 'run-3' } })
    const w = await open(['etc/nginx/nginx.conf', 'home/sam/notes.md'])

    await w.get('input[name="restore-agent"][value="3"]').setValue(true)
    await w.get('input[name="restore-location"][value="folder"]').setValue(true)
    await w.get('#restore-folder').setValue('/srv/restore/')

    expect(w.get('[data-testid="restore-preview"]').text()).toContain(
      '/srv/restore/etc/nginx/nginx.conf',
    )
    expect(w.get('[data-testid="restore-submit"]').text()).toBe('Restore onto media-store-01')
    await submit(w)

    expect(apiClient.post).toHaveBeenCalledWith(RESTORE_URL, {
      paths: ['etc/nginx/nginx.conf', 'home/sam/notes.md'],
      target_path: '/srv/restore',
      hostname: 'media-store-01',
      domain: null,
    })
  })

  it('will not start with a folder that is not an absolute path', async () => {
    const w = await open()
    await w.get('input[name="restore-location"][value="folder"]').setValue(true)
    await w.get('#restore-folder').setValue('srv/restore')

    expect(w.text()).toContain('Use an absolute path')
    expect(w.get('[data-testid="restore-submit"]').attributes('disabled')).toBeDefined()
    await submit(w)
    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('needs an agent picked when no agent claims the archive', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: AGENTS })
    wrapper = mount(RestoreDialog, {
      props: {
        open: true,
        repoId: 5,
        archive: { ...ARCHIVE, matched: false, hostname: 'old-host' },
        paths: [],
      },
      global: { stubs: { teleport: true } },
    })
    await flushPromises()

    expect(wrapper.get('[data-testid="restore-submit"]').text()).toBe('Pick an agent')
    expect(wrapper.get('[data-testid="restore-submit"]').attributes('disabled')).toBeDefined()
  })

  it('shows why the restore could not be recorded, and stays open', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('restore failed'))
    const w = await open()

    await submit(w)

    expect(w.get('.form-error').text()).toContain('restore failed')
    expect(w.emitted('close')).toBeUndefined()
  })
})
