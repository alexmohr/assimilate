// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw, mockErrorUtilsPassthrough } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())
vi.mock('../utils/error', () => mockErrorUtilsPassthrough())

import { renderWithPlugins } from '../test-utils'
import { dialogButton } from '../test-utils/dom'
import { apiClient } from '../api/client'
import RepoPassphraseDialog from './RepoPassphraseDialog.vue'

function mount(open = true): ReturnType<typeof renderWithPlugins> {
  return renderWithPlugins(RepoPassphraseDialog, { props: { open, repoId: 12 } })
}

function input(): HTMLInputElement {
  const el = document.body.querySelector<HTMLInputElement>('#repo-set-passphrase')
  if (!el) throw new Error('no passphrase input')
  return el
}

async function type(value: string): Promise<void> {
  input().value = value
  input().dispatchEvent(new Event('input'))
  await flushPromises()
}

async function submit(): Promise<void> {
  document.body.querySelector('.modal-dialog form')?.dispatchEvent(new Event('submit'))
  await flushPromises()
}

describe('RepoPassphraseDialog', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    vi.mocked(apiClient.put)
      .mockReset()
      .mockResolvedValue({} as never)
  })

  it('masks the passphrase as it is typed', () => {
    mount()
    expect(input().type).toBe('password')
  })

  it('cannot be saved empty', () => {
    mount()
    expect(dialogButton('Save passphrase').disabled).toBe(true)
  })

  it('sends the passphrase to the repository and reports success', async () => {
    const wrapper = mount()
    await type('correct horse')
    await submit()

    expect(apiClient.put).toHaveBeenCalledWith('/repos/12/passphrase', {
      passphrase: 'correct horse',
    })
    expect(wrapper.emitted('saved')).toHaveLength(1)
    expect(input().value).toBe('')
  })

  it('shows why borg rejected it and keeps the dialog open', async () => {
    vi.mocked(apiClient.put).mockRejectedValueOnce(
      new Error('passphrase is incorrect for this repository'),
    )
    const wrapper = mount()
    await type('wrong')
    await submit()

    expect(document.body.querySelector('.form-error')?.textContent).toContain('incorrect')
    expect(wrapper.emitted('saved')).toBeUndefined()
  })

  it('forgets what was typed once it is closed and reopened', async () => {
    const wrapper = mount()
    await type('half typed')
    await wrapper.setProps({ open: false })
    await wrapper.setProps({ open: true })
    await flushPromises()

    expect(input().value).toBe('')
  })

  it('asks the parent to close from the dialog close control too', async () => {
    const wrapper = mount()
    document.body.querySelector<HTMLButtonElement>('.modal-close')?.click()
    await flushPromises()
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('asks the parent to close on Cancel', async () => {
    const wrapper = mount()
    dialogButton('Cancel').click()
    await flushPromises()
    expect(wrapper.emitted('close')).toHaveLength(1)
  })
})
