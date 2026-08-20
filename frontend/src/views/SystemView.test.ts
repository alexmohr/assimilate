// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import SystemView from './SystemView.vue'
import TimezoneSelect from '../components/TimezoneSelect.vue'

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
          notification_delivery_retention_days: 30,
          borg_query_timeout_secs: 600,
          session_idle_timeout_minutes: 480,
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
    expect(wrapper.text()).toContain('SSH public key')
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

  it('renders Notification Delivery Retention input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    expect(wrapper.find('#settings-notification-delivery-retention').exists()).toBe(true)
  })

  it('populates notification delivery retention from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-notification-delivery-retention')
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

  it('renders Session Idle Timeout input', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find('#settings-idle-timeout')
    expect(input.exists()).toBe(true)
    expect((input.element as HTMLInputElement).value).toBe('480')
  })

  it('populates session idle timeout from API response', async () => {
    setupSuccessMocks()
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find('#settings-idle-timeout')
    expect((input.element as HTMLInputElement).value).toBe('480')
  })

  it('updates session idle timeout and persists it via save', async () => {
    setupSuccessMocks()
    mockPut.mockResolvedValue({
      data: {
        timezone: 'Europe/Berlin',
        retention_days: 30,
        report_retention_days: 365,
        failed_report_retention_days: 365,
        system_event_retention_days: 90,
        notification_delivery_retention_days: 30,
        borg_query_timeout_secs: 600,
        session_idle_timeout_minutes: 120,
      },
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    const input = wrapper.find<HTMLInputElement>('#settings-idle-timeout')
    await input.setValue('120')
    await wrapper.find('form.form-stack').trigger('submit')
    await flushPromises()
    expect(mockPut).toHaveBeenCalledWith(
      '/system/settings',
      expect.objectContaining({ session_idle_timeout_minutes: 120 }),
    )
  })

  // Every retention field was read back from the API in a test, but nothing
  // typed into one, so the v-model write path each field owns went unexercised
  // - a field wired to the wrong form key would still have passed.
  it('sends every edited retention field on save', async () => {
    setupSuccessMocks()
    mockPut.mockResolvedValue({
      data: {
        timezone: 'Europe/Berlin',
        retention_days: 14,
        report_retention_days: 180,
        failed_report_retention_days: 200,
        system_event_retention_days: 45,
        notification_delivery_retention_days: 15,
        borg_query_timeout_secs: 900,
        session_idle_timeout_minutes: 60,
      },
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()

    const edits: [string, string, string, number][] = [
      ['#settings-retention', '14', 'retention_days', 14],
      ['#settings-report-retention', '180', 'report_retention_days', 180],
      ['#settings-failed-retention', '200', 'failed_report_retention_days', 200],
      ['#settings-event-retention', '45', 'system_event_retention_days', 45],
      [
        '#settings-notification-delivery-retention',
        '15',
        'notification_delivery_retention_days',
        15,
      ],
      ['#settings-borg-timeout', '900', 'borg_query_timeout_secs', 900],
    ]

    for (const [selector, typed] of edits) {
      await wrapper.find<HTMLInputElement>(selector).setValue(typed)
    }

    // The timezone is a component rather than an input, so it writes back
    // through its model event.
    await wrapper.findComponent(TimezoneSelect).vm.$emit('update:modelValue', 'Europe/Berlin')
    await flushPromises()

    await wrapper.find('form.form-stack').trigger('submit')
    await flushPromises()

    expect(mockPut).toHaveBeenCalledWith(
      '/system/settings',
      expect.objectContaining({
        ...Object.fromEntries(edits.map(([, , key, value]) => [key, value])),
        timezone: 'Europe/Berlin',
      }),
    )
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

    expect(wrapper.text()).toContain('Database storage')
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
            notification_delivery_retention_days: 30,
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
    expect(document.body.textContent).toContain('Regenerate SSH key')
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
        notification_delivery_retention_days: 15,
        borg_query_timeout_secs: 600,
        session_idle_timeout_minutes: 480,
      },
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    await wrapper.find('form.form-stack').trigger('submit')
    await flushPromises()
    expect(mockPut).toHaveBeenCalledWith('/system/settings', {
      retention_days: 30,
      report_retention_days: 365,
      failed_report_retention_days: 365,
      system_event_retention_days: 90,
      notification_delivery_retention_days: 30,
      timezone: 'Europe/Berlin',
      borg_query_timeout_secs: 600,
      session_idle_timeout_minutes: 480,
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
        notification_delivery_retention_days: 30,
        borg_query_timeout_secs: 120,
      },
    })
    const wrapper = renderWithPlugins(SystemView)
    await flushPromises()
    await wrapper.find('form.form-stack').trigger('submit')
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

  describe('destructive confirmations', () => {
    async function render() {
      const wrapper = renderWithPlugins(SystemView)
      await flushPromises()
      return wrapper
    }

    function button(wrapper: ReturnType<typeof renderWithPlugins>, label: string) {
      const match = wrapper.findAll('button').find((b) => b.text().trim() === label)
      if (!match) throw new Error(`no button labelled "${label}"`)
      return match
    }

    // Both actions are irreversible from the UI's point of view, so the row
    // button only opens a dialog - it must never fire the request itself.
    it.each([
      ['Regenerate', 'Regenerate SSH key'],
      ['Reset', 'Reset system state'],
    ])('opens a confirmation for %s rather than acting immediately', async (trigger, title) => {
      const wrapper = await render()
      mockPost.mockClear()

      await button(wrapper, trigger).trigger('click')
      await flushPromises()

      expect(wrapper.text()).toContain(title)
      expect(mockPost).not.toHaveBeenCalled()
    })

    // Backing out has to be a genuine no-op by either route: the Cancel button
    // and BaseModal's own close event (Escape, backdrop) are wired separately.
    it.each([
      ['Regenerate', 'Cancel'],
      ['Reset', 'Cancel'],
      ['Regenerate', 'dismissal'],
      ['Reset', 'dismissal'],
    ])('does nothing when the %s dialog is closed by %s', async (trigger, how) => {
      const wrapper = await render()
      await button(wrapper, trigger).trigger('click')
      await flushPromises()
      mockPost.mockClear()

      if (how === 'Cancel') {
        await button(wrapper, 'Cancel').trigger('click')
        await flushPromises()
      } else {
        await dismissModal(wrapper)
      }

      expect(mockPost).not.toHaveBeenCalled()
      expect(openModals(wrapper)).toHaveLength(0)
    })

    it('spells out what a system reset will do before asking to confirm it', async () => {
      const wrapper = await render()
      await button(wrapper, 'Reset').trigger('click')
      await flushPromises()

      const text = wrapper.text()
      expect(text).toContain('Cancel all running and pending backup operations')
      // The reassurance matters as much as the warning: an operator needs to
      // know this is not going to unschedule their backups.
      expect(text).toContain('Schedules are left unchanged.')
    })
  })
})
