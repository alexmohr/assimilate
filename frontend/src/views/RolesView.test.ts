// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
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
  }
}

const mockRoles: Role[] = [
  makeRole(1, 'admin', true, true),
  makeRole(2, 'operator', true, false),
  makeRole(3, 'viewer', true, false),
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

    const badges = wrapper.findAll('.seeded-badge')
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
    expect(counts[0].text()).toContain('/12')
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

  it('opens and cancels the create modal', async () => {
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    await wrapper.find('button.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.find('.overlay').exists()).toBe(true)

    await wrapper.find('.overlay .btn-ghost').trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.overlay').length).toBe(0)
  })

  it('opens and cancels the edit modal', async () => {
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    const editBtns = wrapper.findAll('tbody .btn-ghost').filter((b) => b.text() === 'Edit')
    await editBtns[0].trigger('click')
    await flushPromises()

    expect(wrapper.find('.overlay').exists()).toBe(true)

    await wrapper.find('.overlay .btn-ghost').trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.overlay').length).toBe(0)
  })

  it('opens and cancels the delete modal on a non-seeded role', async () => {
    const customRole = makeRole(4, 'custom-role', false, false)
    vi.mocked(apiClient.get).mockResolvedValueOnce({ data: [customRole] })
    const wrapper = renderWithPlugins(RolesView)
    await flushPromises()

    await wrapper.find('.btn-danger-text').trigger('click')
    await flushPromises()

    expect(wrapper.find('.overlay').exists()).toBe(true)

    await wrapper.find('.overlay .btn-ghost').trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.overlay').length).toBe(0)
  })
})
