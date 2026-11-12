// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { makeRestoreRun } from '../test-utils/restoreRun'
import { makeDatabaseRestoreRun } from '../test-utils/databaseDump'

import { pushWs, resetWsMock, setWsStatus } from '../test-utils/wsMock'
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))
vi.mock('../api/restores', () => ({
  listRestoreRuns: vi.fn(),
  getRestoreRun: vi.fn(),
  cancelRestoreRun: vi.fn(),
}))
const toastError = vi.fn()
vi.mock('../composables/useToast', () => ({ useToast: () => ({ error: toastError }) }))

import { cancelRestoreRun, getRestoreRun, listRestoreRuns } from '../api/restores'
import RestoreRunsPanel from './RestoreRunsPanel.vue'

async function mountPanel(): Promise<ReturnType<typeof mount>> {
  const wrapper = mount(RestoreRunsPanel)
  await flushPromises()
  return wrapper
}

/** Each row's closed summary: dot, host, archive and facts, time. */
function rows(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('[data-testid="restore-runs"] .log-row').map((r) => r.text())
}

/** Opens row `i`'s detail, which every row has, so a test can read or act on it. */
async function open(wrapper: ReturnType<typeof mount>, i: number): Promise<void> {
  const row = wrapper.findAll('.log-row')[i]!
  if (!row.classes().includes('log-row--open')) {
    await row.find('.log-row-summary').trigger('click')
  }
}

/** Row `i`'s status, as its open detail names it. */
async function statusOf(wrapper: ReturnType<typeof mount>, i: number): Promise<string> {
  await open(wrapper, i)
  const detail = wrapper.findAll('.log-row')[i]!.find('.log-row-detail')
  const terms = detail.findAll('dt').map((dt) => dt.text())
  return detail.findAll('dd')[terms.indexOf('Status')]!.text()
}

function dotOf(wrapper: ReturnType<typeof mount>, i: number): string[] {
  return wrapper.findAll('.log-row')[i]!.find('.status-dot').classes()
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
    expect(failed).toContain('etc/hosts')
    // A failed restore's facts line says why instead of where it went.
    expect(failed).toContain('Permission denied')
    expect(failed).toContain('by admin')
    expect(dotOf(wrapper, 0)).toContain('status-dot--danger')
    expect(whole).toContain('the whole archive')
    expect(whole).toContain('into /restore')
    expect(dotOf(wrapper, 1)).toContain('status-dot--success')
    // The title and subtitle clip; the whole of each is in its tooltip.
    const failedRow = wrapper.findAll('.log-row')[0]!
    expect(failedRow.find('[title="web-01-2026-05-30"]').exists()).toBe(true)

    // The detail holds the rest: status, repository, where it went, the error.
    expect(await statusOf(wrapper, 0)).toBe('Failed')
    const detail = wrapper.findAll('.log-row')[0]!.find('.log-row-detail').text()
    expect(detail).toContain('nas-daily')
    expect(detail).toContain('/restore')
    expect(wrapper.findAll('.log-row')[0]!.find('.error-pre').text()).toBe('Permission denied')
    expect(await statusOf(wrapper, 1)).toBe('Restored')
  })

  it('says what a database restore restored and where, with no target path', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([
      makeDatabaseRestoreRun({}, { status: 'success' }),
      makeDatabaseRestoreRun({ target: { mode: 'original' } }, { id: 'c' }),
    ])
    const wrapper = await mountPanel()

    const [copy, replaced] = rows(wrapper)
    expect(copy).toContain('orders into orders_restored on shop/orders-db')
    expect(copy).not.toContain('into null')
    expect(replaced).toContain('orders (replaced) on shop/orders-db')

    await open(wrapper, 0)
    const detail = wrapper.findAll('.log-row')[0]!.find('.log-row-detail')
    const terms = detail.findAll('dt').map((dt) => dt.text())
    expect(terms).toContain('Database')
    expect(terms).not.toContain('Into')
    expect(detail.text()).toContain('orders into orders_restored on shop/orders-db')
    expect(detail.text()).not.toContain('null')
  })

  it('keeps the list current from pushes', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([makeRestoreRun()])
    const now = [
      makeRestoreRun({ status: 'success' }),
      makeRestoreRun({ id: 'new', status: 'pending' }),
    ]
    vi.mocked(getRestoreRun).mockImplementation((id) =>
      Promise.resolve(now.find((run) => run.id === id)!),
    )
    const wrapper = await mountPanel()

    // A push names the restore and its status; the row is re-read.
    for (const run of now) pushWs('RestoreRunChanged', { id: run.id, status: run.status })
    await flushPromises()

    expect(rows(wrapper)).toHaveLength(2)
    expect(wrapper.findAll('.log-row')[0]!.attributes('data-restore-id')).toBe('new')
    expect(await statusOf(wrapper, 0)).toBe('Waiting for agent')
    expect(await statusOf(wrapper, 1)).toBe('Restored')
    expect(dotOf(wrapper, 1)).toContain('status-dot--success')
  })

  it('does not reopen a finished restore when an older re-read resolves last', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([makeRestoreRun({ status: 'pending' })])
    let resolveRunning: (run: ReturnType<typeof makeRestoreRun>) => void = () => {}
    vi.mocked(getRestoreRun)
      .mockReturnValueOnce(new Promise((resolve) => (resolveRunning = resolve)))
      .mockResolvedValueOnce(makeRestoreRun({ status: 'success' }))
    const wrapper = await mountPanel()

    pushWs('RestoreRunChanged', { id: makeRestoreRun().id, status: 'running' })
    pushWs('RestoreRunChanged', { id: makeRestoreRun().id, status: 'success' })
    await flushPromises()
    expect(await statusOf(wrapper, 0)).toBe('Restored')

    // The hand-over's re-read, read while the restore was still running.
    resolveRunning(makeRestoreRun({ status: 'running' }))
    await flushPromises()
    expect(await statusOf(wrapper, 0)).toBe('Restored')
    expect(dotOf(wrapper, 0)).toContain('status-dot--success')
  })

  it('keeps showing the list when a pushed restore cannot be re-read', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([makeRestoreRun()])
    vi.mocked(getRestoreRun).mockRejectedValue(new Error('not found'))
    const wrapper = await mountPanel()

    pushWs('RestoreRunChanged', { id: 'pruned', status: 'success' })
    await flushPromises()

    expect(rows(wrapper)).toHaveLength(1)
    expect(toastError).not.toHaveBeenCalled()
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

    // The running restore offers no cancel in its detail.
    await open(wrapper, 1)
    expect(wrapper.findAll('[data-testid="restore-cancel"]')).toHaveLength(0)

    await open(wrapper, 0)
    const buttons = wrapper.findAll('[data-testid="restore-cancel"]')
    expect(buttons).toHaveLength(1)
    await buttons[0]!.trigger('click')
    await flushPromises()

    expect(cancelRestoreRun).toHaveBeenCalledWith(makeRestoreRun().id)
    expect(await statusOf(wrapper, 0)).toBe('Cancelled')
    expect(wrapper.findAll('[data-testid="restore-cancel"]')).toHaveLength(0)
  })

  it('says why a cancel or the list failed', async () => {
    vi.mocked(listRestoreRuns).mockResolvedValue([makeRestoreRun({ status: 'pending' })])
    vi.mocked(cancelRestoreRun).mockRejectedValue(new Error('already running'))
    const wrapper = await mountPanel()

    await open(wrapper, 0)
    await wrapper.find('[data-testid="restore-cancel"]').trigger('click')
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
