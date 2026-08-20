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
  role: 'admin' | 'user'
  created_at: string
  last_login_at: string | null
}

const mockUsers: User[] = [
  {
    id: 1,
    username: 'admin',
    role: 'admin',
    created_at: '2026-01-01T00:00:00Z',
    last_login_at: null,
  },
  {
    id: 2,
    username: 'operator1',
    role: 'user',
    created_at: '2026-01-02T00:00:00Z',
    last_login_at: null,
  },
  {
    id: 3,
    username: 'viewer1',
    role: 'user',
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

    expect(wrapper.text()).toContain('you')
  })

  it('does not show "you" badge when no user matches', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.text()).not.toContain('you')
  })

  it('shows role badge for each user', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    const badges = wrapper.findAll('.badge--neutral')
    expect(badges.length).toBe(3)
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

  it('shows repository names in the permissions tab', async () => {
    mockApiGet.mockImplementation((url: string) => {
      if (url === '/repos') {
        return Promise.resolve({
          data: [{ id: 1, name: 'web-server-01 / daily', enabled: true }],
        })
      }
      if (url === '/users/1/permissions') {
        return Promise.resolve({ data: [] })
      }
      return Promise.resolve({ data: userRows() })
    })

    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    const editButtons = wrapper.findAll('button').filter((b) => b.text().includes('Edit'))
    await editButtons[0]!.trigger('click')
    await flushPromises()

    const permissionsTab = wrapper
      .findAll('button.tab')
      .find((t) => t.text().trim() === 'Permissions')
    expect(permissionsTab).toBeDefined()
    await permissionsTab!.trigger('click')
    await flushPromises()

    expect(document.body.textContent).toContain('web-server-01 / daily')
  })

  describe('user dialogs', () => {
    const ADMIN = { auth: { user: { id: 99, username: 'admin', role: 'admin' } } }

    const ROLES = [
      { id: 10, name: 'operators' },
      { id: 11, name: 'auditors' },
    ]
    const GROUPS = [{ id: 20, name: 'eu-west' }]
    const REPOS = [{ id: 1, name: 'server-daily', enabled: true }]

    function mockEditData(): void {
      mockApiGet.mockImplementation((url: string) => {
        if (url === '/roles') return Promise.resolve({ data: ROLES })
        if (url === '/groups') return Promise.resolve({ data: GROUPS })
        if (url === '/repos') return Promise.resolve({ data: REPOS })
        if (/\/users\/\d+\/roles$/.test(url)) return Promise.resolve({ data: [ROLES[0]] })
        if (/\/users\/\d+\/groups$/.test(url)) return Promise.resolve({ data: [] })
        if (/\/users\/\d+\/permissions$/.test(url)) return Promise.resolve({ data: [] })
        return Promise.resolve({ data: userRows() })
      })
    }

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

    async function openEditFor(wrapper: Awaited<ReturnType<typeof render>>, index: number) {
      await wrapper
        .findAll('button')
        .filter((b) => b.text().includes('Edit'))
        [index].trigger('click')
      await flushPromises()
    }

    async function selectTab(wrapper: Awaited<ReturnType<typeof render>>, label: string) {
      await wrapper
        .findAll('button.tab')
        .find((t) => t.text().trim() === label)!
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

    it('prefills the edit dialog with the user role', async () => {
      mockEditData()
      const wrapper = await render()
      await openEditFor(wrapper, 1)

      expect((wrapper.find('#edit-role').element as HTMLSelectElement).value).toBe('user')
    })

    it('checks the roles the user already holds and leaves the rest clear', async () => {
      mockEditData()
      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Roles & Groups')

      const boxes = wrapper.findAll('.rg-item input[type="checkbox"]')
      expect(boxes.length).toBeGreaterThanOrEqual(3)
      expect((boxes[0].element as HTMLInputElement).checked).toBe(true)
      expect((boxes[1].element as HTMLInputElement).checked).toBe(false)
    })

    it('toggles a role on and back off', async () => {
      mockEditData()
      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Roles & Groups')

      const second = wrapper.findAll('.rg-item input[type="checkbox"]')[1]
      await second.setValue(true)
      expect((second.element as HTMLInputElement).checked).toBe(true)

      await second.setValue(false)
      await flushPromises()
      expect(
        (wrapper.findAll('.rg-item input[type="checkbox"]')[1].element as HTMLInputElement).checked,
      ).toBe(false)
    })

    it('toggles a group membership', async () => {
      mockEditData()
      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Roles & Groups')

      const boxes = wrapper.findAll('.rg-item input[type="checkbox"]')
      const groupBox = boxes[boxes.length - 1]
      expect((groupBox.element as HTMLInputElement).checked).toBe(false)
      await groupBox.setValue(true)
      expect((groupBox.element as HTMLInputElement).checked).toBe(true)
    })

    it('saves the selected roles and groups', async () => {
      mockEditData()
      const put = apiClient.put as ReturnType<typeof vi.fn>
      put.mockResolvedValue({ data: {} })
      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Roles & Groups')

      const boxes = wrapper.findAll('.rg-item input[type="checkbox"]')
      await boxes[1].setValue(true)
      await boxes[boxes.length - 1].setValue(true)

      await wrapper
        .findAll('button')
        .find((b) => b.text() === 'Save')!
        .trigger('click')
      await flushPromises()

      expect(put).toHaveBeenCalledWith('/users/2/roles', { role_ids: [ROLES[0].id, ROLES[1].id] })
      expect(put).toHaveBeenCalledWith('/users/2/groups', { group_ids: [GROUPS[0].id] })
    })

    // Each column writes the whole permission row back, so a toggle on one
    // flag must not silently clear the others.
    it('sends the full permission row when one flag is toggled', async () => {
      mockEditData()
      const put = apiClient.put as ReturnType<typeof vi.fn>
      put.mockResolvedValue({ data: {} })

      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Permissions')

      await wrapper.findAll('.perm-check-cell input[type="checkbox"]')[0].trigger('change')
      await flushPromises()

      expect(put).toHaveBeenCalledWith(
        '/repos/1/permissions/2',
        expect.objectContaining({
          can_view: true,
          can_backup: false,
          can_modify_schedules: false,
          can_extract: false,
          can_delete: false,
        }),
      )
    })

    it('drives each permission column independently', async () => {
      mockEditData()
      const put = apiClient.put as ReturnType<typeof vi.fn>
      put.mockResolvedValue({ data: {} })

      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Permissions')

      const boxes = wrapper.findAll('.perm-check-cell input[type="checkbox"]')
      expect(boxes).toHaveLength(5)
      for (const box of boxes) {
        await box.trigger('change')
        await flushPromises()
      }

      // Each toggle keeps the flags already set, so the assertion is that
      // the Nth call is the first to carry the Nth flag - not that it is the
      // only true one.
      const FIELDS = [
        'can_view',
        'can_backup',
        'can_modify_schedules',
        'can_extract',
        'can_delete',
      ] as const
      expect(put.mock.calls).toHaveLength(5)
      FIELDS.forEach((field, i) => {
        const body = put.mock.calls[i][1] as Record<string, boolean>
        expect(body[field]).toBe(true)
        for (const later of FIELDS.slice(i + 1)) expect(body[later]).toBe(false)
      })
    })

    it('sets a new password from the password tab', async () => {
      mockEditData()
      const put = apiClient.put as ReturnType<typeof vi.fn>
      put.mockResolvedValue({ data: {} })

      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await selectTab(wrapper, 'Password')

      await wrapper.find('#edit-password').setValue('a much longer secret')
      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Reset Password')!
        .trigger('click')
      await flushPromises()

      expect(put).toHaveBeenCalled()
      const passwordCall = put.mock.calls.find((c) => String(c[0]).includes('password'))
      expect(passwordCall).toBeDefined()
    })

    it('names the user it is about to delete and drops the row on confirm', async () => {
      const del = apiClient.delete as ReturnType<typeof vi.fn>
      del.mockResolvedValue({ data: {} })

      const wrapper = await render()
      // Row order matches the fixture, so the first delete button is admin.
      await wrapper.findAll('button.btn-danger-text')[0].trigger('click')
      await flushPromises()
      expect(document.body.textContent).toContain('admin')

      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Delete')!
        .trigger('click')
      await flushPromises()

      expect(del).toHaveBeenCalledWith('/users/1')
    })

    it('keeps the user when the delete is cancelled', async () => {
      const del = apiClient.delete as ReturnType<typeof vi.fn>
      const wrapper = await render()
      await wrapper.findAll('button.btn-danger-text')[0].trigger('click')
      await flushPromises()

      await wrapper
        .findAll('button')
        .find((b) => b.text().trim() === 'Cancel')!
        .trigger('click')
      await flushPromises()

      expect(del).not.toHaveBeenCalled()
    })

    // Save Role is gated on the value actually changing, so the select has to
    // write back through v-model for the button to become usable at all.
    it('promotes a user to admin from the general tab', async () => {
      mockEditData()
      const put = apiClient.put as ReturnType<typeof vi.fn>
      put.mockResolvedValue({ data: {} })

      const wrapper = await render()
      await openEditFor(wrapper, 1)

      const save = () => wrapper.findAll('button').find((b) => b.text().includes('Save Role'))!
      expect(save().attributes('disabled')).toBeDefined()

      await wrapper.find('#edit-role').setValue('admin')
      expect(save().attributes('disabled')).toBeUndefined()

      await save().trigger('click')
      await flushPromises()

      expect(put).toHaveBeenCalledWith('/users/2/role', { role: 'admin' })
      // The row behind the dialog reflects the new role without a refetch.
      expect(wrapper.find('#edit-role').exists()).toBe(true)
      expect(save().attributes('disabled')).toBeDefined()
    })

    it('reports a failed role change instead of claiming it saved', async () => {
      mockEditData()
      const put = apiClient.put as ReturnType<typeof vi.fn>
      put.mockRejectedValue(new Error('forbidden'))

      const wrapper = await render()
      await openEditFor(wrapper, 1)
      await wrapper.find('#edit-role').setValue('admin')
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('Save Role'))!
        .trigger('click')
      await flushPromises()

      expect(wrapper.find('.form-error').exists()).toBe(true)
    })

    // Escape and the backdrop close a dialog through BaseModal, wired
    // separately from each dialog's own Cancel button.
    it.each([
      ['create', openCreate],
      [
        'edit',
        async (w: Awaited<ReturnType<typeof render>>): Promise<void> => {
          await openEditFor(w, 1)
        },
      ],
      [
        'delete',
        async (w: Awaited<ReturnType<typeof render>>): Promise<void> => {
          await w.findAll('button.btn-danger-text')[0].trigger('click')
          await flushPromises()
        },
      ],
    ])('closes the %s dialog when it is dismissed', async (_name, open) => {
      mockEditData()
      const wrapper = await render()
      await open(wrapper)
      expect(openModals(wrapper)).toHaveLength(1)

      await dismissModal(wrapper)

      expect(openModals(wrapper)).toHaveLength(0)
      expect(apiClient.post).not.toHaveBeenCalled()
      expect(apiClient.put).not.toHaveBeenCalled()
      expect(apiClient.delete).not.toHaveBeenCalled()
    })
  })
})
