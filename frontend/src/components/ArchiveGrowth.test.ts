// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ArchiveGrowth, ArchiveGrowthResponse } from '../types/generated'

vi.mock('../api/archives', () => ({
  getArchiveGrowth: vi.fn(),
  recomputeArchiveGrowth: vi.fn(),
}))

import { getArchiveGrowth, recomputeArchiveGrowth } from '../api/archives'
import ArchiveGrowthView from './ArchiveGrowth.vue'

const GB = 1024 ** 3
const MB = 1024 ** 2

function makeArchive(name: string, start: string, deduplicated_size: number): ArchiveEntry {
  return {
    name,
    start,
    hostname: 'web-01',
    comment: '',
    original_size: 400 * GB,
    deduplicated_size,
    matched: true,
    agent_hostname: 'web-01',
    agent_domain: null,
  }
}

const ARCHIVE = makeArchive('web-01-2026-10-27', '2026-10-27T02:00:00Z', 3 * GB)
const EARLIER = [
  makeArchive('web-01-2026-10-26', '2026-10-26T02:00:00Z', 400 * MB),
  makeArchive('web-01-2026-10-25', '2026-10-25T02:00:00Z', 200 * MB),
]

const GROWTH: ArchiveGrowth = {
  total_bytes: 3 * GB,
  files: 4,
  children: [
    {
      name: 'home/alex',
      kind: 'directory',
      bytes: 2 * GB,
      files: 3,
      children: [
        { name: 'ubuntu.iso', kind: 'file', bytes: 1.5 * GB, files: 1, children: [] },
        { name: '', kind: 'others', bytes: 0.5 * GB, files: 2, children: [] },
      ],
    },
    { name: 'var/log', kind: 'directory', bytes: GB, files: 1, children: [] },
  ],
  largest_files: [
    { path: 'home/alex/ubuntu.iso', bytes: 1.5 * GB, size: 6 * GB },
    { path: 'var/log/syslog', bytes: GB, size: 2 * GB },
  ],
}

function done(growth: ArchiveGrowth = GROWTH): ArchiveGrowthResponse {
  return { status: 'done', growth, computed_at: '2026-10-27T08:00:00Z', error: null }
}

function inProgress(status: 'pending' | 'computing' = 'computing'): ArchiveGrowthResponse {
  return { status, growth: null, computed_at: null, error: null }
}

async function mountGrowth(archives: ArchiveEntry[] = [ARCHIVE, ...EARLIER]) {
  const wrapper = renderWithPlugins(ArchiveGrowthView, {
    props: { repoId: 3, archive: ARCHIVE, archives },
  })
  await flushPromises()
  return wrapper
}

function rowNames(wrapper: Awaited<ReturnType<typeof mountGrowth>>): string[] {
  return wrapper
    .findAll('[data-testid="archive-growth-folders"] tbody tr td:first-child')
    .map((cell) => cell.text())
}

describe('ArchiveGrowth', () => {
  beforeEach(() => {
    vi.mocked(getArchiveGrowth).mockReset()
    vi.mocked(recomputeArchiveGrowth).mockReset()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('shows the total, the comparison with the host and the top folders', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(done())

    const wrapper = await mountGrowth()

    expect(getArchiveGrowth).toHaveBeenCalledWith(3, 'web-01-2026-10-27')
    expect(wrapper.find('[data-testid="archive-growth-total"]').text()).toBe('3.0 GB')
    expect(wrapper.text()).toContain("10.2 times this host's usual 300.0 MB")
    expect(rowNames(wrapper)).toEqual(['home/alex', 'var/log'])
    expect(wrapper.text()).toContain('67%')
  })

  it('drills into a folder and back out through the path', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(done())
    const wrapper = await mountGrowth()

    await wrapper.find('button.growth-folder').trigger('click')

    expect(rowNames(wrapper)).toEqual(['ubuntu.iso', '2 files more'])
    const crumbs = wrapper.findAll('.path-crumbs button')
    expect(crumbs.map((crumb) => crumb.text())).toEqual(['/', 'home/alex'])

    await crumbs[0]!.trigger('click')
    expect(rowNames(wrapper)).toEqual(['home/alex', 'var/log'])
  })

  it('lists the largest files with their own size', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(done())
    const wrapper = await mountGrowth()

    const largest = wrapper.findAll('.segmented-option').find((b) => b.text() === 'Largest files')
    await largest!.trigger('click')

    const rows = wrapper.findAll('[data-testid="archive-growth-files"] tbody tr')
    expect(rows.map((row) => row.findAll('td').map((cell) => cell.text()))).toEqual([
      ['home/alex/ubuntu.iso', '1.5 GB', '6.0 GB'],
      ['var/log/syslog', '1.0 GB', '2.0 GB'],
    ])
  })

  it('says when there is no earlier archive of the host to compare with', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(done())

    const wrapper = await mountGrowth([ARCHIVE])

    expect(wrapper.text()).toContain('No earlier archive of this host to compare with')
  })

  it('explains that later archives share the rest when the breakdown is smaller', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(done({ ...GROWTH, total_bytes: GB }))

    const wrapper = await mountGrowth()

    expect(wrapper.text()).toContain('Archives written since share the rest')
  })

  it('polls while the breakdown is being worked out', async () => {
    vi.useFakeTimers()
    vi.mocked(getArchiveGrowth)
      .mockResolvedValueOnce(inProgress('pending'))
      .mockResolvedValueOnce(inProgress())
      .mockResolvedValue(done())

    const wrapper = await mountGrowth()
    expect(wrapper.text()).toContain("Working out where this archive's data is")

    await vi.advanceTimersByTimeAsync(2000)
    await vi.advanceTimersByTimeAsync(2000)
    await flushPromises()

    expect(getArchiveGrowth).toHaveBeenCalledTimes(3)
    expect(wrapper.find('[data-testid="archive-growth-total"]').exists()).toBe(true)

    await vi.advanceTimersByTimeAsync(10_000)
    expect(getArchiveGrowth).toHaveBeenCalledTimes(3)
  })

  it('stops polling once unmounted', async () => {
    vi.useFakeTimers()
    vi.mocked(getArchiveGrowth).mockResolvedValue(inProgress())
    const wrapper = await mountGrowth()

    wrapper.unmount()
    await vi.advanceTimersByTimeAsync(10_000)

    expect(getArchiveGrowth).toHaveBeenCalledTimes(1)
  })

  it('shows why working it out failed and tries again on request', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue({
      status: 'failed',
      growth: null,
      computed_at: null,
      error: 'repository is locked by another operation',
    })
    vi.mocked(recomputeArchiveGrowth).mockResolvedValue(done())
    const wrapper = await mountGrowth()

    expect(wrapper.text()).toContain('repository is locked by another operation')
    const retry = wrapper.findAll('button').find((b) => b.text() === 'Try again')
    await retry!.trigger('click')
    await flushPromises()

    expect(recomputeArchiveGrowth).toHaveBeenCalledWith(3, 'web-01-2026-10-27')
    expect(wrapper.find('[data-testid="archive-growth-total"]').text()).toBe('3.0 GB')
  })

  it('recomputes a finished breakdown', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(done())
    vi.mocked(recomputeArchiveGrowth).mockResolvedValue(inProgress('pending'))
    const wrapper = await mountGrowth()

    const recompute = wrapper.findAll('button').find((b) => b.text() === 'Recompute')
    await recompute!.trigger('click')
    await flushPromises()

    expect(recomputeArchiveGrowth).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).toContain("Working out where this archive's data is")
    wrapper.unmount()
  })

  it('says so when no data is held by this archive alone', async () => {
    vi.mocked(getArchiveGrowth).mockResolvedValue(
      done({ total_bytes: 0, files: 0, children: [], largest_files: [] }),
    )

    const wrapper = await mountGrowth()

    expect(wrapper.text()).toContain('Nothing only this archive holds')
    expect(wrapper.find('[data-testid="archive-growth-folders"]').exists()).toBe(false)
  })

  it('shows a request that fails outright', async () => {
    vi.mocked(getArchiveGrowth).mockRejectedValue(new Error('Network Error'))

    const wrapper = await mountGrowth()

    expect(wrapper.find('.error-banner').text()).toContain('Network Error')
  })
})
