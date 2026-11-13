// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount, RouterLinkStub } from '@vue/test-utils'
import { nextTick } from 'vue'
import ArchiveSelector from './ArchiveSelector.vue'
import type { RepoArchive } from '../utils/repoArchive'

function archive(overrides: Partial<RepoArchive> = {}): RepoArchive {
  return {
    repo_id: 1,
    repo_name: 'nightly',
    name: 'web-01-2026-03-01',
    start: '2026-03-01T02:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 1000,
    deduplicated_size: 100,
    matched: true,
    agent_hostname: 'web-01',
    ...overrides,
  }
}

const WEB = archive()
const DB = archive({
  name: 'db-01-2026-02-01',
  start: '2026-02-01T02:00:00Z',
  hostname: 'db-01',
  agent_hostname: 'db-01',
  original_size: 3000,
  deduplicated_size: 300,
})

type Wrapper = ReturnType<typeof selector>

function selector(props: Record<string, unknown> = {}) {
  return mount(ArchiveSelector, {
    global: { stubs: { RouterLink: RouterLinkStub } },
    props: {
      archives: [WEB, DB],
      selected: null,
      ...props,
    },
  })
}

async function openListMenu(wrapper: Wrapper): Promise<void> {
  await wrapper.get('button[aria-label="List options"]').trigger('click')
  await nextTick()
}

/** Picks a grouping or sort from the list's menu. */
async function pickFromMenu(wrapper: Wrapper, label: string): Promise<void> {
  await openListMenu(wrapper)
  const option = wrapper.findAll('.overflow-menu-item').find((b) => b.text() === label)
  expect(option).toBeDefined()
  await option!.trigger('click')
  await nextTick()
}

function rows(wrapper: Wrapper) {
  return wrapper.findAll('[data-testid="archive-row"]')
}

function rowNames(wrapper: Wrapper): string[] {
  return rows(wrapper).map((row) => row.get('.browser-row-title').text())
}

describe('ArchiveSelector', () => {
  it('groups by host and totals each group', () => {
    const wrapper = selector()
    const groups = wrapper.findAll('.group-header')
    expect(groups).toHaveLength(2)
    // Groups sort by hostname, so db-01 leads.
    expect(groups[0]!.find('.group-hostname').text()).toBe('db-01')
    expect(groups[0]!.find('.group-count').text()).toBe('1')
    expect(groups[0]!.find('.group-size').text()).toBe('2.9 KB')
    expect(rowNames(wrapper)).toEqual([DB.name, WEB.name])
  })

  it('links a group host to its agent without swallowing the collapse toggle', async () => {
    const wrapper = selector()
    const header = () => wrapper.findAll('.group-header')[0]!

    const link = header().findComponent(RouterLinkStub)
    expect(link.props('to')).toBe('/agents/db-01')
    expect(link.text()).toBe('db-01')

    // The link lives inside the header, so the header can no longer be the
    // button; the header takes the click and the toggle beside the link
    // carries the state for a screen reader.
    expect(header().classes()).not.toContain('collapsed')
    await header().get('.group-toggle').trigger('click')
    expect(header().classes()).toContain('collapsed')
    expect(header().get('.group-toggle').attributes('aria-expanded')).toBe('false')
    expect(rowNames(wrapper)).toEqual([WEB.name])

    // Clicking the header anywhere else toggles it back...
    await header().get('.group-count').trigger('click')
    expect(header().classes()).not.toContain('collapsed')
    expect(rowNames(wrapper)).toEqual([DB.name, WEB.name])

    // ...but following the host link must not collapse the group under it.
    await header().findComponent(RouterLinkStub).trigger('click')
    expect(header().classes()).not.toContain('collapsed')
  })

  it('switches to a flat list through the list menu', async () => {
    const wrapper = selector()
    expect(wrapper.find('.group-header').exists()).toBe(true)

    await pickFromMenu(wrapper, 'Flat list')

    expect(wrapper.find('.group-header').exists()).toBe(false)
    expect(wrapper.findAll('.row-group-label').map((l) => l.text())).toEqual(['All archives'])
    expect(rows(wrapper)).toHaveLength(2)
    // The list spans two hosts, so each row names its own.
    expect(rows(wrapper)[0]!.get('.browser-row-facts').text()).toContain('web-01')
  })

  it('searches name and host through one box', async () => {
    const wrapper = selector()
    await pickFromMenu(wrapper, 'Flat list')

    await wrapper.get('.archive-controls input').setValue('db-01')
    await nextTick()

    expect(rowNames(wrapper)).toEqual([DB.name])
    expect(wrapper.get('.archives-count').text()).toBe('1 of 2')
  })

  it('says so when the search matches nothing', async () => {
    const wrapper = selector()
    await wrapper.get('.archive-controls input').setValue('nothing-like-this')
    await nextTick()

    expect(wrapper.get('.archive-no-match').text()).toContain('No archive names match')
    expect(rows(wrapper)).toHaveLength(0)
  })

  it('reorders through the sort in the list menu', async () => {
    const wrapper = selector()
    await pickFromMenu(wrapper, 'Flat list')
    expect(rowNames(wrapper)).toEqual([WEB.name, DB.name])

    await pickFromMenu(wrapper, 'Size largest first')

    expect(rowNames(wrapper)).toEqual([DB.name, WEB.name])
    await openListMenu(wrapper)
    const checked = wrapper
      .findAll('[role="menuitemradio"][aria-checked="true"]')
      .map((b) => b.text())
    expect(checked).toEqual(['Flat list', 'Size largest first'])
  })

  it('reports the picked archive through the model', async () => {
    const wrapper = selector()
    await rows(wrapper)[0]!.trigger('click')

    expect(wrapper.emitted('update:selected')?.[0]).toEqual([DB])
  })

  it('marks only the archives whose delete is already queued', () => {
    const wrapper = selector({ deletingKeys: [`${DB.repo_id}/${DB.name}`] })
    expect(wrapper.findAll('.archive-row-deleting')).toHaveLength(1)
    expect(rows(wrapper)[0]!.classes()).toContain('browser-row--dim')
    expect(rows(wrapper)[1]!.classes()).not.toContain('browser-row--dim')
  })

  it.each(['By host', 'Flat list', 'By day'])(
    'marks an archive queued for maintenance in the %s view',
    async (mode) => {
      const wrapper = selector({ queuedKeys: [`${DB.repo_id}/${DB.name}`] })
      await pickFromMenu(wrapper, mode)

      const marked = rows(wrapper).filter((r) => r.text().includes('Removed at next maintenance'))
      expect(marked).toHaveLength(1)
      expect(marked[0]!.text()).toContain(DB.name)
    },
  )

  it('shows placeholder rows while loading, and nothing else', () => {
    const wrapper = selector({ loading: true })
    expect(wrapper.find('.archive-loading').exists()).toBe(true)
    expect(wrapper.find('.archive-controls').exists()).toBe(false)
    expect(rows(wrapper)).toHaveLength(0)
    expect(wrapper.find('.archives-count').exists()).toBe(false)
  })

  it('shows a failed load instead of an empty list', () => {
    const wrapper = selector({ archives: [], error: 'Connection refused' })
    expect(wrapper.find('.error-banner').text()).toBe('Connection refused')
    expect(wrapper.find('.empty-state').exists()).toBe(false)
  })

  it('uses the caller wording for an empty repository', () => {
    const wrapper = selector({
      archives: [],
      emptyTitle: 'No archives',
      emptyDescription: 'No backup archives found for this schedule.',
    })
    expect(wrapper.find('.empty-state').text()).toContain('this schedule')
  })

  it('drops the controls and rows for a caller filtered to one archive', () => {
    const wrapper = selector({ hideControls: true, refreshable: true })
    expect(wrapper.find('.archive-controls').exists()).toBe(false)
    expect(rows(wrapper)).toHaveLength(0)
    expect(wrapper.find('button[aria-label="List options"]').exists()).toBe(false)
    // The count still reports what the repository holds.
    expect(wrapper.get('.archives-count').text()).toBe('2')
  })

  it('renders the actions and banner slots in the panel', () => {
    const wrapper = mount(ArchiveSelector, {
      global: { stubs: { RouterLink: RouterLinkStub } },
      props: { archives: [WEB], selected: null },
      slots: {
        actions: '<button class="stub-action">Diff</button>',
        banner: '<div class="stub-banner">3 unmatched</div>',
      },
    })
    expect(wrapper.find('.panel-header .stub-action').exists()).toBe(true)
    expect(wrapper.find('.stub-banner').exists()).toBe(true)
  })

  it('offers a refresh in the menu only to a refreshable list', async () => {
    const plain = selector()
    await openListMenu(plain)
    expect(plain.findAll('.overflow-menu-item').some((b) => b.text() === 'Refresh archives')).toBe(
      false,
    )

    const wrapper = selector({ refreshable: true })
    await pickFromMenu(wrapper, 'Refresh archives')
    expect(wrapper.emitted('refresh')).toHaveLength(1)
  })

  describe('looking for files', () => {
    it('offers a file search from two characters on, through the archives in view', async () => {
      const wrapper = selector({ fileSearch: true })
      const input = wrapper.get('.archive-controls input')
      expect(input.attributes('placeholder')).toBe('Search archives or files')

      await input.setValue('n')
      expect(wrapper.find('[data-testid="find-files"]').exists()).toBe(false)

      await input.setValue(' nginx ')
      const find = wrapper.get('[data-testid="find-files"]')
      expect(find.text()).toContain('Find files named “nginx”')
      expect(find.text()).toContain('in all 2 archives')
      await find.trigger('click')

      expect(wrapper.emitted('search-files')).toEqual([['nginx', [WEB, DB]]])
    })

    it('says to use that row when no archive name matches, and Enter runs it', async () => {
      const wrapper = selector({ fileSearch: true })
      const input = wrapper.get('.archive-controls input')
      await input.setValue('nginx')

      expect(wrapper.get('.archive-no-match').text()).toContain('Use the row above')
      await input.trigger('keydown', { key: 'Enter' })
      expect(wrapper.emitted('search-files')).toHaveLength(1)
    })

    it('leaves Enter to the archive names while some match', async () => {
      const wrapper = selector({ fileSearch: true })
      const input = wrapper.get('.archive-controls input')
      await input.setValue('web')
      await input.trigger('keydown', { key: 'Enter' })
      expect(wrapper.emitted('search-files')).toBeUndefined()
    })

    it('does not offer it to a list that does not search files', async () => {
      const wrapper = selector()
      const input = wrapper.get('.archive-controls input')
      expect(input.attributes('placeholder')).toBe('Search archives')
      await input.setValue('nginx')
      expect(wrapper.find('[data-testid="find-files"]').exists()).toBe(false)
    })
  })

  it('clears the search when an archive it hides is selected from outside', async () => {
    const wrapper = selector()
    await wrapper.get('.archive-controls input').setValue('web')
    expect(rowNames(wrapper)).toEqual([WEB.name])

    await wrapper.setProps({ selected: DB })

    expect((wrapper.get('.archive-controls input').element as HTMLInputElement).value).toBe('')
    expect(rowNames(wrapper)).toEqual([DB.name, WEB.name])
  })

  describe('across repositories', () => {
    const OFFSITE = archive({ name: 'web-01-2026-03-01', repo_id: 2, repo_name: 'offsite' })

    function repoPills(wrapper: Wrapper) {
      return wrapper.findAll('[role="group"][aria-label="Repository"] button')
    }

    it('offers a filter and tags rows only when the list spans several repositories', async () => {
      const single = selector()
      expect(repoPills(single)).toHaveLength(0)
      expect(rows(single).some((r) => r.text().includes('nightly'))).toBe(false)

      const spread = selector({ archives: [WEB, OFFSITE] })
      expect(repoPills(spread).map((o) => o.text())).toEqual([
        'All · 2',
        'nightly · 1',
        'offsite · 1',
      ])
      const facts = () => rows(spread).map((r) => r.get('.browser-row-facts').text())
      expect(facts()[0]).toContain('nightly')
      expect(facts()[1]).toContain('offsite')

      await repoPills(spread)[2]!.trigger('click')
      expect(repoPills(spread)[2]!.attributes('aria-pressed')).toBe('true')
      expect(rows(spread)).toHaveLength(1)
      // Narrowed to one repository, the row no longer names it.
      expect(facts()[0]).not.toContain('offsite')
    })

    it('searches files only in the repository the pills narrow to', async () => {
      const wrapper = selector({ archives: [WEB, OFFSITE], fileSearch: true })
      await repoPills(wrapper)[2]!.trigger('click')
      await wrapper.get('.archive-controls input').setValue('hosts')

      const find = wrapper.get('[data-testid="find-files"]')
      expect(find.text()).toContain('in 1 archive')
      expect(find.text()).not.toContain('all')
      await find.trigger('click')
      expect(wrapper.emitted('search-files')).toEqual([['hosts', [OFFSITE]]])
    })

    it('tells two copies of the same name apart when selecting', () => {
      const wrapper = selector({ archives: [WEB, OFFSITE], selected: OFFSITE })
      const selected = wrapper.findAll('.browser-row--selected')
      expect(selected).toHaveLength(1)
      expect(selected[0]!.text()).toContain('offsite')
    })
  })

  it('lists one host by day, without repeating its name on every row', () => {
    const wrapper = selector({
      archives: [WEB, archive({ name: 'web-01-old', start: '2020-01-05T02:00:00Z' })],
    })
    expect(wrapper.findAll('.row-group-label').map((h) => h.text())).toEqual([
      'March 2026',
      'January 2020',
    ])
    expect(rows(wrapper).map((r) => r.get('.browser-row-facts').text())).toEqual([
      '+100.0 B new',
      '+100.0 B new',
    ])
  })

  it('offers host grouping only when the list holds more than one host', async () => {
    const labels = async (wrapper: Wrapper): Promise<string[]> => {
      await openListMenu(wrapper)
      return wrapper
        .findAll('[role="menuitemradio"]')
        .map((b) => b.text())
        .filter((t) => t.startsWith('By ') || t === 'Flat list')
    }

    expect(await labels(selector({ archives: [WEB, DB] }))).toEqual([
      'By day',
      'By host',
      'Flat list',
    ])
    expect(await labels(selector({ archives: [WEB] }))).toEqual(['By day', 'Flat list'])
  })
})
