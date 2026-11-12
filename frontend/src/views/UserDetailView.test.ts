// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, type DOMWrapper, type VueWrapper } from '@vue/test-utils'
import { dismissModal, openModals, renderWithPlugins } from '../test-utils'
import { openMenu } from '../test-utils/overflowMenu'
import UserDetailView from './UserDetailView.vue'
import type { UserSection } from '../utils/userSettings'

vi.mock('../api/client')

vi.mock('../utils/format', () => ({
  formatDate: (v: string | null | undefined, fallback = '') => v ?? fallback,
}))

import { apiClient } from '../api/client'

/*
 * Editing a user used to be a tabbed dialog on the Users list. It became this
 * page (one collapsible section per former tab, each with its own Edit and
 * Save), so the dialog's tests live here now, driven through the page.
 */

const USERS = [
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
]

const ROLES = [
  { id: 10, name: 'operators' },
  { id: 11, name: 'auditors' },
]
const GROUPS = [{ id: 20, name: 'eu-west' }]
const REPOS = [{ id: 1, name: 'web-server-01 / daily', enabled: true }]

const ADMIN = { auth: { user: { id: 1, username: 'admin', role: 'admin' } } }

const mockGet = apiClient.get as ReturnType<typeof vi.fn>
const mockPut = apiClient.put as ReturnType<typeof vi.fn>
const mockDelete = apiClient.delete as ReturnType<typeof vi.fn>

function mockData(): void {
  mockGet.mockImplementation((url: string) => {
    if (url === '/roles') return Promise.resolve({ data: ROLES })
    if (url === '/groups') return Promise.resolve({ data: GROUPS })
    if (url === '/repos') return Promise.resolve({ data: REPOS })
    if (/\/users\/\d+\/roles$/.test(url)) return Promise.resolve({ data: [ROLES[0]] })
    if (/\/users\/\d+\/groups$/.test(url)) return Promise.resolve({ data: [] })
    if (/\/users\/\d+\/permissions$/.test(url)) return Promise.resolve({ data: [] })
    return Promise.resolve({ data: USERS.map((u) => ({ ...u })) })
  })
}

type Wrapper = VueWrapper

async function render(id = 2, section: UserSection = 'role'): Promise<Wrapper> {
  const wrapper = renderWithPlugins(UserDetailView, {
    props: { id: String(id) },
    storeState: ADMIN,
  }) as Wrapper
  await flushPromises()
  // Role starts open; any other section is opened from its head.
  if (section !== 'role') {
    await wrapper.get(`#acc-head-${section}`).trigger('click')
    await flushPromises()
  }
  return wrapper
}

/** One section's body, so its Edit and Save are not confused with another's. */
function pane(wrapper: Wrapper, section: UserSection): DOMWrapper<Element> {
  return wrapper.get(`#acc-body-${section}`)
}

async function click(scope: DOMWrapper<Element>, label: string): Promise<void> {
  const button = scope.findAll('button').find((b) => b.text().trim() === label)
  if (!button) throw new Error(`no "${label}" button`)
  await button.trigger('click')
  await flushPromises()
}

async function editSection(wrapper: Wrapper, section: UserSection): Promise<DOMWrapper<Element>> {
  await click(pane(wrapper, section), 'Edit')
  return pane(wrapper, section)
}

function checkbox(scope: DOMWrapper<Element>, label: string): DOMWrapper<HTMLInputElement> {
  const choice = scope.findAll('label.user-choice').find((l) => l.text().trim() === label)
  if (!choice) throw new Error(`no "${label}" choice`)
  return choice.get<HTMLInputElement>('input[type="checkbox"]')
}

describe('UserDetailView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockData()
    mockPut.mockResolvedValue({ data: {} })
    mockDelete.mockResolvedValue({ data: {} })
  })

  describe('role', () => {
    it('prefills the role editor with the user role', async () => {
      const wrapper = await render()
      const section = await editSection(wrapper, 'role')
      expect(section.get<HTMLSelectElement>('#user-edit-role').element.value).toBe('user')
    })

    it('promotes a user to admin', async () => {
      const wrapper = await render()
      const section = await editSection(wrapper, 'role')
      await section.get('#user-edit-role').setValue('admin')
      await click(section, 'Save')

      expect(mockPut).toHaveBeenCalledWith('/users/2/role', { role: 'admin' })
      // The page reflects the new role without a refetch.
      expect(pane(wrapper, 'role').find('#user-edit-role').exists()).toBe(false)
      expect(pane(wrapper, 'role').text()).toContain('Admin')
    })

    // Saving an unchanged role used to be blocked by a disabled button; now
    // Save just closes the editor, still without a request.
    it('sends nothing when the role was not changed', async () => {
      const wrapper = await render()
      const section = await editSection(wrapper, 'role')
      await click(section, 'Save')

      expect(mockPut).not.toHaveBeenCalled()
      expect(pane(wrapper, 'role').find('#user-edit-role').exists()).toBe(false)
    })

    it('reports a failed role change instead of claiming it saved', async () => {
      mockPut.mockRejectedValue(new Error('forbidden'))
      const wrapper = await render()
      const section = await editSection(wrapper, 'role')
      await section.get('#user-edit-role').setValue('admin')
      await click(section, 'Save')

      expect(pane(wrapper, 'role').find('.form-error').exists()).toBe(true)
      expect(pane(wrapper, 'role').find('#user-edit-role').exists()).toBe(true)
    })
  })

  describe('roles and groups', () => {
    it('checks the roles the user already holds and leaves the rest clear', async () => {
      const wrapper = await render(2, 'access')
      const section = await editSection(wrapper, 'access')

      expect(checkbox(section, 'operators').element.checked).toBe(true)
      expect(checkbox(section, 'auditors').element.checked).toBe(false)
      expect(checkbox(section, 'eu-west').element.checked).toBe(false)
    })

    it('toggles a role on and back off', async () => {
      const wrapper = await render(2, 'access')
      const section = await editSection(wrapper, 'access')

      await checkbox(section, 'auditors').setValue(true)
      expect(checkbox(pane(wrapper, 'access'), 'auditors').element.checked).toBe(true)

      await checkbox(pane(wrapper, 'access'), 'auditors').setValue(false)
      await flushPromises()
      expect(checkbox(pane(wrapper, 'access'), 'auditors').element.checked).toBe(false)
    })

    it('toggles a group membership', async () => {
      const wrapper = await render(2, 'access')
      const section = await editSection(wrapper, 'access')

      expect(checkbox(section, 'eu-west').element.checked).toBe(false)
      await checkbox(section, 'eu-west').setValue(true)
      expect(checkbox(pane(wrapper, 'access'), 'eu-west').element.checked).toBe(true)
    })

    it('saves the selected roles and groups', async () => {
      const wrapper = await render(2, 'access')
      const section = await editSection(wrapper, 'access')

      await checkbox(section, 'auditors').setValue(true)
      await checkbox(section, 'eu-west').setValue(true)
      await click(pane(wrapper, 'access'), 'Save')

      expect(mockPut).toHaveBeenCalledWith('/users/2/roles', {
        role_ids: [ROLES[0].id, ROLES[1].id],
      })
      expect(mockPut).toHaveBeenCalledWith('/users/2/groups', { group_ids: [GROUPS[0].id] })
      expect(pane(wrapper, 'access').text()).toContain('operators, auditors')
      expect(pane(wrapper, 'access').text()).toContain('eu-west')
    })

    it('reports a failed save and keeps the editor and the saved assignments', async () => {
      mockPut.mockRejectedValue(new Error('forbidden'))
      const wrapper = await render(2, 'access')
      const section = await editSection(wrapper, 'access')
      await checkbox(section, 'auditors').setValue(true)
      await click(pane(wrapper, 'access'), 'Save')

      expect(pane(wrapper, 'access').get('.form-error').text()).toBe(
        'Failed to save assignments: forbidden',
      )
      expect(checkbox(pane(wrapper, 'access'), 'auditors').element.checked).toBe(true)
    })
    it('keeps the half of a save that went through when the other half fails', async () => {
      mockPut.mockImplementation((url: string) =>
        url.endsWith('/groups') ? Promise.reject(new Error('forbidden')) : Promise.resolve({}),
      )
      const wrapper = await render(2, 'access')
      const section = await editSection(wrapper, 'access')
      await checkbox(section, 'auditors').setValue(true)
      await checkbox(section, 'eu-west').setValue(true)
      await click(pane(wrapper, 'access'), 'Save')

      expect(pane(wrapper, 'access').get('.form-error').text()).toBe(
        'Failed to save assignments: forbidden',
      )
      // The roles were stored, the group was not: that is what the page shows.
      await click(pane(wrapper, 'access'), 'Cancel')
      expect(pane(wrapper, 'access').text()).toContain('operators, auditors')
      expect(pane(wrapper, 'access').text()).not.toContain('eu-west')
    })
  })

  describe('repository permissions', () => {
    it('shows repository names', async () => {
      const wrapper = await render(2, 'permissions')
      expect(pane(wrapper, 'permissions').text()).toContain('web-server-01 / daily')
    })

    // Each repository writes its whole permission row back, so a toggle on
    // one flag must not silently clear the others.
    it('sends the full permission row when one flag is toggled', async () => {
      const wrapper = await render(2, 'permissions')
      const section = await editSection(wrapper, 'permissions')

      await section.get('input[aria-label="View on web-server-01 / daily"]').trigger('change')
      await click(pane(wrapper, 'permissions'), 'Save')

      expect(mockPut).toHaveBeenCalledWith('/repos/1/permissions/2', {
        can_view: true,
        can_backup: false,
        can_modify_schedules: false,
        can_extract: false,
        can_delete: false,
      })
    })

    it('drives each permission column independently', async () => {
      const COLUMNS = [
        ['View', 'can_view'],
        ['Backup', 'can_backup'],
        ['Schedules', 'can_modify_schedules'],
        ['Extract', 'can_extract'],
        ['Delete', 'can_delete'],
      ] as const
      const wrapper = await render(2, 'permissions')
      await editSection(wrapper, 'permissions')
      expect(pane(wrapper, 'permissions').findAll('input[type="checkbox"]')).toHaveLength(5)

      // Toggle one column at a time and save, so each save is the first to
      // carry that flag while every later one is still clear.
      for (const [i, [label, field]] of COLUMNS.entries()) {
        if (i > 0) await click(pane(wrapper, 'permissions'), 'Edit')
        await pane(wrapper, 'permissions')
          .get(`input[aria-label="${label} on web-server-01 / daily"]`)
          .trigger('change')
        await click(pane(wrapper, 'permissions'), 'Save')

        const body = mockPut.mock.calls.at(-1)![1] as Record<string, boolean>
        expect(body[field]).toBe(true)
        for (const [, later] of COLUMNS.slice(i + 1)) expect(body[later]).toBe(false)
      }
      expect(mockPut).toHaveBeenCalledTimes(5)
    })

    it('reports a failed permission save and keeps the editor open', async () => {
      mockPut.mockRejectedValue(new Error('repository is locked'))
      const wrapper = await render(2, 'permissions')
      const section = await editSection(wrapper, 'permissions')
      await section.get('input[aria-label="View on web-server-01 / daily"]').trigger('change')
      await click(pane(wrapper, 'permissions'), 'Save')

      expect(pane(wrapper, 'permissions').get('.form-error').text()).toBe(
        'Failed to save permissions: repository is locked',
      )
      expect(
        pane(wrapper, 'permissions').get<HTMLInputElement>(
          'input[aria-label="View on web-server-01 / daily"]',
        ).element.checked,
      ).toBe(true)
    })

    it('keeps the repositories that saved when another one fails, and retries only the rest', async () => {
      const repos = [...REPOS, { id: 2, name: 'db-01 / nightly', enabled: true }]
      mockGet.mockImplementation((url: string) => {
        if (url === '/repos') return Promise.resolve({ data: repos })
        if (/\/users\/\d+\/permissions$/.test(url)) return Promise.resolve({ data: [] })
        if (url === '/roles' || url === '/groups' || /\/users\/\d+\/(roles|groups)$/.test(url))
          return Promise.resolve({ data: [] })
        return Promise.resolve({ data: USERS.map((u) => ({ ...u })) })
      })
      mockPut.mockImplementation((url: string) =>
        url === '/repos/2/permissions/2'
          ? Promise.reject(new Error('repository is locked'))
          : Promise.resolve({}),
      )
      const wrapper = await render(2, 'permissions')
      const section = await editSection(wrapper, 'permissions')
      await section.get('input[aria-label="View on web-server-01 / daily"]').trigger('change')
      await section.get('input[aria-label="View on db-01 / nightly"]').trigger('change')
      await click(pane(wrapper, 'permissions'), 'Save')

      expect(pane(wrapper, 'permissions').get('.form-error').text()).toBe(
        'Failed to save permissions: repository is locked',
      )
      expect(mockPut).toHaveBeenCalledTimes(2)

      mockPut.mockClear()
      mockPut.mockResolvedValue({})
      await click(pane(wrapper, 'permissions'), 'Save')
      expect(mockPut).toHaveBeenCalledTimes(1)
      expect(mockPut).toHaveBeenCalledWith(
        '/repos/2/permissions/2',
        expect.objectContaining({ can_view: true }),
      )
    })

    it('sends nothing for a repository whose permissions did not change', async () => {
      const wrapper = await render(2, 'permissions')
      await editSection(wrapper, 'permissions')
      await click(pane(wrapper, 'permissions'), 'Save')

      expect(mockPut).not.toHaveBeenCalled()
    })
  })

  describe('password', () => {
    it('sets a new password', async () => {
      const wrapper = await render(2, 'password')
      const section = await editSection(wrapper, 'password')
      await section.get('#user-edit-password').setValue('a much longer secret')
      await click(section, 'Save')

      expect(mockPut).toHaveBeenCalledWith('/users/2/password', {
        password: 'a much longer secret',
      })
    })

    it('reports a failed password reset and keeps the editor open', async () => {
      mockPut.mockRejectedValue(new Error('password too common'))
      const wrapper = await render(2, 'password')
      const section = await editSection(wrapper, 'password')
      await section.get('#user-edit-password').setValue('a much longer secret')
      await click(section, 'Save')

      expect(pane(wrapper, 'password').get('.form-error').text()).toBe(
        'Failed to reset password: password too common',
      )
      expect(pane(wrapper, 'password').find('#user-edit-password').exists()).toBe(true)
    })

    it('refuses a password that is too short without sending it', async () => {
      const wrapper = await render(2, 'password')
      const section = await editSection(wrapper, 'password')
      await section.get('#user-edit-password').setValue('short')
      await click(section, 'Save')

      expect(mockPut).not.toHaveBeenCalled()
      expect(pane(wrapper, 'password').get('.form-error').text()).toContain('at least 8')
    })
  })

  describe('delete', () => {
    async function openDelete(wrapper: Wrapper): Promise<void> {
      await openMenu(wrapper)
      await wrapper
        .findAll('.overflow-menu-item')
        .find((i) => i.text().trim() === 'Delete user')!
        .trigger('click')
      await flushPromises()
    }

    it('names the user it is about to delete and leaves for the list on confirm', async () => {
      const wrapper = await render()
      const push = vi.spyOn(
        (wrapper.vm as unknown as { $router: { push: (to: string) => Promise<void> } }).$router,
        'push',
      )
      await openDelete(wrapper)
      expect(openModals(wrapper)[0].text()).toContain('operator1')

      await click(wrapper.get('.modal-dialog') as DOMWrapper<Element>, 'Delete')

      expect(mockDelete).toHaveBeenCalledWith('/users/2')
      expect(push).toHaveBeenCalledWith('/users')
    })

    it('shows why a delete failed and stays on the user', async () => {
      mockDelete.mockRejectedValue(new Error('user owns schedules'))
      const wrapper = await render()
      const push = vi.spyOn(
        (wrapper.vm as unknown as { $router: { push: (to: string) => Promise<void> } }).$router,
        'push',
      )
      await openDelete(wrapper)
      await click(wrapper.get('.modal-dialog') as DOMWrapper<Element>, 'Delete')

      expect(mockDelete).toHaveBeenCalledWith('/users/2')
      expect(openModals(wrapper)).toHaveLength(1)
      expect(wrapper.get('.modal-dialog .form-error').text()).toBe(
        'Failed to delete user: user owns schedules',
      )
      expect(push).not.toHaveBeenCalled()
    })

    it('keeps the user when the delete is cancelled', async () => {
      const wrapper = await render()
      await openDelete(wrapper)
      await click(wrapper.get('.modal-dialog') as DOMWrapper<Element>, 'Cancel')

      expect(mockDelete).not.toHaveBeenCalled()
      expect(openModals(wrapper)).toHaveLength(0)
    })

    // Escape and the backdrop close the dialog through BaseModal, wired
    // separately from its own Cancel button.
    it('closes the delete dialog when it is dismissed', async () => {
      const wrapper = await render()
      await openDelete(wrapper)
      expect(openModals(wrapper)).toHaveLength(1)

      await dismissModal(wrapper)

      expect(openModals(wrapper)).toHaveLength(0)
      expect(mockDelete).not.toHaveBeenCalled()
    })
  })

  // Cancel closes a section's editor and throws its draft away, without a
  // request; the next Edit starts from what is saved.
  it.each([
    ['role', '#user-edit-role'],
    ['access', 'label.user-choice input'],
    ['permissions', 'input[aria-label="View on web-server-01 / daily"]'],
    ['password', '#user-edit-password'],
  ] as const)('cancels an edit of the %s section without saving', async (name, field) => {
    const wrapper = await render(2, name)
    const section = await editSection(wrapper, name)
    expect(section.find(field).exists()).toBe(true)

    await click(section, 'Cancel')

    expect(pane(wrapper, name).find(field).exists()).toBe(false)
    expect(
      pane(wrapper, name)
        .findAll('button')
        .map((b) => b.text().trim()),
    ).toContain('Edit')
    expect(mockPut).not.toHaveBeenCalled()
  })

  it('discards a cancelled role and groups draft', async () => {
    const wrapper = await render(2, 'access')
    const section = await editSection(wrapper, 'access')
    await checkbox(section, 'auditors').setValue(true)
    await click(pane(wrapper, 'access'), 'Cancel')

    expect(pane(wrapper, 'access').text()).not.toContain('auditors')
    const reopened = await editSection(wrapper, 'access')
    expect(checkbox(reopened, 'auditors').element.checked).toBe(false)
  })

  it('loads the other user when the page is reused for a new id', async () => {
    const wrapper = await render(2)
    expect(wrapper.text()).toContain('operator1')
    expect(wrapper.find('.badge--info').exists()).toBe(false)

    await wrapper.setProps({ id: '1' })
    await flushPromises()

    expect(mockGet).toHaveBeenCalledWith('/users/1/roles')
    expect(wrapper.find('.badge--info').text()).toBe('You')
    expect(wrapper.text()).not.toContain('operator1')
  })

  // You cannot demote or delete yourself, and change your own password on
  // your profile.
  it('shows your own account read-only', async () => {
    const wrapper = await render(1)
    expect(wrapper.find('.badge--info').text()).toBe('You')
    expect(wrapper.find('.overflow-toggle').exists()).toBe(false)
    expect(
      pane(wrapper, 'role')
        .findAll('button')
        .map((b) => b.text().trim()),
    ).not.toContain('Edit')
  })
})
