// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises, type DOMWrapper, type VueWrapper } from '@vue/test-utils'
import MergeAgentDialog from './MergeAgentDialog.vue'
import { apiClient } from '../api/client'
import BaseModal from './BaseModal.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    post: vi.fn().mockResolvedValue({ data: { merged: true } }),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
  extractBlobError: async (_e: unknown): Promise<string> => 'API error',
}))

interface AgentRow {
  id: number
  hostname: string
  display_name: string | null
  is_imported: boolean
}

const SOURCE: AgentRow = {
  id: 10,
  hostname: 'old-webserver',
  display_name: null,
  is_imported: true,
}

const ALL_AGENTS: AgentRow[] = [
  SOURCE,
  { id: 1, hostname: 'web-server-01', display_name: 'Web Server', is_imported: false },
  { id: 2, hostname: 'db-server-01', display_name: null, is_imported: false },
]

function mountDialog(): ReturnType<typeof mount> {
  return mount(MergeAgentDialog, {
    global: { stubs: { Teleport: true } },
    props: { source: SOURCE, allAgents: ALL_AGENTS },
    attachTo: document.body,
  })
}

// The source hostname is rendered with the same `input.mono` classes as the
// pattern field, so the pattern field is addressed by its placeholder.
function patternField(wrapper: VueWrapper): DOMWrapper<Element> {
  return wrapper.find('input[placeholder="e.g. myhost*"]')
}

function mergeButton(wrapper: VueWrapper): DOMWrapper<Element> {
  return wrapper.findAll('button').find((b) => b.text().includes('Merge'))!
}

async function clickMerge(wrapper: VueWrapper): Promise<void> {
  await mergeButton(wrapper).trigger('click')
}

describe('MergeAgentDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(apiClient.post).mockResolvedValue({ data: { merged: true } } as never)
  })

  it('renders dialog with source hostname', () => {
    const wrapper = mountDialog()
    const sourceInput = wrapper.find('input[disabled]')
    expect((sourceInput.element as HTMLInputElement).value).toBe('old-webserver')
  })

  it('renders Merge into select with non-imported agents only', () => {
    const wrapper = mountDialog()
    const select = wrapper.find('select')
    const options = select.findAll('option')
    expect(options.length).toBe(3)
    expect(options[1].text()).toContain('web-server-01')
    expect(options[2].text()).toContain('db-server-01')
  })

  it('Merge button is disabled when no target selected', () => {
    const wrapper = mountDialog()
    const mergeBtn = wrapper.findAll('button').find((b) => b.text().includes('Merge'))
    expect(mergeBtn?.attributes('disabled')).toBeDefined()
  })

  it('emits merged event on successful submit', async () => {
    const wrapper = mountDialog()
    const select = wrapper.find('select')
    await select.setValue('1')
    const mergeBtn = wrapper.findAll('button').find((b) => b.text().includes('Merge'))
    await mergeBtn?.trigger('click')
    await flushPromises()
    expect(wrapper.emitted('merged')).toBeTruthy()
  })

  it('emits cancel when Cancel is clicked', async () => {
    const wrapper = mountDialog()
    const cancelBtn = wrapper.findAll('button').find((b) => b.text() === 'Cancel')
    await cancelBtn?.trigger('click')
    expect(wrapper.emitted('cancel')).toBeTruthy()
  })

  it('displays agent hostname in target options', () => {
    const wrapper = mountDialog()
    const select = wrapper.find('select')
    const texts = select.findAll('option').map((o) => o.text())
    expect(texts.some((t) => t.includes('web-server-01'))).toBe(true)
  })

  it('displays display_name with hostname when set', () => {
    const wrapper = mountDialog()
    const select = wrapper.find('select')
    const texts = select.findAll('option').map((o) => o.text())
    expect(texts.some((t) => t.includes('Web Server'))).toBe(true)
  })

  // The pattern is what stops the same imported host being re-adopted as a
  // separate agent next time it appears, so it defaults on and is prefilled
  // from the source hostname.
  it('offers to save a pattern by default, prefilled from the source', () => {
    const wrapper = mountDialog()
    const checkbox = wrapper.find('input[type="checkbox"]')
    expect((checkbox.element as HTMLInputElement).checked).toBe(true)
    expect((patternField(wrapper).element as HTMLInputElement).value).toBe('old-webserver*')
  })

  it('hides the pattern field when the operator declines to save one', async () => {
    const wrapper = mountDialog()
    await wrapper.find('input[type="checkbox"]').setValue(false)
    expect(patternField(wrapper).exists()).toBe(false)
  })

  it('sends the edited pattern with the merge', async () => {
    const wrapper = mountDialog()

    await wrapper.find('select').setValue('1')
    await patternField(wrapper).setValue('  legacy-*  ')
    await clickMerge(wrapper)
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/web-server-01/merge-from/10',
      { create_pattern: 'legacy-*' },
      { params: {} },
    )
  })

  // A blank pattern is the same request as an unchecked box: the merge still
  // goes through, it just leaves no alias behind.
  it.each([
    [
      'the box is unchecked',
      async (w: VueWrapper) => w.find('input[type="checkbox"]').setValue(false),
    ],
    ['the pattern is blanked', async (w: VueWrapper) => patternField(w).setValue('   ')],
  ])('sends no pattern when %s', async (_name, decline) => {
    const wrapper = mountDialog()

    await wrapper.find('select').setValue('1')
    await decline(wrapper)
    await clickMerge(wrapper)
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/web-server-01/merge-from/10',
      {},
      { params: {} },
    )
  })

  // Escape and the backdrop reach the dialog through BaseModal's close event,
  // which has to land on the same cancel path as the footer button.
  it('treats a modal dismissal as a cancel', async () => {
    const wrapper = mountDialog()
    wrapper.findComponent(BaseModal).vm.$emit('close')
    await flushPromises()
    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })

  it('surfaces a failed merge and stays open', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('target busy'))
    const wrapper = mountDialog()

    await wrapper.find('select').setValue('1')
    await clickMerge(wrapper)
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toBe('API error')
    expect(wrapper.emitted('merged')).toBeUndefined()
    // The button is usable again so the operator can retry.
    expect(mergeButton(wrapper).attributes('disabled')).toBeUndefined()
  })
})
