// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import ExcludesView from './ExcludesView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    put: vi.fn(),
  },
}))

import { apiClient } from '../api/client'

const mockGet = vi.mocked(apiClient.get)
const mockPut = vi.mocked(apiClient.put)

const MOCK_RAW_TEXT = 'node_modules\n__pycache__'

async function mountWith(rawText: string): Promise<ReturnType<typeof renderWithPlugins>> {
  mockGet.mockResolvedValue({ data: { raw_text: rawText } })
  const wrapper = renderWithPlugins(ExcludesView)
  await flushPromises()
  return wrapper
}

type Wrapper = Awaited<ReturnType<typeof mountWith>>

function button(wrapper: Wrapper, label: string) {
  const found = wrapper.findAll('button').find((b) => b.text() === label)
  if (!found) throw new Error(`no button "${label}"`)
  return found
}

async function startEdit(wrapper: Wrapper): Promise<void> {
  await button(wrapper, 'Edit').trigger('click')
}

/** The saved list as it reads at rest: `#` headings and patterns, in order. */
function listItems(wrapper: Wrapper): { kind: string; text: string }[] {
  return wrapper.findAll('ul[aria-label="Global exclude patterns"] > li').map((li) => ({
    kind: li.classes('pattern-comment') ? 'comment' : 'pattern',
    text: li.text(),
  }))
}

describe('ExcludesView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders page title', async () => {
    const wrapper = await mountWith(MOCK_RAW_TEXT)
    expect(wrapper.text()).toContain('Global Excludes')
  })

  it('calls GET /excludes on mount', async () => {
    await mountWith(MOCK_RAW_TEXT)
    expect(mockGet).toHaveBeenCalledWith('/excludes')
  })

  it('shows error when API fails', async () => {
    mockGet.mockRejectedValue(new Error('Network error'))
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(wrapper.find('.excludes-card').exists()).toBe(false)
  })

  describe('read view', () => {
    it('lists the loaded patterns one per row, read-only, with a count', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      expect(wrapper.find('textarea').exists()).toBe(false)
      expect(listItems(wrapper)).toEqual([
        { kind: 'pattern', text: 'node_modules' },
        { kind: 'pattern', text: '__pycache__' },
      ])
      expect(wrapper.find('.pattern-foot').text()).toContain('2 patterns')
    })

    it('says what the card is for', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      expect(wrapper.find('.pane-lede').text()).toContain('Paths every backup leaves out')
    })

    it('shows a # comment as a heading over the patterns under it, without its hashes', async () => {
      const wrapper = await mountWith('# Caches\n*/.cache\n\n  ##   Build output  \n*/target\n')
      expect(listItems(wrapper)).toEqual([
        { kind: 'comment', text: 'Caches' },
        { kind: 'pattern', text: '*/.cache' },
        { kind: 'comment', text: 'Build output' },
        { kind: 'pattern', text: '*/target' },
      ])
      // Patterns are set in the monospace face, headings are not.
      expect(wrapper.find('.pattern-row').classes()).toContain('mono')
      expect(wrapper.find('.pattern-comment').classes()).not.toContain('mono')
    })

    it('counts only patterns, not comments or blank lines', async () => {
      const wrapper = await mountWith('# one\n\n*/a\n   \n# two\n')
      expect(wrapper.find('.pattern-foot').text()).toContain('1 pattern')
      expect(wrapper.find('.pattern-foot').text()).not.toContain('1 patterns')
    })

    it('trims the whitespace around each pattern', async () => {
      const wrapper = await mountWith('   */.cache   \n')
      expect(listItems(wrapper)).toEqual([{ kind: 'pattern', text: '*/.cache' }])
    })

    it('says so when there are no excludes', async () => {
      const wrapper = await mountWith('')
      expect(wrapper.find('ul[aria-label="Global exclude patterns"]').exists()).toBe(false)
      expect(wrapper.text()).toContain('No global excludes yet')
      expect(wrapper.find('.pattern-foot').text()).toContain('0 patterns')
    })

    it('treats a list of comments alone as having no excludes', async () => {
      const wrapper = await mountWith('# nothing yet\n')
      expect(wrapper.text()).toContain('No global excludes yet')
      expect(wrapper.find('ul[aria-label="Global exclude patterns"]').exists()).toBe(false)
    })

    it('shows no pattern warnings at rest', async () => {
      const wrapper = await mountWith('.cache')
      expect(wrapper.find('[aria-label="Pattern warnings"]').exists()).toBe(false)
    })
  })

  describe('pattern reference', () => {
    it('opens and closes the reference from a link in the card', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      expect(wrapper.text()).not.toContain('Borg Pattern Syntax')

      await button(wrapper, 'Pattern reference').trigger('click')
      expect(wrapper.text()).toContain('Borg Pattern Syntax')
      expect(wrapper.text()).toContain('ignore global excludes')

      await button(wrapper, 'Close pattern reference').trigger('click')
      expect(wrapper.text()).not.toContain('Borg Pattern Syntax')
    })

    it('offers the reference while editing too, and keeps it open across modes', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      await startEdit(wrapper)
      await button(wrapper, 'Pattern reference').trigger('click')
      expect(wrapper.text()).toContain('Borg Pattern Syntax')

      await button(wrapper, 'Cancel').trigger('click')
      expect(wrapper.text()).toContain('Borg Pattern Syntax')
      expect(button(wrapper, 'Close pattern reference').exists()).toBe(true)
    })
  })

  describe('editing', () => {
    it('opens a text editor filled with the saved text', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      await startEdit(wrapper)
      const textarea = wrapper.find<HTMLTextAreaElement>('textarea')
      expect(textarea.attributes('aria-label')).toBe('Global exclude patterns')
      expect(textarea.element.value).toBe(MOCK_RAW_TEXT)
      expect(wrapper.find('ul[aria-label="Global exclude patterns"]').exists()).toBe(false)
      expect(wrapper.findAll('button').map((b) => b.text())).not.toContain('Edit')
    })

    it('opens an empty editor when there are no excludes', async () => {
      const wrapper = await mountWith('')
      await startEdit(wrapper)
      expect(wrapper.find<HTMLTextAreaElement>('textarea').element.value).toBe('')
    })

    it('saves the edited text and shows it as the list', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      mockPut.mockResolvedValue({ data: {} })
      await startEdit(wrapper)
      await wrapper.find('textarea').setValue('# Caches\n*/.cache')
      await button(wrapper, 'Save').trigger('click')
      await flushPromises()

      expect(mockPut).toHaveBeenCalledWith('/excludes', { raw_text: '# Caches\n*/.cache' })
      expect(wrapper.find('textarea').exists()).toBe(false)
      expect(listItems(wrapper)).toEqual([
        { kind: 'comment', text: 'Caches' },
        { kind: 'pattern', text: '*/.cache' },
      ])
      expect(wrapper.find('.pattern-foot').text()).toContain('1 pattern')
    })

    it('throws the draft away on Cancel', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      await startEdit(wrapper)
      await wrapper.find('textarea').setValue('*/discarded')
      await button(wrapper, 'Cancel').trigger('click')

      expect(mockPut).not.toHaveBeenCalled()
      expect(listItems(wrapper).map((i) => i.text)).toEqual(['node_modules', '__pycache__'])

      // Editing again starts from the saved text, not the discarded draft.
      await startEdit(wrapper)
      expect(wrapper.find<HTMLTextAreaElement>('textarea').element.value).toBe(MOCK_RAW_TEXT)
    })

    it('stays in the editor with the error and the draft when saving fails', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      mockPut.mockRejectedValue(new Error('boom'))
      await startEdit(wrapper)
      await wrapper.find('textarea').setValue('*/kept')
      await button(wrapper, 'Save').trigger('click')
      await flushPromises()

      expect(wrapper.find('.form-error').exists()).toBe(true)
      expect(wrapper.find<HTMLTextAreaElement>('textarea').element.value).toBe('*/kept')

      // The saved list is untouched.
      await button(wrapper, 'Cancel').trigger('click')
      expect(listItems(wrapper).map((i) => i.text)).toEqual(['node_modules', '__pycache__'])
    })

    it('disables Save and Cancel while the save is in flight', async () => {
      const wrapper = await mountWith(MOCK_RAW_TEXT)
      mockPut.mockReturnValue(new Promise(() => {}))
      await startEdit(wrapper)
      await button(wrapper, 'Save').trigger('click')
      await flushPromises()

      expect(button(wrapper, 'Saving...').attributes('disabled')).toBeDefined()
      expect(button(wrapper, 'Cancel').attributes('disabled')).toBeDefined()
    })

    it('warns about a bare relative pattern', async () => {
      const wrapper = await mountWith('.cache\n*/.cache')
      await startEdit(wrapper)
      const warnings = wrapper.find('[aria-label="Pattern warnings"]')
      expect(warnings.exists()).toBe(true)
      expect(warnings.text()).toContain('Line 1')
      expect(warnings.text()).toContain('"*/.cache"')
      expect(warnings.findAll('li')).toHaveLength(1)
    })

    it('updates the warnings as the draft is typed', async () => {
      const wrapper = await mountWith('*/.cache')
      await startEdit(wrapper)
      expect(wrapper.find('[aria-label="Pattern warnings"]').exists()).toBe(false)

      await wrapper.find('textarea').setValue('*/.cache\n# fine\ntmp\n.thumbnails')
      const warnings = wrapper.find('[aria-label="Pattern warnings"]')
      // "tmp" is spelled from the root, so only line 4 needs a warning.
      expect(warnings.findAll('li')).toHaveLength(1)
      expect(warnings.text()).toContain('Line 4')

      await wrapper.find('textarea').setValue('*/.thumbnails')
      expect(wrapper.find('[aria-label="Pattern warnings"]').exists()).toBe(false)
    })

    it('shows no warning when every pattern reaches all users', async () => {
      const wrapper = await mountWith('*/.cache\n/proc')
      await startEdit(wrapper)
      expect(wrapper.find('[aria-label="Pattern warnings"]').exists()).toBe(false)
    })
  })
})
