// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentDangerZone from './AgentDangerZone.vue'
import BaseModal from './BaseModal.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { delete: vi.fn(), put: vi.fn(), post: vi.fn() },
}))

const AGENT = { hostname: 'web-01', is_imported: false } as unknown as AgentRow
const IMPORTED = { hostname: 'legacy-01', is_imported: true } as unknown as AgentRow

function mount(agent: AgentRow = AGENT) {
  return renderWithPlugins(AgentDangerZone, { props: { agent } })
}

/** The dialogs teleport, so their buttons are found on the document body. */
function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no dialog button labelled "${label}"`)
  return match
}

describe('AgentDangerZone', () => {
  beforeEach(() => {
    vi.mocked(apiClient.delete)
      .mockReset()
      .mockResolvedValue({} as never)
    vi.mocked(apiClient.put)
      .mockReset()
      .mockResolvedValue({} as never)
    vi.mocked(apiClient.post)
      .mockReset()
      .mockResolvedValue({} as never)
  })

  it('offers only Delete Agent for a managed host', () => {
    const wrapper = mount()
    const headings = wrapper.findAll('.danger-heading').map((h) => h.text())
    expect(headings).toEqual(['Delete Agent'])
  })

  it('offers Hide and Delete Archives for an imported host instead', () => {
    const wrapper = mount(IMPORTED)
    const headings = wrapper.findAll('.danger-heading').map((h) => h.text())
    expect(headings).toEqual(['Hide Agent', 'Delete Archives & Remove'])
  })

  it('confirms before deleting a managed agent', async () => {
    const wrapper = mount()
    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('Permanently delete')
    expect(document.body.textContent).toContain('web-01')

    dialogButton('Delete Agent').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/web-01')
  })

  it('confirms before destroying an imported host archives', async () => {
    const wrapper = mount(IMPORTED)
    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()

    expect(apiClient.post).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('permanently destroy all borg archives')

    dialogButton('Delete Archives & Remove').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/legacy-01/delete-archives')
  })

  it('hides an imported agent without a confirmation, since it is reversible', async () => {
    const wrapper = mount(IMPORTED)
    await wrapper.find('.btn-ghost').trigger('click')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith('/agents/legacy-01/hide')
  })

  it('keeps the user on the page when a delete fails', async () => {
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('agent busy'))
    const wrapper = mount()
    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()
    dialogButton('Delete Agent').click()
    await flushPromises()

    // The button is released again rather than left disabled forever.
    expect(wrapper.find('.btn-danger').attributes('disabled')).toBeUndefined()
  })

  it('keeps the imported agent visible when hiding it fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent busy'))
    const wrapper = mount(IMPORTED)
    await wrapper.find('.btn-ghost').trigger('click')
    await flushPromises()

    expect(wrapper.find('.btn-ghost').attributes('disabled')).toBeUndefined()
  })

  it('deletes the archives of an imported agent on confirmation', async () => {
    const wrapper = mount(IMPORTED)
    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()
    dialogButton('Delete Archives & Remove').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/legacy-01/delete-archives')
  })

  it('releases the button when deleting the archives fails', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('repo locked'))
    const wrapper = mount(IMPORTED)
    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()
    dialogButton('Delete Archives & Remove').click()
    await flushPromises()

    expect(wrapper.find('.btn-danger').attributes('disabled')).toBeUndefined()
  })

  // Both dialogs guard something irreversible, so backing out has to be a
  // genuine no-op rather than a dismissal that still fired the request - by
  // the footer button and by Escape/backdrop, which arrive as BaseModal's
  // close event on a separate handler.
  const DIALOGS = [
    ['delete-agent', AGENT, 0],
    ['delete-archives', IMPORTED, 1],
  ] as const
  const DISMISSALS = [
    [
      'the Cancel button',
      (_w: ReturnType<typeof mount>, _i: number): void => dialogButton('Cancel').click(),
    ],
    [
      'a modal dismissal',
      (w: ReturnType<typeof mount>, i: number): void => {
        w.findAllComponents(BaseModal)[i].vm.$emit('close')
      },
    ],
  ] as const

  it.each(
    DIALOGS.flatMap(([name, agent, index]) =>
      DISMISSALS.map(
        ([how, dismiss]) => [`${name} dialog by ${how}`, agent, index, dismiss] as const,
      ),
    ),
  )('backs out of the %s without acting', async (_name, agent, index, dismiss) => {
    const wrapper = mount(agent)
    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()
    expect(document.body.querySelector('.modal-dialog')).not.toBeNull()

    dismiss(wrapper, index)
    await flushPromises()

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(apiClient.post).not.toHaveBeenCalled()
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })
})
