// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import GroupRepoPermissionsDialog from './GroupRepoPermissionsDialog.vue'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), put: vi.fn(), delete: vi.fn() },
}))

import { apiClient } from '../api/client'

const mockGet = apiClient.get as ReturnType<typeof vi.fn>
const mockPut = apiClient.put as ReturnType<typeof vi.fn>

const group = { id: 7, name: 'data-team', description: null, created_at: '2026-01-01T00:00:00Z' }

beforeEach(() => {
  vi.clearAllMocks()
  mockGet.mockImplementation((url: string) => {
    if (url === '/repos') {
      return Promise.resolve({
        data: [
          { id: 1, name: 'server-daily' },
          { id: 2, name: 'database-hourly' },
        ],
      })
    }
    return Promise.resolve({
      data: [{ group_id: 7, repo_id: 2, permissions: ['view', 'backup'] }],
    })
  })
  mockPut.mockImplementation((url: string, body: { permissions: string[] }) =>
    Promise.resolve({ data: { group_id: 7, repo_id: Number(url.split('/').pop()), ...body } }),
  )
})

async function render(): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(GroupRepoPermissionsDialog, {
    props: { open: true, group },
    attachTo: document.body,
  })
  await flushPromises()
  return wrapper
}

function checkbox(label: string): HTMLInputElement {
  const box = document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)
  if (!box) throw new Error(`no checkbox "${label}"`)
  return box
}

describe('GroupRepoPermissionsDialog', () => {
  it("shows the group's grant on each repository", async () => {
    const wrapper = await render()

    expect(mockGet).toHaveBeenCalledWith('/groups/7/permissions')
    expect(checkbox('View on database-hourly').checked).toBe(true)
    expect(checkbox('Backup on database-hourly').checked).toBe(true)
    expect(checkbox('Extract on database-hourly').checked).toBe(false)
    expect(checkbox('View on server-daily').checked).toBe(false)
    wrapper.unmount()
  })

  it('saves only the repositories whose permissions changed', async () => {
    const wrapper = await render()

    checkbox('View on server-daily').click()
    checkbox('Backup on database-hourly').click()
    const save = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent?.trim() === 'Save permissions',
    )
    save?.click()
    await flushPromises()

    expect(mockPut).toHaveBeenCalledTimes(2)
    expect(mockPut).toHaveBeenCalledWith('/groups/7/permissions/1', { permissions: ['view'] })
    expect(mockPut).toHaveBeenCalledWith('/groups/7/permissions/2', { permissions: ['view'] })
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('saves nothing when nothing changed', async () => {
    const wrapper = await render()

    const save = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent?.trim() === 'Save permissions',
    )
    save?.click()
    await flushPromises()

    expect(mockPut).not.toHaveBeenCalled()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })
})
