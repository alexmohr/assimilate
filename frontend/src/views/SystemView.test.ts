// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises, type VueWrapper } from '@vue/test-utils'
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
  extractBlobError: async (_e: unknown, fallback: string): Promise<string> => fallback,
}))

import { apiClient } from '../api/client'

const mockGet = vi.mocked(apiClient.get)
const mockPut = vi.mocked(apiClient.put)
const mockPost = vi.mocked(apiClient.post)

const SSH_KEY = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA test-key'

function setupSuccessMocks(): void {
  mockGet.mockImplementation((url: string) => {
    if (url === '/system/ssh-public-key') {
      return Promise.resolve({ data: { public_key: SSH_KEY } })
    }
    if (url === '/system/settings') {
      return Promise.resolve({
        data: {
          timezone: 'Europe/Berlin',
          retention_days: 30,
          report_retention_days: 365,
          failed_report_retention_days: 365,
          system_event_retention_days: 90,
          borg_query_timeout_secs: 600,
        },
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

  it('renders Report Retention input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.find('#settings-report-retention').exists()).toBe(true)
  })

  it('populates report retention from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-report-retention')
    expect(input.element.value).toBe('365')
  })

  it('renders Failed Report Retention input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.find('#settings-failed-retention').exists()).toBe(true)
  })

  it('populates failed report retention from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-failed-retention')
    expect(input.element.value).toBe('365')
  })

  it('renders System Event Retention input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.find('#settings-event-retention').exists()).toBe(true)
  })

  it('populates system event retention from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-event-retention')
    expect(input.element.value).toBe('90')
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
          data: {
            timezone: 'UTC',
            retention_days: 7,
            report_retention_days: 0,
            failed_report_retention_days: 365,
            system_event_retention_days: 90,
            borg_query_timeout_secs: 300,
          },
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

  it('saves new retention values to API', async () => {
    setupSuccessMocks()
    mockPut.mockResolvedValue({
      data: {
        timezone: 'Europe/Berlin',
        retention_days: 30,
        report_retention_days: 180,
        failed_report_retention_days: 90,
        system_event_retention_days: 45,
        borg_query_timeout_secs: 600,
      },
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save')!
    await saveBtn.trigger('click')
    await flushPromises()
    expect(mockPut).toHaveBeenCalledWith('/system/settings', {
      retention_days: 30,
      report_retention_days: 365,
      failed_report_retention_days: 365,
      system_event_retention_days: 90,
      timezone: 'Europe/Berlin',
      borg_query_timeout_secs: 600,
    })
  })

  it('updates form values from save response', async () => {
    setupSuccessMocks()
    mockPut.mockResolvedValue({
      data: {
        timezone: 'America/New_York',
        retention_days: 14,
        report_retention_days: 0,
        failed_report_retention_days: 365,
        system_event_retention_days: 90,
        borg_query_timeout_secs: 120,
      },
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save')!
    await saveBtn.trigger('click')
    await flushPromises()
    const retentionInput = wrapper.find<HTMLInputElement>('#settings-retention')
    expect(retentionInput.element.value).toBe('14')
    const reportInput = wrapper.find<HTMLInputElement>('#settings-report-retention')
    expect(reportInput.element.value).toBe('0')
  })

  describe('config import', () => {
    const MOCK_IMPORT_RESULT = {
      hosts_created: 2,
      hosts_updated: 0,
      schedules_created: 3,
      repos_created: 1,
      repos_updated: 0,
      warnings: [],
    }

    async function selectFile(wrapper: VueWrapper, content: string, name: string): Promise<void> {
      const file = new File([content], name, { type: 'application/json' })
      const fileInput = wrapper.find<HTMLInputElement>('input[type="file"]')
      Object.defineProperty(fileInput.element, 'files', {
        value: [file],
        writable: false,
      })
      await fileInput.trigger('change')
      await flushPromises()
    }

    it('renders No file chosen initially', () => {
      setupSuccessMocks()
      const wrapper = renderWithPlugins(SystemView)
      expect(wrapper.text()).toContain('No file chosen')
    })

    it('shows filename after file selection', async () => {
      setupSuccessMocks()
      const wrapper = renderWithPlugins(SystemView)
      await flushPromises()
      await selectFile(wrapper, '{}', 'my-config.json')
      expect(wrapper.text()).toContain('my-config.json')
    })

    it('disables Import button when no file is selected', async () => {
      setupSuccessMocks()
      const wrapper = renderWithPlugins(SystemView)
      await flushPromises()
      const importBtn = wrapper.findAll('button').find((b) => b.text() === 'Import')!
      expect((importBtn.element as HTMLButtonElement).disabled).toBe(true)
    })

    async function selectAndImport(wrapper: VueWrapper): Promise<void> {
      await selectFile(
        wrapper,
        JSON.stringify({ version: 1, hosts: [], schedules: [], repos: [] }),
        'cfg.json',
      )
      const importBtn = wrapper.findAll('button').find((b) => b.text() === 'Import')!
      await importBtn.trigger('click')
    }

    it('calls API and shows result on successful import', async () => {
      setupSuccessMocks()
      mockPost.mockResolvedValue({ data: MOCK_IMPORT_RESULT })
      const wrapper = renderWithPlugins(SystemView)
      await flushPromises()
      await selectAndImport(wrapper)
      await flushPromises()
      expect(mockPost).toHaveBeenCalledWith('/config/import', {
        version: 1,
        hosts: [],
        schedules: [],
        repos: [],
      })
      expect(wrapper.text()).toContain('Hosts created: 2')
      expect(wrapper.text()).toContain('Schedules created: 3')
      expect(wrapper.text()).toContain('Repos created: 1')
      expect(wrapper.text()).toContain('Repos updated: 0')
    })

    it('shows error when import API fails', async () => {
      setupSuccessMocks()
      mockPost.mockRejectedValue(new Error('Network error'))
      const wrapper = renderWithPlugins(SystemView)
      await flushPromises()
      await selectAndImport(wrapper)
      await flushPromises()
      expect(wrapper.text()).toContain('Import failed')
    })
  })
})
