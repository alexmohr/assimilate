// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { pushWs, resetWsMock } from '../test-utils/wsMock'
import { makeDatabaseDump, makeDatabaseRestoreRun } from '../test-utils/databaseDump'
import type { ArchiveDatabaseDump, RestoreRun } from '../types/generated'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn() },
}))
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

import { apiClient } from '../api/client'
import BaseModal from './BaseModal.vue'
import DatabaseRestoreDialog from './DatabaseRestoreDialog.vue'

const ARCHIVE = 'db-01-2026-10-01T02:00'
const URL = '/repos/4/archives/db-01-2026-10-01T02%3A00/databases/restore'

/** How the restore now stands, as the API reads it back. */
let current: RestoreRun

async function mountDialog(
  dump: ArchiveDatabaseDump | null = makeDatabaseDump(),
  defaultDomain: string | null = 'lan',
) {
  const wrapper = renderWithPlugins(DatabaseRestoreDialog, {
    props: {
      open: true,
      repoId: 4,
      archiveName: ARCHIVE,
      dump,
      defaultHost: 'db-01',
      defaultDomain,
    },
  })
  await flushPromises()
  return wrapper
}

type Wrapper = Awaited<ReturnType<typeof mountDialog>>

function submit(wrapper: Wrapper) {
  return wrapper.find<HTMLButtonElement>('[data-testid="db-restore-submit"]')
}

function button(wrapper: Wrapper, label: string) {
  return wrapper.findAll('button').find((b) => b.text() === label)
}

describe('DatabaseRestoreDialog', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    resetWsMock()
    current = makeDatabaseRestoreRun()
    vi.mocked(apiClient.get).mockImplementation(() => Promise.resolve({ data: current } as never))
    vi.mocked(apiClient.post).mockImplementation(() =>
      Promise.resolve({ data: makeDatabaseRestoreRun() } as never),
    )
  })

  it('restores into a new database on the archive host by default', async () => {
    const wrapper = await mountDialog()

    expect(wrapper.find('dl').text()).toContain('orders')
    expect(wrapper.find('dl').text()).toContain('shop/orders-db')
    expect(wrapper.find<HTMLInputElement>('#db-restore-host').element.value).toBe('db-01')
    expect(wrapper.find<HTMLInputElement>('#db-restore-name').element.value).toBe('orders_restored')
    expect(wrapper.text()).toContain('Created next to orders, which is left as it is.')
    expect(submit(wrapper).text()).toBe('Restore')
    expect(submit(wrapper).element.disabled).toBe(false)
  })

  it('says why a new database name is refused and will not submit it', async () => {
    const wrapper = await mountDialog()
    await wrapper.find('#db-restore-name').setValue('pg_orders')

    expect(wrapper.find('[data-testid="db-restore-name-problem"]').text()).toBe(
      'Names starting with pg_ are reserved by PostgreSQL.',
    )
    expect(wrapper.find('#db-restore-name').attributes('aria-invalid')).toBe('true')
    expect(submit(wrapper).element.disabled).toBe(true)
  })

  it('needs a host to restore with', async () => {
    const wrapper = await mountDialog()
    await wrapper.find('#db-restore-host').setValue('  ')

    expect(submit(wrapper).element.disabled).toBe(true)
  })

  it('sends the new database and follows the restore until it is over', async () => {
    const wrapper = await mountDialog()
    await wrapper.find('#db-restore-name').setValue('orders_copy')
    await submit(wrapper).trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(URL, {
      archive_path: 'var/lib/assimilate/database-dumps/shop/orders-db/orders.dump',
      target: { mode: 'new_database', name: 'orders_copy' },
      hostname: 'db-01',
      domain: 'lan',
    })
    const status = wrapper.find('[data-testid="db-restore-status"]')
    expect(status.text()).toContain('Restoring')
    expect(status.text()).toContain('Restoring orders into orders_restored on shop/orders-db')

    current = makeDatabaseRestoreRun({}, { status: 'success' })
    pushWs('RestoreRunChanged', { id: current.id, status: current.status })
    await flushPromises()

    expect(status.text()).toContain(
      'Restored orders into orders_restored on shop/orders-db with db-01.',
    )
    await button(wrapper, 'Done')!.trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('names another host bare, without the archive host domain', async () => {
    const wrapper = await mountDialog()
    await wrapper.find('#db-restore-host').setValue(' db-02 ')
    await submit(wrapper).trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      URL,
      expect.objectContaining({ hostname: 'db-02', domain: null }),
    )
  })

  it('replaces the original only once its name is typed', async () => {
    const wrapper = await mountDialog()
    await button(wrapper, 'Replace orders')!.trigger('click')

    expect(wrapper.find('#db-restore-name').exists()).toBe(false)
    expect(wrapper.find('[data-testid="db-restore-replace-warning"]').text()).toContain(
      'dropped from orders and recreated from the archive, in one transaction',
    )
    expect(submit(wrapper).text()).toBe('Replace orders')
    expect(submit(wrapper).classes()).toContain('btn-danger')
    expect(submit(wrapper).element.disabled).toBe(true)

    await wrapper.find('#db-restore-confirm').setValue('order')
    expect(submit(wrapper).element.disabled).toBe(true)
    await wrapper.find('#db-restore-confirm').setValue('orders')
    expect(submit(wrapper).element.disabled).toBe(false)

    await submit(wrapper).trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(URL, {
      archive_path: 'var/lib/assimilate/database-dumps/shop/orders-db/orders.dump',
      target: { mode: 'original' },
      hostname: 'db-01',
      domain: 'lan',
    })
  })

  it('shows why the server refused the restore', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(
      Object.assign(new Error('Request failed'), {
        isAxiosError: true,
        response: { status: 400, data: { error: 'docker is not turned on for db-01' } },
      }),
    )
    const wrapper = await mountDialog()
    await submit(wrapper).trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toContain('docker is not turned on for db-01')
    expect(wrapper.find('[data-testid="db-restore-status"]').exists()).toBe(false)
  })

  it('shows the error a failed restore ended with', async () => {
    current = makeDatabaseRestoreRun({}, { status: 'failed', error_message: 'pg_restore: boom' })
    const wrapper = await mountDialog()
    await submit(wrapper).trigger('click')
    await flushPromises()

    const status = wrapper.find('[data-testid="db-restore-status"]')
    expect(status.text()).toContain('Failed')
    expect(status.find('.form-error').text()).toBe('pg_restore: boom')
  })

  it('falls back to a generic message when a failure has none', async () => {
    current = makeDatabaseRestoreRun({}, { status: 'failed', error_message: null })
    const wrapper = await mountDialog()
    await submit(wrapper).trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toBe('The restore failed.')
  })

  it('says when the host is offline and when the restore was cancelled', async () => {
    current = makeDatabaseRestoreRun({}, { status: 'pending' })
    vi.mocked(apiClient.post).mockResolvedValue({ data: current } as never)
    const wrapper = await mountDialog()
    await submit(wrapper).trigger('click')
    await flushPromises()

    const status = wrapper.find('[data-testid="db-restore-status"]')
    expect(status.text()).toContain('db-01 is offline. The restore starts when it connects again.')

    current = makeDatabaseRestoreRun({}, { status: 'cancelled' })
    pushWs('RestoreRunChanged', { id: current.id, status: current.status })
    await flushPromises()
    expect(status.text()).toContain('The restore was cancelled.')
  })

  it('starts over each time it opens', async () => {
    const wrapper = await mountDialog()
    await wrapper.find('#db-restore-name').setValue('other')
    await button(wrapper, 'Cancel')!.trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)

    await wrapper.setProps({ open: false })
    await wrapper.setProps({ open: true, dump: makeDatabaseDump({ database: 'billing' }) })

    expect(wrapper.find<HTMLInputElement>('#db-restore-name').element.value).toBe(
      'billing_restored',
    )
  })

  it('closes when the modal is dismissed', async () => {
    const wrapper = await mountDialog()
    wrapper.findComponent(BaseModal).vm.$emit('close')

    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('shows nothing to restore without a dump', async () => {
    const wrapper = await mountDialog(null, null)

    expect(wrapper.find('#db-restore-host').exists()).toBe(false)
    expect(submit(wrapper).element.disabled).toBe(true)
  })
})
