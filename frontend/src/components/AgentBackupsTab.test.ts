// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

const toastSuccess = vi.fn()
const toastError = vi.fn()

vi.mock('../api/client', () => ({
  apiClient: { delete: vi.fn() },
}))
vi.mock('../composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: toastError }),
}))

import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentBackupsTab from './AgentBackupsTab.vue'
import type { ReportRow } from '../types/report'

function makeReport(id: number, status: string): ReportRow {
  return {
    id,
    agent_id: 1,
    repo_id: 1,
    schedule_id: null,
    started_at: '2026-08-27T00:00:00Z',
    finished_at: '2026-08-27T00:05:00Z',
    status,
    original_size: 0,
    compressed_size: 0,
    deduplicated_size: 0,
    files_processed: 0,
    duration_secs: 0,
    error_message: status === 'failed' ? 'connection refused' : null,
    warnings: [],
    borg_version: null,
    archive_name: status === 'success' ? 'archive-1' : null,
    borg_command: null,
    hostname: 'db-server-01',
    repo_name: 'database-hourly',
    schedule_name: null,
  }
}

const REPORTS: ReportRow[] = [makeReport(1, 'success'), makeReport(2, 'failed'), makeReport(3, 'failed')]

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentBackupsTab, {
    props: {
      reports: REPORTS,
      filter: 'all',
      sortAscending: false,
      expandedReportId: null,
      highlightedArchiveName: undefined,
      pinnedReportId: null,
      hostname: 'db-server-01',
      domain: null,
      canCleanFailed: true,
      ...props,
    },
  })
}

function cleanButton(wrapper: ReturnType<typeof mount>) {
  return wrapper.findAll('button').find((b) => b.text().startsWith('Clean up failed'))
}

function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no dialog button labelled "${label}"`)
  return match
}

describe('AgentBackupsTab', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    toastSuccess.mockReset()
    toastError.mockReset()
    vi.mocked(apiClient.delete)
      .mockReset()
      .mockResolvedValue({ data: { deleted: 2 } } as never)
  })

  it('shows the failed count on the clean-up button', () => {
    const wrapper = mount()
    expect(cleanButton(wrapper)?.text()).toBe('Clean up failed (2)')
  })

  it('hides the button when there are no failed reports', () => {
    const wrapper = mount({ reports: [makeReport(1, 'success')] })
    expect(cleanButton(wrapper)).toBeUndefined()
  })

  it('hides the button for a non-admin viewer', () => {
    const wrapper = mount({ canCleanFailed: false })
    expect(cleanButton(wrapper)).toBeUndefined()
  })

  it('does not call the API when the button is merely clicked', async () => {
    const wrapper = mount()
    await cleanButton(wrapper)?.trigger('click')
    await flushPromises()
    expect(apiClient.delete).not.toHaveBeenCalled()
  })

  it('does nothing when the confirmation dialog is cancelled', async () => {
    const wrapper = mount()
    await cleanButton(wrapper)?.trigger('click')
    await flushPromises()

    dialogButton('Cancel').click()
    await flushPromises()

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })

  it('deletes the failed reports on confirmation and notifies the parent', async () => {
    const wrapper = mount()
    await cleanButton(wrapper)?.trigger('click')
    await flushPromises()

    dialogButton('Delete failed reports').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/db-server-01/reports/failed', {
      params: {},
    })
    expect(toastSuccess).toHaveBeenCalledWith('Deleted 2 failed backup reports.')
    expect(wrapper.emitted('cleaned')).toHaveLength(1)
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })

  it('reports a failure as an error toast and leaves the dialog open', async () => {
    vi.mocked(apiClient.delete).mockRejectedValueOnce(new Error('locked'))
    const wrapper = mount()
    await cleanButton(wrapper)?.trigger('click')
    await flushPromises()

    dialogButton('Delete failed reports').click()
    await flushPromises()

    expect(toastError).toHaveBeenCalled()
    expect(wrapper.emitted('cleaned')).toBeUndefined()
    expect(document.body.querySelector('.modal-dialog')).not.toBeNull()
  })
})
