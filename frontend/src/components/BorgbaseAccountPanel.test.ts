// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { mockApiClientRw } from '../test-utils/sharedMocks'

vi.mock('../api/client', () => mockApiClientRw())

import { apiClient } from '../api/client'
import { renderWithPlugins } from '../test-utils'
import { dialogButton, findButton } from '../test-utils/dom'
import BorgbaseAccountPanel from './BorgbaseAccountPanel.vue'
import type { BorgbaseRepo } from '../types/generated'

const REPOS: BorgbaseRepo[] = [
  {
    id: 'ab12',
    name: 'laptop',
    ssh_host: 'ab12.repo.borgbase.com',
    quota_enabled: true,
    quota: 500,
    usage: 12.5,
    last_modified: null,
  },
  {
    id: 'cd34',
    name: 'nas',
    ssh_host: 'cd34.repo.borgbase.com',
    quota_enabled: false,
    quota: null,
    usage: null,
    last_modified: null,
  },
]

/** The server answers `/borgbase` with `tokenSet` and, when a token is set, lists `repos`. */
function serverState(tokenSet: boolean, repos: BorgbaseRepo[] = REPOS): void {
  vi.mocked(apiClient.get).mockImplementation(
    (url: string) =>
      Promise.resolve({ data: url === '/borgbase' ? { token_set: tokenSet } : repos }) as never,
  )
}

/** Whether a button with exactly this label is on the panel. */
function hasButton(wrapper: ReturnType<typeof renderWithPlugins>, label: string): boolean {
  return wrapper.findAll('button').some((b) => b.text().trim() === label)
}

async function mountPanel() {
  const wrapper = renderWithPlugins(BorgbaseAccountPanel, { props: { embedded: true } })
  await flushPromises()
  return wrapper
}

describe('BorgbaseAccountPanel', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('asks for a token before it offers anything else', async () => {
    serverState(false)
    const wrapper = await mountPanel()

    expect(wrapper.find('#borgbase-token').exists()).toBe(true)
    expect(wrapper.find('#borgbase-repo-name').exists()).toBe(false)
    expect(apiClient.get).not.toHaveBeenCalledWith('/borgbase/repos')
    expect(hasButton(wrapper, 'Remove token')).toBe(false)
  })

  it('shows a failed load instead of the form', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('down'))
    const wrapper = await mountPanel()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(wrapper.find('#borgbase-token').exists()).toBe(false)
  })

  it('checks and stores a new token, then lists the account', async () => {
    serverState(false)
    vi.mocked(apiClient.put).mockResolvedValue({ data: { token_set: true } } as never)
    const wrapper = await mountPanel()
    serverState(true)

    await wrapper.find('#borgbase-token').setValue('secret-token')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith('/borgbase/token', { token: 'secret-token' })
    expect(wrapper.findAll('[data-testid="borgbase-repo"]')).toHaveLength(2)
    expect(wrapper.find<HTMLInputElement>('#borgbase-token').element.value).toBe('')
  })

  it('keeps the entered token and says why when BorgBase refuses it', async () => {
    serverState(false)
    vi.mocked(apiClient.put).mockRejectedValue(new Error('refused'))
    const wrapper = await mountPanel()

    await wrapper.find('#borgbase-token').setValue('wrong')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.find<HTMLInputElement>('#borgbase-token').element.value).toBe('wrong')
    expect(wrapper.find('#borgbase-repo-name').exists()).toBe(false)
  })

  it('lists repositories with usage and quota as reported', async () => {
    serverState(true)
    const wrapper = await mountPanel()

    const rows = wrapper.findAll('[data-testid="borgbase-repo"]')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.text()).toContain('laptop')
    expect(rows[0]!.text()).toContain('ab12.repo.borgbase.com')
    expect(rows[0]!.text()).toContain('12.5')
    expect(rows[0]!.text()).toContain('500')
    expect(rows[1]!.text()).toContain('None')
    expect(rows[1]!.text()).toContain('Not enforced')
  })

  it('says so when the account has no repositories', async () => {
    serverState(true, [])
    const wrapper = await mountPanel()

    expect(wrapper.find('.empty-state').exists()).toBe(true)
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('shows why the repositories could not be listed', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) =>
      url === '/borgbase'
        ? (Promise.resolve({ data: { token_set: true } }) as never)
        : (Promise.reject(new Error('bad gateway')) as never),
    )
    const wrapper = await mountPanel()

    expect(wrapper.find('.error-banner').exists()).toBe(true)
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('creates a repository and shows how to add it', async () => {
    serverState(true)
    vi.mocked(apiClient.post).mockResolvedValue({
      data: {
        id: 'ef56',
        name: 'offsite',
        ssh_host: 'ef56.repo.borgbase.com',
        ssh_user: 'ef56',
        repo_path: './repo',
      },
    } as never)
    const wrapper = await mountPanel()

    expect(findButton(wrapper, /^Create repository$/).attributes('disabled')).toBeDefined()
    await wrapper.find('#borgbase-repo-name').setValue('  offsite  ')
    await wrapper.find('#borgbase-repo-region').setValue('us')
    await wrapper.find('#borgbase-repo-access').setValue('full')
    await wrapper.findAll('form')[1]!.trigger('submit')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/borgbase/repos', {
      name: 'offsite',
      region: 'us',
      access: 'full',
    })
    const notice = wrapper.get('[data-testid="borgbase-created"]').text()
    expect(notice).toContain('ef56.repo.borgbase.com')
    expect(notice).toContain('./repo')
    expect(wrapper.find<HTMLInputElement>('#borgbase-repo-name').element.value).toBe('')
  })

  it('shows why a repository could not be created and keeps the name', async () => {
    serverState(true)
    vi.mocked(apiClient.post).mockRejectedValue(new Error('name taken'))
    const wrapper = await mountPanel()

    await wrapper.find('#borgbase-repo-name').setValue('offsite')
    await wrapper.findAll('form')[1]!.trigger('submit')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.find('[data-testid="borgbase-created"]').exists()).toBe(false)
    expect(wrapper.find<HTMLInputElement>('#borgbase-repo-name').element.value).toBe('offsite')
  })

  it('removes the token after confirmation and goes back to asking for one', async () => {
    serverState(true)
    vi.mocked(apiClient.delete).mockResolvedValue({} as never)
    const wrapper = await mountPanel()

    await findButton(wrapper, /^Remove token$/).trigger('click')
    dialogButton('Remove token').click()
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/borgbase/token')
    expect(hasButton(wrapper, 'Remove token')).toBe(false)
    expect(wrapper.find('[data-testid="borgbase-repo"]').exists()).toBe(false)
  })

  it('keeps the token and shows the failure when removing it fails', async () => {
    serverState(true)
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('nope'))
    const wrapper = await mountPanel()

    await findButton(wrapper, /^Remove token$/).trigger('click')
    dialogButton('Remove token').click()
    await flushPromises()

    expect(hasButton(wrapper, 'Remove token')).toBe(true)
  })

  it('keeps the token when removing it is cancelled', async () => {
    serverState(true)
    const wrapper = await mountPanel()

    await findButton(wrapper, /^Remove token$/).trigger('click')
    dialogButton('Cancel').click()
    await flushPromises()

    expect(apiClient.delete).not.toHaveBeenCalled()
    expect(hasButton(wrapper, 'Remove token')).toBe(true)
    expect(document.body.querySelector('.modal-dialog')).toBeNull()
  })
})
