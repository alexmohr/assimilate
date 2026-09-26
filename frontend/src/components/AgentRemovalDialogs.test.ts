// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentRemovalDialogs from './AgentRemovalDialogs.vue'
import BaseModal from './BaseModal.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { delete: vi.fn(), put: vi.fn(), post: vi.fn() },
}))

const AGENT = { hostname: 'web-01', is_imported: false } as unknown as AgentRow
const IMPORTED = { hostname: 'legacy-01', is_imported: true } as unknown as AgentRow

function mount(agent: AgentRow = AGENT) {
  return renderWithPlugins(AgentRemovalDialogs, { props: { agent } })
}

/** The header's overflow menu drives these through the exposed functions. */
interface Exposed {
  requestDelete: () => void
  hide: () => Promise<void>
  requestDeleteArchives: () => void
}

function exposed(wrapper: ReturnType<typeof mount>): Exposed {
  return wrapper.vm as unknown as Exposed
}

/** Opens the host's destructive dialog: Delete agent, or Delete archives if imported. */
async function openDestructive(wrapper: ReturnType<typeof mount>, agent: AgentRow): Promise<void> {
  if (agent.is_imported) exposed(wrapper).requestDeleteArchives()
  else exposed(wrapper).requestDelete()
  await flushPromises()
}

function dialogConfirm(label: string): HTMLButtonElement {
  const match = [
    ...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog .btn-danger'),
  ].find((b) => b.textContent?.trim().startsWith(label))
  if (!match) throw new Error(`no confirm button labelled "${label}"`)
  return match
}

/** The dialogs teleport, so their buttons are found on the document body. */
function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no dialog button labelled "${label}"`)
  return match
}

describe('AgentRemovalDialogs', () => {
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

  it('renders nothing until an action is requested', () => {
    mount()
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })

  it('confirms before deleting a managed agent', async () => {
    const wrapper = mount()
    await openDestructive(wrapper, AGENT)

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('Permanently delete')
    expect(document.body.textContent).toContain('web-01')
    // The settings pane used to say this; the dialog is all that is left.
    expect(document.body.textContent).toContain('Archives already written to a repository')

    dialogButton('Delete agent').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/web-01', { params: {} })
  })

  it('confirms before destroying an imported host archives', async () => {
    const wrapper = mount(IMPORTED)
    await openDestructive(wrapper, IMPORTED)

    expect(apiClient.post).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('permanently destroy all borg archives')

    dialogButton('Delete archives and remove').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/legacy-01/delete-archives',
      {},
      { params: {} },
    )
  })

  it('hides an imported agent without a confirmation, since it is reversible', async () => {
    const wrapper = mount(IMPORTED)
    await exposed(wrapper).hide()
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith('/agents/legacy-01/hide', {}, { params: {} })
  })

  it('keeps the user on the page when a delete fails', async () => {
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('agent busy'))
    const wrapper = mount()
    await openDestructive(wrapper, AGENT)
    dialogButton('Delete agent').click()
    await flushPromises()

    // The button is released again rather than left disabled forever.
    expect(dialogConfirm('Delete agent').disabled).toBe(false)
  })

  it('lets a failed hide be retried', async () => {
    vi.mocked(apiClient.put).mockRejectedValueOnce(new Error('agent busy'))
    const wrapper = mount(IMPORTED)
    await exposed(wrapper).hide()
    await exposed(wrapper).hide()

    expect(apiClient.put).toHaveBeenCalledTimes(2)
  })

  // There is no button left to disable, so the guard is in the function.
  it('ignores a second hide while the first is in flight', async () => {
    let release!: () => void
    vi.mocked(apiClient.put).mockReturnValue(
      new Promise((resolve) => {
        release = () => resolve({} as never)
      }) as never,
    )
    const wrapper = mount(IMPORTED)
    const first = exposed(wrapper).hide()
    await exposed(wrapper).hide()
    release()
    await first

    expect(apiClient.put).toHaveBeenCalledTimes(1)
  })

  it('deletes the archives of an imported agent on confirmation', async () => {
    const wrapper = mount(IMPORTED)
    await openDestructive(wrapper, IMPORTED)
    dialogButton('Delete archives and remove').click()
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith(
      '/agents/legacy-01/delete-archives',
      {},
      { params: {} },
    )
  })

  it('releases the button when deleting the archives fails', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('repo locked'))
    const wrapper = mount(IMPORTED)
    await openDestructive(wrapper, IMPORTED)
    dialogButton('Delete archives and remove').click()
    await flushPromises()

    expect(dialogConfirm('Delete archives and remove').disabled).toBe(false)
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
    await openDestructive(wrapper, agent)
    expect(document.body.querySelector('.modal-dialog')).not.toBeNull()

    dismiss(wrapper, index)
    await flushPromises()

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(apiClient.post).not.toHaveBeenCalled()
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })
})
