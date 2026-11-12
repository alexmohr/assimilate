// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { ArchiveEntryResponse, DiffResponse } from '../types/generated'

vi.mock('../api/archives', () => ({ diffArchives: vi.fn() }))

import { diffArchives } from '../api/archives'
import ArchiveDiff from './ArchiveDiff.vue'
import FileDiffDialog from './FileDiffDialog.vue'
import ImageDiffDialog from './ImageDiffDialog.vue'

function archive(start: string, overrides: Partial<ArchiveEntryResponse> = {}) {
  return {
    name: `web-01-${start}`,
    start,
    hostname: 'web-01',
    comment: '',
    original_size: 2048,
    deduplicated_size: 1024,
    matched: true,
    agent_hostname: 'web-01',
    ...overrides,
  } as ArchiveEntryResponse
}

const NEWEST = archive('2026-05-30T12:00:00')
const PREVIOUS = archive('2026-05-29T12:00:00')
const WEEK_BEFORE = archive('2026-05-20T12:00:00')
const OTHER_HOST = archive('2026-05-28T12:00:00', {
  name: 'db-01-2026-05-28T12:00:00',
  hostname: 'db-01',
  agent_hostname: 'db-01',
})
const ARCHIVES = [NEWEST, PREVIOUS, OTHER_HOST, WEEK_BEFORE]

function diff(overrides: Partial<DiffResponse> = {}): DiffResponse {
  return {
    added: ['etc/nginx/new.conf'],
    removed: ['etc/nginx/old.conf'],
    modified: ['etc/nginx/nginx.conf', 'srv/www/logo.PNG'],
    total_changes: 4,
    ...overrides,
  } as DiffResponse
}

async function mountDiff(
  props: { archive?: ArchiveEntryResponse; archives?: ArchiveEntryResponse[] } = {},
) {
  const wrapper = mount(ArchiveDiff, {
    props: { repoId: 1, archive: NEWEST, archives: ARCHIVES, ...props },
    global: { stubs: { FileDiffDialog: true, ImageDiffDialog: true } },
  })
  await flushPromises()
  return wrapper
}

function pill(wrapper: Awaited<ReturnType<typeof mountDiff>>, label: string) {
  return wrapper
    .get('[role="group"][aria-label="Compare with"]')
    .findAll('button')
    .find((b) => b.text().startsWith(label))!
}

function rowTitles(wrapper: Awaited<ReturnType<typeof mountDiff>>): string[] {
  return wrapper
    .findAll('[data-testid="diff-row"]')
    .map((row) => row.find('.browser-row-title').text())
}

describe('ArchiveDiff', () => {
  beforeEach(() => {
    vi.mocked(diffArchives).mockReset()
    vi.mocked(diffArchives).mockResolvedValue(diff())
  })

  it('compares with the previous archive of the same machine by default', async () => {
    const wrapper = await mountDiff()

    expect(diffArchives).toHaveBeenCalledTimes(1)
    expect(diffArchives).toHaveBeenCalledWith(1, {
      archive1: PREVIOUS.name,
      archive2: NEWEST.name,
    })
    expect(pill(wrapper, 'Previous').attributes('aria-pressed')).toBe('true')
    expect(wrapper.text()).toContain(`against ${PREVIOUS.name}`)
  })

  it('lists added, changed and removed paths with a summary', async () => {
    const wrapper = await mountDiff()

    expect(wrapper.text()).toContain('1 added · 2 changed · 1 removed')
    expect(rowTitles(wrapper)).toEqual([
      'etc/nginx/new.conf',
      'etc/nginx/nginx.conf',
      'srv/www/logo.PNG',
      'etc/nginx/old.conf',
    ])
    const dots = wrapper
      .findAll('[data-testid="diff-row"]')
      .map((row) => row.find('.status-dot').classes())
    expect(dots[0]).toContain('status-dot--success')
    expect(dots[1]).toContain('status-dot--warning')
    expect(dots[3]).toContain('status-dot--danger')
  })

  it('says when it shows only the first of more changes', async () => {
    vi.mocked(diffArchives).mockResolvedValue(diff({ total_changes: 250 }))
    const wrapper = await mountDiff()

    expect(wrapper.text()).toContain('(the first 4 of 250)')
  })

  it('says when nothing changed', async () => {
    vi.mocked(diffArchives).mockResolvedValue(
      diff({ added: [], removed: [], modified: [], total_changes: 0 }),
    )
    const wrapper = await mountDiff()

    expect(wrapper.text()).toContain(`Nothing changed since ${PREVIOUS.name}.`)
    expect(wrapper.findAll('[data-testid="diff-row"]')).toHaveLength(0)
  })

  it('shows the error when the comparison fails', async () => {
    vi.mocked(diffArchives).mockRejectedValue(new Error('borg diff failed'))
    const wrapper = await mountDiff()

    expect(wrapper.get('.error-banner').text()).toBe('borg diff failed')
  })

  it('has nothing to compare the oldest archive with until another is picked', async () => {
    const wrapper = await mountDiff({ archive: WEEK_BEFORE })

    expect(diffArchives).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('The oldest archive of this machine')
    expect(pill(wrapper, 'A week earlier').attributes('disabled')).toBeDefined()

    await pill(wrapper, 'Other archive').trigger('click')
    await flushPromises()
    // The newest other archive of the same machine; never another host's.
    expect(diffArchives).toHaveBeenCalledWith(1, {
      archive1: NEWEST.name,
      archive2: WEEK_BEFORE.name,
    })
    const options = wrapper
      .get('select[aria-label="Archive to compare with"]')
      .findAll('option')
      .map((o) => o.text())
    expect(options).toEqual([NEWEST.name, PREVIOUS.name])
  })

  it('compares with an archive a week earlier, or any other picked', async () => {
    const wrapper = await mountDiff()

    await pill(wrapper, 'A week earlier').trigger('click')
    await flushPromises()
    expect(diffArchives).toHaveBeenLastCalledWith(1, {
      archive1: WEEK_BEFORE.name,
      archive2: NEWEST.name,
    })

    await pill(wrapper, 'Other archive').trigger('click')
    await flushPromises()
    expect(diffArchives).toHaveBeenLastCalledWith(1, {
      archive1: PREVIOUS.name,
      archive2: NEWEST.name,
    })
    await wrapper.get('select[aria-label="Archive to compare with"]').setValue(WEEK_BEFORE.name)
    await flushPromises()
    expect(diffArchives).toHaveBeenLastCalledWith(1, {
      archive1: WEEK_BEFORE.name,
      archive2: NEWEST.name,
    })
  })

  it('goes back to the previous archive when its pill is picked again', async () => {
    const wrapper = await mountDiff()
    await pill(wrapper, 'A week earlier').trigger('click')
    await flushPromises()

    await pill(wrapper, 'Previous').trigger('click')
    await flushPromises()

    expect(pill(wrapper, 'Previous').attributes('aria-pressed')).toBe('true')
    expect(pill(wrapper, 'A week earlier').attributes('aria-pressed')).toBe('false')
    expect(diffArchives).toHaveBeenLastCalledWith(1, {
      archive1: PREVIOUS.name,
      archive2: NEWEST.name,
    })
  })

  it('goes back to the previous archive when another archive opens', async () => {
    const wrapper = await mountDiff()
    await pill(wrapper, 'A week earlier').trigger('click')
    await flushPromises()

    await wrapper.setProps({ archive: PREVIOUS })
    await flushPromises()

    expect(pill(wrapper, 'Previous').attributes('aria-pressed')).toBe('true')
    expect(diffArchives).toHaveBeenLastCalledWith(1, {
      archive1: WEEK_BEFORE.name,
      archive2: PREVIOUS.name,
    })
  })

  it('keeps only the latest comparison when the base changes mid-flight', async () => {
    let answerFirst: (d: DiffResponse) => void = () => undefined
    vi.mocked(diffArchives)
      .mockReturnValueOnce(new Promise((resolve) => (answerFirst = resolve)))
      .mockResolvedValueOnce(diff({ added: ['late/only.txt'], removed: [], modified: [] }))
    const wrapper = await mountDiff()

    await pill(wrapper, 'A week earlier').trigger('click')
    await flushPromises()
    answerFirst(diff())
    await flushPromises()

    expect(rowTitles(wrapper)).toEqual(['late/only.txt'])
  })

  describe('opening a changed file', () => {
    it('offers only changed rows as buttons, each saying how it compares', async () => {
      const wrapper = await mountDiff()
      const rows = wrapper.findAll('[data-testid="diff-row"]')

      expect(rows.map((row) => row.element.tagName)).toEqual(['DIV', 'BUTTON', 'BUTTON', 'DIV'])
      expect(rows[1]!.attributes('aria-label')).toBe('Compare etc/nginx/nginx.conf')
      expect(rows[1]!.find('.browser-row-facts').text()).toContain('show diff')
      expect(rows[2]!.find('.browser-row-facts').text()).toContain('compare images')
      expect(rows[0]!.find('.browser-row-facts').text()).toBe('added')

      // An added or removed file has no other side, so clicking it opens nothing.
      await rows[0]!.trigger('click')
      await rows[3]!.trigger('click')
      expect(wrapper.findComponent(FileDiffDialog).exists()).toBe(false)
      expect(wrapper.findComponent(ImageDiffDialog).exists()).toBe(false)
    })

    it('opens a text file as a text diff against the base', async () => {
      const wrapper = await mountDiff()
      await wrapper.findAll('[data-testid="diff-row"]')[1]!.trigger('click')

      const dialog = wrapper.findComponent(FileDiffDialog)
      expect(dialog.exists()).toBe(true)
      expect(dialog.props()).toMatchObject({
        open: true,
        repoId: 1,
        archive: NEWEST,
        path: 'etc/nginx/nginx.conf',
        initialAgainst: PREVIOUS.name,
      })
      expect(wrapper.findComponent(ImageDiffDialog).exists()).toBe(false)

      dialog.vm.$emit('close')
      await flushPromises()
      expect(wrapper.findComponent(FileDiffDialog).exists()).toBe(false)
    })

    it('opens an image as an image comparison, whatever case its extension', async () => {
      const wrapper = await mountDiff()
      await wrapper.findAll('[data-testid="diff-row"]')[2]!.trigger('click')

      const dialog = wrapper.findComponent(ImageDiffDialog)
      expect(dialog.exists()).toBe(true)
      expect(dialog.props()).toMatchObject({
        open: true,
        repoId: 1,
        archive: NEWEST,
        base: PREVIOUS,
        path: 'srv/www/logo.PNG',
      })
      expect(wrapper.findComponent(FileDiffDialog).exists()).toBe(false)

      dialog.vm.$emit('close')
      await flushPromises()
      expect(wrapper.findComponent(ImageDiffDialog).exists()).toBe(false)
    })

    it('compares the image against the base picked, not always the previous', async () => {
      const wrapper = await mountDiff()
      await pill(wrapper, 'A week earlier').trigger('click')
      await flushPromises()
      await wrapper.findAll('[data-testid="diff-row"]')[2]!.trigger('click')

      expect(wrapper.findComponent(ImageDiffDialog).props('base')).toEqual(WEEK_BEFORE)
    })
  })
})
