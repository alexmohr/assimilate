// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRead } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRead())
vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback: string) => fallback,
}))

import { renderWithPlugins } from '../test-utils'
import { findButton } from '../test-utils/dom'
import { apiClient } from '../api/client'
import DatabaseStorageView from './DatabaseStorageView.vue'

const mockGet = vi.mocked(apiClient.get)

const STORAGE = {
  database_bytes: 1073741824,
  other_bytes: 268435456,
  relations: [
    {
      table_name: 'archive_files',
      table_bytes: 536870912,
      index_bytes: 134217728,
      toast_bytes: 0,
      total_bytes: 671088640,
    },
    {
      table_name: 'backup_reports',
      table_bytes: 67108864,
      index_bytes: 67108864,
      toast_bytes: 0,
      total_bytes: 134217728,
    },
  ],
}

describe('DatabaseStorageView', () => {
  beforeEach(() => {
    mockGet.mockReset()
    mockGet.mockResolvedValue({ data: STORAGE })
  })

  it('renders database storage ordered by backend usage', async () => {
    const wrapper = renderWithPlugins(DatabaseStorageView)
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/system/database-storage')
    expect(wrapper.text()).toContain('Database Storage')
    expect(wrapper.text()).toContain('1.0 GB')
    expect(wrapper.text()).toContain('archive_files')
    expect(wrapper.text()).toContain('640.0 MB')
    expect(wrapper.text()).toContain('backup_reports')
    const names = wrapper.findAll('.storage-name').map((cell) => cell.text())
    expect(names).toEqual(['archive_files', 'backup_reports', 'Other PostgreSQL storage'])
  })

  it('shows each table its share of the whole database', async () => {
    const wrapper = renderWithPlugins(DatabaseStorageView)
    await flushPromises()

    const shares = wrapper.findAll('.storage-share-value').map((cell) => cell.text())
    expect(shares).toEqual(['62.5%', '12.5%', '25.0%'])
  })

  it('says why when the storage figures cannot be loaded', async () => {
    mockGet.mockRejectedValueOnce(new Error('boom'))
    const wrapper = renderWithPlugins(DatabaseStorageView)
    await flushPromises()

    expect(wrapper.text()).toContain('Failed to load database storage')
    expect(wrapper.text()).not.toContain('archive_files')
  })

  it('reloads the figures on Refresh', async () => {
    const wrapper = renderWithPlugins(DatabaseStorageView)
    await flushPromises()
    mockGet.mockResolvedValueOnce({
      data: { ...STORAGE, database_bytes: 2147483648 },
    })

    await findButton(wrapper, /^Refresh$/).trigger('click')
    await flushPromises()

    expect(mockGet).toHaveBeenCalledTimes(2)
    expect(wrapper.text()).toContain('2.0 GB')
  })
})
