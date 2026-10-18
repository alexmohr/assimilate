// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import RestoreTree from './RestoreTree.vue'
import type { EntryRow, TreeRow } from '../composables/useRestoreTree'

function row(path: string, depth: number, isDir: boolean, extra: Partial<EntryRow> = {}): EntryRow {
  return {
    kind: 'entry',
    key: path,
    path,
    name: path.split('/').pop() ?? path,
    depth,
    isDir,
    entry: { path, type: isDir ? 'd' : '-', size: isDir ? 0 : 2048, mtime: '', mode: '' },
    expandable: isDir,
    expanded: false,
    loading: false,
    match: false,
    ...extra,
  }
}

const ROWS: TreeRow[] = [
  row('etc', 0, true, { expanded: true }),
  row('etc/hosts', 1, false),
  row('etc/nginx', 1, true),
  row('var', 0, true),
]

function mountTree(selection: string[], rows: TreeRow[] = ROWS, highlight = '') {
  return mount(RestoreTree, {
    props: {
      rows,
      selection,
      totals: { 'etc/nginx': { path: 'etc/nginx', files: 2, bytes: 4096 } },
      changeOf: (entry) => (entry.path === 'etc/hosts' ? 'changed' : null),
      highlight,
    },
    attachTo: document.body,
  })
}

function item(wrapper: ReturnType<typeof mountTree>, path: string) {
  return wrapper.find(`[data-path="${path}"]`)
}

describe('RestoreTree', () => {
  it('shows a ticked folder checked, its parent mixed and the rest unticked', () => {
    const wrapper = mountTree(['etc/nginx'])

    expect(item(wrapper, 'etc/nginx').attributes('aria-checked')).toBe('true')
    expect(item(wrapper, 'etc').attributes('aria-checked')).toBe('mixed')
    expect(item(wrapper, 'etc/hosts').attributes('aria-checked')).toBe('false')
    const parentBox = item(wrapper, 'etc').find('input').element as HTMLInputElement
    expect(parentBox.indeterminate).toBe(true)
    wrapper.unmount()
  })

  it('shows folder totals, file sizes and change badges', () => {
    const wrapper = mountTree([])

    expect(item(wrapper, 'etc/nginx').text()).toContain('2 files')
    expect(item(wrapper, 'etc/hosts').text()).toContain('2.0 KB')
    expect(item(wrapper, 'etc/hosts').find('.badge').text()).toBe('Changed')
    wrapper.unmount()
  })

  it('emits a tick for a checkbox click', async () => {
    const wrapper = mountTree([])

    await item(wrapper, 'etc/hosts').find('input').trigger('click')

    expect(wrapper.emitted('tick')).toEqual([['etc/hosts']])
    wrapper.unmount()
  })

  it('moves with the arrow keys and ticks with space', async () => {
    const wrapper = mountTree([])
    const tree = wrapper.find('[role="tree"]')

    await tree.trigger('keydown', { key: 'ArrowDown' })
    await tree.trigger('keydown', { key: ' ' })
    await tree.trigger('keydown', { key: 'ArrowDown' })
    await tree.trigger('keydown', { key: 'ArrowRight' })
    await tree.trigger('keydown', { key: 'ArrowLeft' })
    await tree.trigger('keydown', { key: 'ArrowLeft' })

    expect(wrapper.emitted('tick')).toEqual([['etc/hosts']])
    expect(wrapper.emitted('expand')).toEqual([['etc/nginx'], ['etc']])
    expect(item(wrapper, 'etc').attributes('tabindex')).toBe('0')
    wrapper.unmount()
  })

  it('highlights the search text in names', () => {
    const wrapper = mountTree([], [row('etc/nginx', 0, true, { match: true })], 'ngi')

    expect(item(wrapper, 'etc/nginx').find('mark').text()).toBe('ngi')
    wrapper.unmount()
  })

  it('offers more entries of a long directory', async () => {
    const wrapper = mountTree(
      [],
      [row('a', 0, false), { kind: 'more', key: 'more', dir: '', depth: 0, loading: false }],
    )

    await wrapper.find('.tree-row--more button').trigger('click')

    expect(wrapper.emitted('more')).toEqual([['']])
    wrapper.unmount()
  })

  it('opens a folder or ticks a file with Enter', async () => {
    const wrapper = mountTree([])
    const tree = wrapper.find('[role="tree"]')

    await tree.trigger('keydown', { key: 'Enter' })
    await tree.trigger('keydown', { key: 'ArrowDown' })
    await tree.trigger('keydown', { key: 'Enter' })

    expect(wrapper.emitted('expand')).toEqual([['etc']])
    expect(wrapper.emitted('tick')).toEqual([['etc/hosts']])
    wrapper.unmount()
  })

  it('leaves a key the tree does not use to the browser', async () => {
    const wrapper = mountTree([])
    const event = new KeyboardEvent('keydown', { key: 'a', cancelable: true, bubbles: true })

    wrapper.find('[role="tree"]').element.dispatchEvent(event)

    expect(event.defaultPrevented).toBe(false)
    expect(wrapper.emitted('tick')).toBeUndefined()
    wrapper.unmount()
  })

  it('opens a folder or ticks a file when its name is clicked', async () => {
    const wrapper = mountTree([])

    await item(wrapper, 'var').find('.tree-name').trigger('click')
    await item(wrapper, 'etc/hosts').find('.tree-name').trigger('click')

    expect(wrapper.emitted('expand')).toEqual([['var']])
    expect(wrapper.emitted('tick')).toEqual([['etc/hosts']])
    wrapper.unmount()
  })

  it('makes a focused "Load more" row the one Tab lands on', async () => {
    const wrapper = mountTree(
      [],
      [row('a', 0, false), { kind: 'more', key: 'more', dir: '', depth: 0, loading: false }],
    )
    const more = wrapper.find('.tree-row--more')

    await more.trigger('focus')

    expect(more.attributes('tabindex')).toBe('0')
    expect(item(wrapper, 'a').attributes('tabindex')).toBe('-1')
    wrapper.unmount()
  })

  it('forgets the focused row once it leaves the tree', async () => {
    const wrapper = mountTree([])
    await item(wrapper, 'var').trigger('focus')
    expect(item(wrapper, 'var').attributes('tabindex')).toBe('0')

    await wrapper.setProps({ rows: ROWS.slice(0, 3) })
    await wrapper.setProps({ rows: ROWS })

    expect(item(wrapper, 'etc').attributes('tabindex')).toBe('0')
    expect(item(wrapper, 'var').attributes('tabindex')).toBe('-1')
    wrapper.unmount()
  })
})
