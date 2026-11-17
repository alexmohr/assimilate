// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { h } from 'vue'
import { renderWithPlugins } from '../test-utils'
import FileSourcesEditor from './FileSourcesEditor.vue'
import FileSourcesSection from './FileSourcesSection.vue'
import FileSourcesView from './FileSourcesView.vue'
import type { FileSet } from '../types/fileSet'
import type { HookCommand } from '../types/generated'

function fileSet(overrides: Partial<FileSet> = {}): FileSet {
  return {
    paths: '/etc\n\n/home',
    excludes: '*.cache',
    includes: '/home/keep',
    fileChangePatterns: '/data/wal/** ignore',
    preCommands: [{ command: 'systemctl stop app', timeout_seconds: null }],
    postCommands: [{ command: 'systemctl start app', timeout_seconds: 30 }],
    ...overrides,
  }
}

function button(wrapper: ReturnType<typeof renderWithPlugins>, label: string) {
  return wrapper.findAll('button').find((b) => b.text() === label)
}

/** The group labels of the read-only view, each with the text under it. */
function viewGroups(wrapper: ReturnType<typeof renderWithPlugins>): Record<string, string> {
  return Object.fromEntries(
    wrapper.findAll('.file-sources-group').map((group) => {
      const label = group.find('.group-label').text()
      return [label, group.text().slice(label.length).replace(/\s+/g, ' ').trim()]
    }),
  )
}

describe('FileSourcesView', () => {
  function mountView(props: Record<string, unknown> = {}) {
    return renderWithPlugins(FileSourcesView, {
      props: {
        paths: ['/etc', '/home'],
        excludes: ['*.cache'],
        includes: ['/home/keep'],
        fileChangePatternsRaw: '/data/wal/** ignore',
        preCommands: [{ command: 'systemctl stop app', timeout_seconds: null }],
        postCommands: [{ command: 'systemctl start app', timeout_seconds: 30 }],
        ...props,
      },
    })
  }

  it('shows every part of the file set under its own label', () => {
    const groups = viewGroups(mountView())
    expect(Object.keys(groups)).toEqual([
      'Backup paths',
      'Exclude patterns',
      'Include patterns',
      'File change patterns',
      'Pre-backup commands',
      'Post-backup commands',
    ])
    expect(groups['Backup paths']).toBe('/etc/home')
    expect(groups['Exclude patterns']).toBe('*.cache')
    expect(groups['Include patterns']).toBe('/home/keep')
    expect(groups['File change patterns']).toBe('/data/wal/** ignore')
  })

  it('says what a command without its own timeout runs under', () => {
    const groups = viewGroups(mountView({ inheritedTimeout: 'the host default (60s)' }))
    expect(groups['Pre-backup commands']).toContain('Timeout: the host default (60s)')
    expect(groups['Post-backup commands']).toContain('Timeout: 30s')
    expect(viewGroups(mountView())['Pre-backup commands']).toContain(
      "Timeout: the schedule's hook command timeout",
    )
  })

  it('says so when a part is empty', () => {
    const groups = viewGroups(
      mountView({
        paths: [],
        excludes: [],
        includes: [],
        fileChangePatternsRaw: '',
        preCommands: [],
        postCommands: [],
      }),
    )
    expect(Object.values(groups)).toEqual([
      'No paths configured.',
      'No excludes configured.',
      'No include patterns configured.',
      'No file change patterns configured.',
      'None configured.',
      'None configured.',
    ])
  })

  it('lets a caller show the paths its own way', () => {
    const wrapper = renderWithPlugins(FileSourcesView, {
      props: {
        paths: ['/etc'],
        excludes: [],
        includes: [],
        fileChangePatternsRaw: '',
        preCommands: [],
        postCommands: [],
      },
      slots: { paths: () => h('p', { class: 'custom-paths' }, 'per host') },
    })
    expect(viewGroups(wrapper)['Backup paths']).toBe('per host')
  })
})

describe('FileSourcesEditor', () => {
  function mountEditor(props: Record<string, unknown> = {}, slots: Record<string, unknown> = {}) {
    return renderWithPlugins(FileSourcesEditor, {
      props: {
        idPrefix: 'test',
        paths: '/etc',
        excludes: '*.cache',
        includes: '',
        fileChangePatterns: '',
        preCommands: [],
        postCommands: [],
        'onUpdate:paths': () => {},
        'onUpdate:excludes': () => {},
        'onUpdate:includes': () => {},
        'onUpdate:fileChangePatterns': () => {},
        'onUpdate:preCommands': () => {},
        'onUpdate:postCommands': () => {},
        ...props,
      },
      slots,
    })
  }

  it('labels each field, under ids from the given prefix', () => {
    const wrapper = mountEditor()
    for (const [id, label] of [
      ['test-paths', 'Backup paths'],
      ['test-excludes', 'Exclude patterns'],
      ['test-includes', 'Include patterns'],
    ]) {
      expect(wrapper.find(`label[for="${id}"]`).text()).toBe(label)
      expect(wrapper.find(`textarea#${id}`).exists()).toBe(true)
    }
  })

  it('starts from the values it was given', () => {
    const wrapper = mountEditor()
    expect((wrapper.find('#test-paths').element as HTMLTextAreaElement).value).toBe('/etc')
    expect((wrapper.find('#test-excludes').element as HTMLTextAreaElement).value).toBe('*.cache')
  })

  it.each([
    ['#test-paths', 'paths', '/srv'],
    ['#test-excludes', 'excludes', '*.tmp'],
    ['#test-includes', 'includes', '/home/keep'],
  ])('writes an edit of %s back through its own model', async (selector, model, value) => {
    const wrapper = mountEditor()
    await wrapper.find(selector).setValue(value)
    expect(wrapper.emitted(`update:${model}`)?.at(-1)?.[0]).toBe(value)
  })

  // Include patterns are exceptions carved out of the excludes, so the field
  // says so rather than reading as an unrelated second list.
  it('describes the include patterns as exceptions to the excludes', () => {
    const field = mountEditor().find('#test-includes').element.parentElement!
    expect(field.textContent).toContain(
      'Checked first: a path matching one of these is backed up even if an exclude covers it.',
    )
  })

  it('uses the dedicated editor for file change patterns', async () => {
    const wrapper = mountEditor()
    const editor = wrapper.findComponent({ name: 'FileChangePatternsEditor' })
    expect(editor.exists()).toBe(true)
    await editor.vm.$emit('update:modelValue', '**/*.log')
    expect(wrapper.emitted('update:fileChangePatterns')?.at(-1)?.[0]).toBe('**/*.log')
  })

  // `*` stops at `/` here, unlike borg's own patterns, so the hint says so.
  it('explains the file change pattern syntax on request', async () => {
    const wrapper = mountEditor()
    await wrapper.find('[aria-label="Help: file change patterns"]').trigger('click')
    const hint = wrapper.find('.help-hint-pop').text()
    expect(hint).toContain('matched against the full warning message')
    expect(hint).toContain('/data/wal/**')
  })

  it('offers one pre and one post command list, each with its own name and example', () => {
    const editors = mountEditor({ defaultTimeoutSeconds: 90 }).findAllComponents({
      name: 'CommandListEditor',
    })
    expect(editors.map((e) => e.props('ariaLabel'))).toEqual([
      'Pre-backup commands',
      'Post-backup commands',
    ])
    expect(editors.map((e) => e.props('placeholder'))).toEqual([
      'e.g. systemctl stop myapp',
      'e.g. systemctl start myapp',
    ])
    expect(editors.map((e) => e.props('defaultTimeoutSeconds'))).toEqual([90, 90])
  })

  it.each([
    [0, 'preCommands'],
    [1, 'postCommands'],
  ])('writes the command list at %i back through %s', async (index, model) => {
    const wrapper = mountEditor()
    const next: HookCommand[] = [{ command: 'echo hi', timeout_seconds: null }]
    await wrapper
      .findAllComponents({ name: 'CommandListEditor' })
      [index].vm.$emit('update:modelValue', next)
    expect(wrapper.emitted(`update:${model}`)?.at(-1)?.[0]).toEqual(next)
  })

  it('lets a caller swap the paths textarea for its own fields', () => {
    const wrapper = mountEditor({}, { paths: () => h('div', { class: 'per-host' }, 'fields') })
    expect(wrapper.find('#test-paths').exists()).toBe(false)
    expect(wrapper.find('.per-host').text()).toBe('fields')
    // The rest of the form stays as it is.
    expect(wrapper.find('#test-excludes').exists()).toBe(true)
  })
})

describe('FileSourcesSection', () => {
  function mountSection(props: Record<string, unknown> = {}, slots: Record<string, unknown> = {}) {
    return renderWithPlugins(FileSourcesSection, {
      props: { value: fileSet(), ...props },
      slots,
    })
  }

  it('shows the saved values, parsed, with an Edit button', () => {
    const wrapper = mountSection({ lede: 'What this host backs up.' })
    expect(wrapper.find('.pane-lede').text()).toBe('What this host backs up.')
    const view = wrapper.findComponent({ name: 'FileSourcesView' })
    // Blank lines are kept in the text but are not paths.
    expect(view.props('paths')).toEqual(['/etc', '/home'])
    expect(view.props('excludes')).toEqual(['*.cache'])
    expect(view.props('includes')).toEqual(['/home/keep'])
    expect(view.props('fileChangePatternsRaw')).toBe('/data/wal/** ignore')
    expect(button(wrapper, 'Edit')).toBeDefined()
    expect(wrapper.findComponent({ name: 'FileSourcesEditor' }).exists()).toBe(false)
  })

  it('hides the Edit button for a set that cannot be edited', () => {
    expect(button(mountSection({ canEdit: false }), 'Edit')).toBeUndefined()
  })

  it('opens the form on the saved values, as typed', async () => {
    const wrapper = mountSection()
    await button(wrapper, 'Edit')!.trigger('click')
    expect((wrapper.find('#file-sources-paths').element as HTMLTextAreaElement).value).toBe(
      '/etc\n\n/home',
    )
    expect(wrapper.findComponent({ name: 'FileSourcesView' }).exists()).toBe(false)
  })

  it('emits the edited set on Save and stays open until the caller closes it', async () => {
    const wrapper = mountSection()
    await button(wrapper, 'Edit')!.trigger('click')
    await wrapper.find('#file-sources-excludes').setValue('*.tmp')
    await wrapper
      .findAllComponents({ name: 'CommandListEditor' })[0]
      .vm.$emit('update:modelValue', [])
    await button(wrapper, 'Save')!.trigger('click')

    expect(wrapper.emitted('save')).toEqual([
      [{ ...fileSet(), excludes: '*.tmp', preCommands: [] }],
    ])
    // A save can fail; the form waits for the caller's word.
    expect(wrapper.find('#file-sources-excludes').exists()).toBe(true)
    ;(wrapper.vm as unknown as { close: () => void }).close()
    await wrapper.vm.$nextTick()
    expect(wrapper.find('#file-sources-excludes').exists()).toBe(false)
  })

  it('drops the edits on Cancel and reopens on the saved values', async () => {
    const wrapper = mountSection()
    await button(wrapper, 'Edit')!.trigger('click')
    await wrapper.find('#file-sources-excludes').setValue('*.tmp')
    await button(wrapper, 'Cancel')!.trigger('click')

    expect(wrapper.emitted('save')).toBeUndefined()
    expect(wrapper.findComponent({ name: 'FileSourcesView' }).props('excludes')).toEqual([
      '*.cache',
    ])

    await button(wrapper, 'Edit')!.trigger('click')
    expect((wrapper.find('#file-sources-excludes').element as HTMLTextAreaElement).value).toBe(
      '*.cache',
    )
  })

  it('does not change the saved command lists while they are edited', async () => {
    const value = fileSet()
    const wrapper = mountSection({ value })
    await button(wrapper, 'Edit')!.trigger('click')
    await wrapper
      .findAllComponents({ name: 'CommandListEditor' })[1]
      .vm.$emit('update:modelValue', [])
    await button(wrapper, 'Cancel')!.trigger('click')
    expect(value.postCommands).toEqual([{ command: 'systemctl start app', timeout_seconds: 30 }])
  })

  it('shows the error and locks the buttons while saving', async () => {
    const wrapper = mountSection()
    await button(wrapper, 'Edit')!.trigger('click')
    await wrapper.setProps({ saving: true, error: 'Failed to save' })
    expect(wrapper.find('.form-error').text()).toBe('Failed to save')
    expect(button(wrapper, 'Cancel')!.attributes('disabled')).toBeDefined()
    expect(button(wrapper, 'Saving...')!.attributes('disabled')).toBeDefined()
  })

  it('passes the inherited command timeout to the view and the form', async () => {
    const wrapper = mountSection({
      defaultTimeoutSeconds: 120,
      inheritedTimeout: 'the host default (120s)',
    })
    expect(wrapper.findComponent({ name: 'FileSourcesView' }).props('inheritedTimeout')).toBe(
      'the host default (120s)',
    )
    await button(wrapper, 'Edit')!.trigger('click')
    expect(
      wrapper.findComponent({ name: 'FileSourcesEditor' }).props('defaultTimeoutSeconds'),
    ).toBe(120)
  })

  it('swaps in per-host paths through its slots, in both the view and the form', async () => {
    const wrapper = mountSection(
      {},
      {
        viewPaths: () => h('dl', { class: 'per-host-view' }, 'host-01: /data'),
        editPaths: () => h('div', { class: 'per-host-edit' }, 'per-host fields'),
      },
    )
    expect(wrapper.find('.per-host-view').text()).toBe('host-01: /data')
    expect(wrapper.find('.paths-list').text()).not.toContain('/etc')

    await button(wrapper, 'Edit')!.trigger('click')
    expect(wrapper.find('.per-host-edit').text()).toBe('per-host fields')
    expect(wrapper.find('#file-sources-paths').exists()).toBe(false)
  })

  it('shows a footnote under the saved values from its hint slot', () => {
    const wrapper = mountSection({}, { hint: () => 'Global excludes apply too.' })
    expect(wrapper.find('p.field-hint').text()).toBe('Global excludes apply too.')
  })
})
