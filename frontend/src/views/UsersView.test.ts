// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import UsersView from './UsersView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatDate: (v: string | null | undefined, fallback = '') => v ?? fallback,
}))

import { apiClient } from '../api/client'

interface User {
  id: number
  username: string
  role: string
  must_change_password: boolean
  created_at: string
  last_login_at: string | null
}

const mockUsers: User[] = [
  {
    id: 1,
    username: 'admin',
    role: 'admin',
    must_change_password: false,
    created_at: '2026-01-01T00:00:00Z',
    last_login_at: null,
  },
  {
    id: 2,
    username: 'operator1',
    role: 'user',
    must_change_password: false,
    created_at: '2026-01-02T00:00:00Z',
    last_login_at: null,
  },
  {
    id: 3,
    username: 'viewer1',
    role: 'user',
    must_change_password: false,
    created_at: '2026-01-03T00:00:00Z',
    last_login_at: null,
  },
]

const mockApiGet = apiClient.get as ReturnType<typeof vi.fn>

/**
 * A fresh copy per call: the view replaces rows in the array it got back when
 * a role is saved, so handing out the shared fixture would let one test's edit
 * show up as another test's starting state.
 */
function userRows(): User[] {
  return mockUsers.map((u) => ({ ...u }))
}

beforeEach(() => {
  vi.clearAllMocks()
  mockApiGet.mockImplementation(() => Promise.resolve({ data: userRows() }))
})

describe('UsersView', () => {
  it('renders user list after loading', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.text()).toContain('admin')
    expect(wrapper.text()).toContain('operator1')
    expect(wrapper.text()).toContain('viewer1')
  })

  // F-47: a failed load used to leave an empty table with nothing to explain
  // it, which reads as "no users" rather than "we could not ask".
  it('explains a failed load instead of showing an empty table', async () => {
    mockApiGet.mockRejectedValue(new Error('network down'))

    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.find('.error-banner').text()).toContain('network down')
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('offers the create dialog from the empty state', async () => {
    mockApiGet.mockResolvedValue({ data: [] })

    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.text()).toContain('No users yet')

    await wrapper.find('.empty-state button').trigger('click')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
  })

  it('renders a New button for creating users', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
  })

  it('shows "you" badge for the currently authenticated user', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 1, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    const own = wrapper.get('[data-user="admin"] .badge--info')
    expect(own.text()).toBe('You')
    expect(wrapper.findAll('.badge--info')).toHaveLength(1)
  })

  it('does not show "you" badge when no user matches', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.find('.badge--info').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('You')
  })

  it('shows role badge for each user', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    // Admins stand out in the list; every other role is plain classification.
    const badges = wrapper.findAll('.list-row-chips .badge')
    expect(badges.map((b) => b.text())).toEqual(['admin', 'user', 'user'])
    expect(badges[0].classes()).toContain('badge--accent')
    expect(badges[1].classes()).toContain('badge--neutral')
    expect(badges[2].classes()).toContain('badge--neutral')
  })

  // Editing a user moved from a dialog to the user's own page, so a row is a
  // link to it rather than a row of Edit and Delete buttons.
  it("links every row to the user's own page", async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    const rows = wrapper.findAll('a.list-row')
    expect(rows.map((r) => r.attributes('href'))).toEqual(['/users/1', '/users/2', '/users/3'])
    for (const row of rows) expect(row.findAll('button')).toHaveLength(0)
  })

  it('flags a user who must change their password', async () => {
    mockApiGet.mockResolvedValue({
      data: [{ ...mockUsers[1], must_change_password: true }],
    })
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.get('[data-user="operator1"] .list-row-sub').text()).toBe('Must change password')
  })

  it('opens create modal on New button click', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    await newButton!.trigger('click')
    await flushPromises()

    expect(document.body.textContent).toContain('New user')
  })

  describe('create dialog', () => {
    const ADMIN = { auth: { user: { id: 99, username: 'admin', role: 'admin' } } }

    async function render() {
      const wrapper = renderWithPlugins(UsersView, { storeState: ADMIN })
      await flushPromises()
      return wrapper
    }

    async function openCreate(wrapper: Awaited<ReturnType<typeof render>>) {
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('New'))!
        .trigger('click')
      await flushPromises()
    }

    it('creates a user from the filled form', async () => {
      const post = apiClient.post as ReturnType<typeof vi.fn>
      post.mockResolvedValue({ data: {} })

      const wrapper = await render()
      await openCreate(wrapper)

      await wrapper.find('#new-username').setValue('newcomer')
      await wrapper.find('#new-password').setValue('correct horse battery')
      await wrapper.find('#new-role').setValue('admin')
      await wrapper.find('form').trigger('submit')
      await flushPromises()

      expect(post).toHaveBeenCalledWith('/users', {
        username: 'newcomer',
        password: 'correct horse battery',
        role: 'admin',
      })
    })

    it('reports a create failure and keeps the dialog open', async () => {
      const post = apiClient.post as ReturnType<typeof vi.fn>
      post.mockRejectedValue(new Error('username taken'))

      const wrapper = await render()
      await openCreate(wrapper)
      await wrapper.find('#new-username').setValue('newcomer')
      await wrapper.find('#new-password').setValue('correct horse battery')
      await wrapper.find('form').trigger('submit')
      await flushPromises()

      expect(wrapper.find('.form-error').exists()).toBe(true)
      expect(wrapper.find('#new-username').exists()).toBe(true)
    })

    it('closes the create dialog on Cancel without posting', async () => {
      const post = apiClient.post as ReturnType<typeof vi.fn>
      const wrapper = await render()
      await openCreate(wrapper)

      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Cancel')!
        .trigger('click')
      await flushPromises()

      expect(post).not.toHaveBeenCalled()
      expect(wrapper.find('#new-username').exists()).toBe(false)
    })

    it('starts the create form empty each time it opens', async () => {
      const wrapper = await render()
      await openCreate(wrapper)
      await wrapper.find('#new-username').setValue('discarded')
      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Cancel')!
        .trigger('click')
      await flushPromises()

      await openCreate(wrapper)
      expect((wrapper.find('#new-username').element as HTMLInputElement).value).toBe('')
    })

    // Escape and the backdrop close the dialog through BaseModal, wired
    // separately from its own Cancel button. Editing and deleting a user
    // moved to the user's own page, so creating is the one dialog left here.
    it('closes the create dialog when it is dismissed', async () => {
      const wrapper = await render()
      await openCreate(wrapper)
      expect(openModals(wrapper)).toHaveLength(1)

      await dismissModal(wrapper)

      expect(openModals(wrapper)).toHaveLength(0)
      expect(apiClient.post).not.toHaveBeenCalled()
    })
  })
})
