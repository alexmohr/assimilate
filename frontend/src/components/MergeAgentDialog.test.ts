// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import MergeAgentDialog from './MergeAgentDialog.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    post: vi.fn().mockResolvedValue({ data: { merged: true } }),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
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
    props: { source: SOURCE, allAgents: ALL_AGENTS },
    attachTo: document.body,
  })
}

describe('MergeAgentDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
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
    await select.setValue('web-server-01')
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
})
