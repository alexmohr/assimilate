// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { resetWsMock } from '../test-utils/wsMock'
import { makeDatabaseDump } from '../test-utils/databaseDump'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ArchiveDatabaseDump } from '../types/generated'

vi.mock('../api/archives', () => ({
  listArchiveDatabases: vi.fn(),
  restoreArchiveDatabase: vi.fn(),
}))
vi.mock('../composables/useWebSocket', () => import('../test-utils/wsMock'))

import { listArchiveDatabases } from '../api/archives'
import ArchiveDatabaseDumps from './ArchiveDatabaseDumps.vue'
import DatabaseRestoreDialog from './DatabaseRestoreDialog.vue'

function makeArchive(name: string, overrides: Partial<ArchiveEntry> = {}): ArchiveEntry {
  return {
    name,
    start: '2026-10-01T02:00:00Z',
    hostname: 'db-01',
    comment: '',
    original_size: 2048,
    deduplicated_size: 1024,
    matched: true,
    agent_hostname: 'db-01',
    agent_domain: 'lan',
    ...overrides,
  } as ArchiveEntry
}

async function mountDumps(canRestore = false, archive = makeArchive('db-01-2026-10-01')) {
  const wrapper = renderWithPlugins(ArchiveDatabaseDumps, {
    props: { repoId: 4, archive, canRestore },
  })
  await flushPromises()
  return wrapper
}

const KUBERNETES = makeDatabaseDump()
const DOCKER = makeDatabaseDump({
  runtime: 'docker',
  namespace: '',
  name: 'wiki-db',
  kind: 'container',
  pod: 'wiki-db-1',
  database: 'wiki',
  archive_path: 'dumps/wiki.dump',
})
const HOST = makeDatabaseDump({
  runtime: 'host',
  namespace: '',
  name: 'host',
  kind: 'host',
  pod: '/var/run/postgresql',
  database: 'app',
  archive_path: 'dumps/app.dump',
})

describe('ArchiveDatabaseDumps', () => {
  beforeEach(() => {
    vi.mocked(listArchiveDatabases).mockReset()
    resetWsMock()
  })

  it('shows a spinner while the dumps are read', async () => {
    vi.mocked(listArchiveDatabases).mockReturnValue(new Promise(() => undefined))
    const wrapper = await mountDumps()

    expect(wrapper.find('.loading-row').exists()).toBe(true)
    expect(listArchiveDatabases).toHaveBeenCalledWith(4, 'db-01-2026-10-01')
  })

  it('says why the dumps could not be read', async () => {
    vi.mocked(listArchiveDatabases).mockRejectedValue(new Error('borg list failed'))
    const wrapper = await mountDumps()

    expect(wrapper.find('.error-banner').text()).toBe('borg list failed')
  })

  it('says when the archive holds no dumps', async () => {
    vi.mocked(listArchiveDatabases).mockResolvedValue([])
    const wrapper = await mountDumps()

    expect(wrapper.text()).toContain('This archive holds no database dumps.')
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('lists each dump with its source named the way its runtime names it', async () => {
    vi.mocked(listArchiveDatabases).mockResolvedValue([
      KUBERNETES,
      DOCKER,
      HOST,
      makeDatabaseDump({ ...HOST, pod: '', database: 'other', archive_path: 'dumps/other.dump' }),
    ])
    const wrapper = await mountDumps()

    // Each cell's lines run together in its text: the source, then where it ran.
    const rows = wrapper.findAll('tbody tr').map((row) => row.findAll('td').map((td) => td.text()))
    expect(rows).toEqual([
      ['orders', 'shop/orders-dbKubernetes · pod orders-db-1', 'CloudNativePG'],
      ['wiki', 'wiki-dbDocker · container wiki-db-1', 'Labelled container'],
      ['app', 'PostgreSQL on this hostHost · through /var/run/postgresql', 'Host server'],
      ['other', 'PostgreSQL on this hostHost', 'Host server'],
    ])
    expect(wrapper.findAll('button').some((b) => b.text() === 'Restore')).toBe(false)
  })

  it('opens the restore dialog on the archive host for one who may restore', async () => {
    vi.mocked(listArchiveDatabases).mockResolvedValue([KUBERNETES, DOCKER])
    const wrapper = await mountDumps(true)

    const dialog = wrapper.findComponent(DatabaseRestoreDialog)
    expect(dialog.props('open')).toBe(false)

    const restore = wrapper.findAll('tbody tr')[1]!.find('button')
    expect(restore.attributes('title')).toBe('Restore wiki')
    await restore.trigger('click')

    expect(dialog.props()).toMatchObject({
      open: true,
      repoId: 4,
      archiveName: 'db-01-2026-10-01',
      dump: DOCKER,
      defaultHost: 'db-01',
      defaultDomain: 'lan',
    })

    dialog.vm.$emit('close')
    await flushPromises()
    expect(dialog.props('open')).toBe(false)
  })

  it('reads the dumps again for another archive, keeping only the latest answer', async () => {
    let answerFirst: (dumps: ArchiveDatabaseDump[]) => void = () => undefined
    vi.mocked(listArchiveDatabases).mockReturnValueOnce(
      new Promise((resolve) => {
        answerFirst = resolve
      }),
    )
    vi.mocked(listArchiveDatabases).mockResolvedValueOnce([DOCKER])
    const wrapper = await mountDumps()

    await wrapper.setProps({ archive: makeArchive('db-01-2026-10-02') })
    await flushPromises()
    answerFirst([KUBERNETES])
    await flushPromises()

    expect(listArchiveDatabases).toHaveBeenLastCalledWith(4, 'db-01-2026-10-02')
    expect(wrapper.findAll('tbody tr')).toHaveLength(1)
    expect(wrapper.find('tbody').text()).toContain('wiki')
  })

  it('drops a failure that arrives after a later archive was opened', async () => {
    let failFirst: (error: Error) => void = () => undefined
    vi.mocked(listArchiveDatabases).mockReturnValueOnce(
      new Promise((_, reject) => {
        failFirst = reject
      }),
    )
    vi.mocked(listArchiveDatabases).mockResolvedValueOnce([DOCKER])
    const wrapper = await mountDumps()

    await wrapper.setProps({ archive: makeArchive('db-01-2026-10-02') })
    await flushPromises()
    failFirst(new Error('stale'))
    await flushPromises()

    expect(wrapper.find('.error-banner').exists()).toBe(false)
    expect(wrapper.find('tbody').text()).toContain('wiki')
  })
})
