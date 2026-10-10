// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { makeRestoreRun } from '../test-utils/restoreRun'

import { pushWs, resetWsMock, setWsStatus } from '../test-utils/wsMock'
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))
vi.mock('../api/restores', () => ({ listRestoreRuns: vi.fn(), cancelRestoreRun: vi.fn() }))
const toastError = vi.fn()
vi.mock('../composables/useToast', () => ({ useToast: () => ({ error: toastError }) }))

import { cancelRestoreRun, listRestoreRuns } from '../api/restores'
import RestoreRunsPanel from './RestoreRunsPanel.vue'

async function mountPanel(): Promise<ReturnType<typeof mount>> {
  const wrapper = mount(RestoreRunsPanel)
  await flushPromises()
  return wrapper
}

function rows(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('tbody tr').map((r) => r.text())
}

describe('RestoreRunsPanel', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    resetWsMock()
  })

  it('says when there are no restores', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([])
    const wrapper = await mountPanel()

    expect(wrapper.text()).toContain('No restores')
  })

  it('lists each restore with what it restores, where and how it went', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([
      makeRestoreRun({ status: 'failed', error_message: 'Permission denied' }),
      makeRestoreRun({ id: 'b', paths: [], status: 'success' }),
    ])
    const wrapper = await mountPanel()

    const [failed, whole] = rows(wrapper)
    expect(failed).toContain('web-01')
    expect(failed).toContain('web-01-2026-05-30')
    expect(failed).toContain('nas-daily')
    expect(failed).toContain('etc/hosts')
    expect(failed).toContain('/restore')
    expect(failed).toContain('Failed')
    expect(failed).toContain('Permission denied')
    // The cells clip long text; the whole of it is in their titles.
    const failedRow = wrapper.findAll('tbody tr')[0]!
    expect(failedRow.find('[title="Permission denied"]').exists()).toBe(true)
    expect(failedRow.find('[title="etc/hosts into /restore"]').exists()).toBe(true)
    expect(whole).toContain('the whole archive')
    expect(whole).toContain('Restored')
  })

  it('keeps the list current from pushes', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([makeRestoreRun()])
    const wrapper = await mountPanel()

    pushWs('RestoreRunChanged', { run: makeRestoreRun({ status: 'success' }) })
    pushWs('RestoreRunChanged', { run: makeRestoreRun({ id: 'new', status: 'pending' }) })
    await flushPromises()

    const [newest, updated] = rows(wrapper)
    expect(newest).toContain('Waiting for agent')
    expect(updated).toContain('Restored')
    expect(rows(wrapper)).toHaveLength(2)
  })

  it('reloads when the UI WebSocket reconnects', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([])
    await mountPanel()

    setWsStatus('reconnecting')
    await flushPromises()
    setWsStatus('connected')
    await flushPromises()

    expect(listRestoreRuns).toHaveBeenCalledTimes(2)
  })

  it('cancels only a waiting restore', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([
      makeRestoreRun({ status: 'pending' }),
      makeRestoreRun({ id: 'b' }),
    ])
    vi.mocked(cancelRestoreRun).mockResolvedValue(makeRestoreRun({ status: 'cancelled' }))
    const wrapper = await mountPanel()

    const buttons = wrapper.findAll('button')
    expect(buttons).toHaveLength(1)
    await buttons[0]!.trigger('click')
    await flushPromises()

    expect(cancelRestoreRun).toHaveBeenCalledWith(makeRestoreRun().id)
    expect(rows(wrapper)[0]).toContain('Cancelled')
    expect(wrapper.findAll('button')).toHaveLength(0)
  })

  it('says why a cancel or the list failed', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([makeRestoreRun({ status: 'pending' })])
    vi.mocked(cancelRestoreRun).mockRejectedValue(new Error('already running'))
    const wrapper = await mountPanel()

    await wrapper.find('button').trigger('click')
    await flushPromises()
    expect(toastError).toHaveBeenCalledWith('already running')

    vi.mocked(listRestoreRuns).mockRejectedValue(new Error('server down'))
    setWsStatus('reconnecting')
    await flushPromises()
    setWsStatus('connected')
    await flushPromises()
    expect(toastError).toHaveBeenCalledWith('server down')
  })
})
