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

  it('closes the create modal when Cancel is clicked', async () => {
    const wrapper = renderWithPlugins(RolesView)

    await flushPromises()

    const newButton = wrapper.findAll('button').find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
    await newButton!.trigger('click')

    expect(wrapper.find('.overlay').exists()).toBe(true)

    const cancelBtn = wrapper.findAll('button').find((b) => b.text() === 'Cancel')
    expect(cancelBtn).toBeDefined()
    await cancelBtn!.trigger('click')

    expect(wrapper.find('.overlay').exists()).toBe(false)
  })
})
