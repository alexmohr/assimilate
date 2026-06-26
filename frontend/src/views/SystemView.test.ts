// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import SystemView from './SystemView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
    post: vi.fn(),
  },
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({
    copied: false,
    copy: vi.fn(),
  }),
}))

vi.mock('../composables/useTimezone', () => ({
  useTimezone: () => ({
    setTimezone: vi.fn(),
  }),
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback: string) => fallback,
}))

import { apiClient } from '../api/client'

const mockGet = vi.mocked(apiClient.get)

const SSH_KEY = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA test-key'

function setupSuccessMocks(): void {
  mockGet.mockImplementation((url: string) => {
    if (url === '/system/ssh-public-key') {
      return Promise.resolve({ data: { public_key: SSH_KEY } })
    }
    if (url === '/system/settings') {
      return Promise.resolve({
        data: { timezone: 'Europe/Berlin', retention_days: 30, borg_query_timeout_secs: 600 },
      })
    }
    if (url === '/system/version') {
      return Promise.resolve({
        data: {
          server_version: '0.1.0',
          server_git_sha: '',
          build_timestamp: '2026-06-06T10:00:00Z',
          agent_version: '0.1.0',
        },
      })
    }
    if (url === '/system/database-storage') {
      return Promise.resolve({
        data: {
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
        },
      })
    }
    return Promise.reject(new Error(`Unexpected GET ${url}`))
  })
}

describe('SystemView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders page title', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain('System')
  })

  it('renders SSH Public Key section heading', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain('SSH Public Key')
  })

  it('displays the SSH public key after loading', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain(SSH_KEY)
  })

  it('renders Copy button for the key', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain('Copy')
  })

  it('renders Regenerate button', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain('Regenerate')
  })

  it('renders Settings section heading', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain('Settings')
  })

  it('renders Retention Days input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.find('#settings-retention').exists()).toBe(true)
  })

  it('populates retention days from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-retention')
    expect(input.element.value).toBe('30')
  })

  it('renders Borg Timeout input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.find('#settings-borg-timeout').exists()).toBe(true)
  })

  it('populates borg timeout from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-borg-timeout')
    expect(input.element.value).toBe('600')
  })

  it('renders Save button for settings', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const buttons = wrapper.findAll('button')
    const saveBtn = buttons.find((b) => b.text() === 'Save')
    expect(saveBtn).toBeDefined()
  })

  it('renders database storage ordered by backend usage', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()

    expect(wrapper.text()).toContain('Database Storage')
    expect(wrapper.text()).toContain('1.0 GB')
    expect(wrapper.text()).toContain('archive_files')
    expect(wrapper.text()).toContain('640.0 MB')
    expect(wrapper.text()).toContain('backup_reports')
  })

  it('shows error message when SSH key API fails', async () => {
    mockGet.mockImplementation((url: string) => {
      if (url === '/system/ssh-public-key') {
        return Promise.reject(new Error('Network error'))
      }
      if (url === '/system/settings') {
        return Promise.resolve({
          data: { timezone: 'UTC', retention_days: 7, borg_query_timeout_secs: 300 },
        })
      }
      if (url === '/system/version') {
        return Promise.resolve({
          data: {
            server_version: '0.1.0',
            server_git_sha: '',
            build_timestamp: 'unknown',
            agent_version: null,
          },
        })
      }
      if (url === '/system/database-storage') {
        return Promise.resolve({ data: { database_bytes: 0, other_bytes: 0, relations: [] } })
      }
      return Promise.reject(new Error(`Unexpected GET ${url}`))
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.text()).toContain('Failed to load SSH public key')
  })

  it('opens regenerate confirmation dialog on button click', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const regenBtn = wrapper.findAll('button').find((b) => b.text() === 'Regenerate')
    expect(regenBtn).toBeDefined()
    await regenBtn!.trigger('click')
    await flushPromises()
    expect(document.body.textContent).toContain('Regenerate SSH Key')
  })
})
