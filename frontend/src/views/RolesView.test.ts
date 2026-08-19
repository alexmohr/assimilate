// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { clickButtonWithText, openModals, renderWithPlugins } from '../test-utils'
import RolesView from './RolesView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

import { apiClient } from '../api/client'

interface Role {
  id: number
  name: string
  is_seeded: boolean
  can_create_agent: boolean
  can_delete_agent: boolean
  can_delete_own_agent: boolean
  can_create_repo: boolean
  can_delete_repo: boolean
  can_delete_own_repo: boolean
  can_create_schedule: boolean
  can_delete_schedule: boolean
  can_delete_own_schedule: boolean
  can_manage_tags: boolean
  can_view_all_repos: boolean
  can_manage_tunnels: boolean
  can_upgrade_agent: boolean
}

function makeRole(id: number, name: string, isSeeded: boolean, allPerms: boolean): Role {
  return {
    id,
    name,
    is_seeded: isSeeded,
    can_create_agent: allPerms,
    can_delete_agent: allPerms,
    can_delete_own_agent: allPerms,
    can_create_repo: allPerms,
    can_delete_repo: allPerms,
    can_delete_own_repo: allPerms,
    can_create_schedule: allPerms,
    can_delete_schedule: allPerms,
    can_delete_own_schedule: allPerms,
    can_manage_tags: allPerms,
    can_view_all_repos: allPerms,
    can_manage_tunnels: allPerms,
    can_upgrade_agent: allPerms,
  }
}

const mockRoles: Role[] = [
  makeRole(1, 'admin', true, true),
  makeRole(2, 'operator', true, false),
  makeRole(3, 'viewer', true, false),
  makeRole(4, 'custom-role', false, false),
]

const mockApiGet = apiClient.get as ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  mockApiGet.mockResolvedValue({ data: mockRoles })
})

describe('RolesView', () => {
  it('renders built-in roles after loading', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    expect(wrapper.text()).toContain('admin')
    expect(wrapper.text()).toContain('operator')
    expect(wrapper.text()).toContain('viewer')
  })

  it('shows "built-in" badge for seeded roles', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const badges = wrapper.findAll('.badge--neutral')
    expect(badges.length).toBe(3)
    badges.forEach((b) => expect(b.text()).toBe('built-in'))
  })

  it('renders permission column headers', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    expect(wrapper.text()).toContain('Create Agent')
    expect(wrapper.text()).toContain('View All Repos')
    expect(wrapper.text()).toContain('Manage Tags')
  })

  it('shows permission count per role', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const counts = wrapper.findAll('.perm-count')
    expect(counts.length).toBeGreaterThan(0)
    expect(counts[0].text()).toContain('/13')
  })

  it('renders New button', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
  })

  it('shows permission indicators in the matrix', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const yes = wrapper.findAll('.perm-yes')
    const no = wrapper.findAll('.perm-no')
    expect(yes.length).toBeGreaterThan(0)
    expect(no.length).toBeGreaterThan(0)
  })

  it('renders the create role form when New is clicked', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const newButton = wrapper.findAll('button').find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
    await newButton!.trigger('click')

    expect(wrapper.find('form').exists()).toBe(true)
    expect(wrapper.find('input#create-role-name').exists()).toBe(true)
    const perms = wrapper.findAll('.perm-checkbox')
    expect(perms.length).toBe(13)
  })

  it('opens the edit modal and populates the form', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const editButtons = wrapper.findAll('button').filter((b) => b.text() === 'Edit')
    expect(editButtons.length).toBeGreaterThan(0)
    await editButtons[0].trigger('click')

    expect(wrapper.find('.modal-backdrop').exists()).toBe(true)
    expect(wrapper.text()).toContain('Edit Role')
    expect(wrapper.text()).toContain('admin')
  })

  it('rejects an empty name, then creates a role once the name field is filled in', async () => {
    const mockApiPost = apiClient.post as ReturnType<typeof vi.fn>
    mockApiPost.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    await clickButtonWithText(wrapper, 'New')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(wrapper.text()).toContain('Name is required')
    expect(mockApiPost).not.toHaveBeenCalled()

    await wrapper.find('#create-role-name').setValue('editor')
    const permCheckboxes = wrapper.findAll('.permissions-grid input[type="checkbox"]')
    await permCheckboxes[0]!.setValue(true)
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockApiPost).toHaveBeenCalledWith(
      '/roles',
      expect.objectContaining({ name: 'editor', can_create_agent: true }),
    )
  })

  it('edits a role via the edit modal, flipping a permission on', async () => {
    const mockApiPut = apiClient.put as ReturnType<typeof vi.fn>
    mockApiPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    // operator (id 2) starts with every permission false, so checking the box is a real 0->1 flip.
    const editButtons = wrapper.findAll('button').filter((b) => b.text() === 'Edit')
    await editButtons[1]!.trigger('click')

    const permCheckboxes = wrapper.findAll('.permissions-grid input[type="checkbox"]')
    await permCheckboxes[0]!.setValue(true)
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockApiPut).toHaveBeenCalledWith(
      '/roles/2',
      expect.objectContaining({ can_create_agent: true }),
    )
  })

  it('closes the create modal via the close button and the Cancel button', async () => {
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    await clickButtonWithText(wrapper, 'New')
    await wrapper.find('button.modal-close').trigger('click')
    await flushPromises()
    expect(wrapper.find('#create-role-name').exists()).toBe(false)

    await clickButtonWithText(wrapper, 'New')
    await clickButtonWithText(wrapper, 'Cancel')
    await flushPromises()
    expect(wrapper.find('#create-role-name').exists()).toBe(false)

    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('closes the edit modal via the close button and the Cancel button', async () => {
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()
    const editButton = wrapper.findAll('button').find((b) => b.text() === 'Edit')

    await editButton!.trigger('click')
    await wrapper.find('button.modal-close').trigger('click')
    await flushPromises()
    expect(wrapper.find('.permissions-grid').exists()).toBe(false)

    await editButton!.trigger('click')
    await clickButtonWithText(wrapper, 'Cancel')
    await flushPromises()
    expect(wrapper.find('.permissions-grid').exists()).toBe(false)

    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('deletes a non-seeded role via the confirm dialog', async () => {
    const mockApiDelete = apiClient.delete as ReturnType<typeof vi.fn>
    mockApiDelete.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    const deleteButtons = wrapper.findAll('button.btn-danger-text')
    const enabledDelete = deleteButtons.find((b) => b.attributes('disabled') === undefined)
    expect(enabledDelete).toBeDefined()
    await enabledDelete!.trigger('click')
    expect(wrapper.text()).toContain('custom-role')

    await wrapper.find('button.btn-danger').trigger('click')
    await flushPromises()

    expect(mockApiDelete).toHaveBeenCalledWith('/roles/4')
  })

  // A fresh install has no custom roles, so the empty state's own button is
  // the only way into the create dialog from that screen.
  it('opens the create dialog from the empty state', async () => {
    mockApiGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    expect(wrapper.find('table').exists()).toBe(false)
    await clickButtonWithText(wrapper, 'Create role')
    await flushPromises()

    expect(openModals(wrapper)).toHaveLength(1)
  })

  it('cancels the delete confirm dialog without deleting', async () => {
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    const deleteButtons = wrapper.findAll('button.btn-danger-text')
    const enabledDelete = deleteButtons.find((b) => b.attributes('disabled') === undefined)
    await enabledDelete!.trigger('click')

    await wrapper.find('button.modal-close').trigger('click')
    await flushPromises()

    expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
    expect(apiClient.delete).not.toHaveBeenCalled()
  })
})
