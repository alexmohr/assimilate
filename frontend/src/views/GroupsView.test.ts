// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  cancelThenConfirmDelete,
  clickButtonWithText,
  dismissModal,
  openModals,
  renderWithPlugins,
} from '../test-utils'
import GroupsView from './GroupsView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

import { apiClient } from '../api/client'

interface Group {
  id: number
  name: string
  description: string | null
  created_at: string
}

interface GroupMember {
  user_id: number
}

const mockGroups: Group[] = [
  {
    id: 1,
    name: 'backend-team',
    description: 'Backend engineers',
    created_at: '2026-01-01T00:00:00Z',
  },
  { id: 2, name: 'data-team', description: 'Data scientists', created_at: '2026-01-02T00:00:00Z' },
]

const mockMembers: Record<number, GroupMember[]> = {
  1: [{ user_id: 1 }, { user_id: 2 }],
  2: [{ user_id: 3 }],
}

const mockUsers = [
  { id: 1, username: 'admin', role: 'admin' },
  { id: 2, username: 'operator1', role: 'user' },
  { id: 3, username: 'viewer1', role: 'user' },
]

const mockApiGet = apiClient.get as ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  mockApiGet.mockImplementation((url: string) => {
    if (url === '/groups') return Promise.resolve({ data: mockGroups })
    if (url === '/groups/1/members') return Promise.resolve({ data: mockMembers[1] })
    if (url === '/groups/2/members') return Promise.resolve({ data: mockMembers[2] })
    if (url === '/users') return Promise.resolve({ data: mockUsers })
    return Promise.resolve({ data: [] })
  })
})

describe('GroupsView', () => {
  it('renders group names after loading', async () => {
    const wrapper = renderWithPlugins(GroupsView)

    await flushPromises()

    expect(wrapper.text()).toContain('backend-team')
    expect(wrapper.text()).toContain('data-team')
  })

  it('shows member counts for each group', async () => {
    const wrapper = renderWithPlugins(GroupsView)

    await flushPromises()

    const rows = wrapper.findAll('tbody tr')
    expect(rows.length).toBe(2)

    const backendRow = rows[0]
    expect(backendRow.text()).toContain('2')

    const dataRow = rows[1]
    expect(dataRow.text()).toContain('1')
  })

  it('renders group descriptions', async () => {
    const wrapper = renderWithPlugins(GroupsView)

    await flushPromises()

    expect(wrapper.text()).toContain('Backend engineers')
    expect(wrapper.text()).toContain('Data scientists')
  })

  it('renders New button', async () => {
    const wrapper = renderWithPlugins(GroupsView)

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
  })

  it('renders Members button for each group', async () => {
    const wrapper = renderWithPlugins(GroupsView)

    await flushPromises()

    const memberButtons = wrapper.findAll('button').filter((b) => b.text() === 'Members')
    expect(memberButtons.length).toBe(2)
  })

  it('shows table headers for Name, Description, Members, Actions', async () => {
    const wrapper = renderWithPlugins(GroupsView)

    await flushPromises()

    const headers = wrapper.findAll('th')
    const headerText = headers.map((h) => h.text())
    expect(headerText).toContain('Name')
    expect(headerText).toContain('Description')
    expect(headerText).toContain('Members')
    expect(headerText).toContain('Actions')
  })

  it('rejects an empty name, then creates a group once the name field is filled in', async () => {
    const mockApiPost = apiClient.post as ReturnType<typeof vi.fn>
    mockApiPost.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()

    await clickButtonWithText(wrapper, 'New')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(wrapper.text()).toContain('Name is required')
    expect(mockApiPost).not.toHaveBeenCalled()

    await wrapper.find('#create-name').setValue('new-team')
    await wrapper.find('#create-desc').setValue('A new team')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockApiPost).toHaveBeenCalledWith('/groups', {
      name: 'new-team',
      description: 'A new team',
    })
  })

  it('closes the create modal via the close button and the Cancel button', async () => {
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()

    await clickButtonWithText(wrapper, 'New')
    await wrapper.find('button.modal-close').trigger('click')
    await flushPromises()
    expect(wrapper.find('#create-name').exists()).toBe(false)

    await clickButtonWithText(wrapper, 'New')
    await clickButtonWithText(wrapper, 'Cancel')
    await flushPromises()
    expect(wrapper.find('#create-name').exists()).toBe(false)

    expect(apiClient.post).not.toHaveBeenCalled()
  })

  it('edits a group via the edit modal, including its description', async () => {
    const mockApiPut = apiClient.put as ReturnType<typeof vi.fn>
    mockApiPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()

    const editButton = wrapper.findAll('button').find((b) => b.text() === 'Edit')
    await editButton!.trigger('click')

    await wrapper.find('#edit-name').setValue('backend-team-renamed')
    await wrapper.find('#edit-desc').setValue('Renamed description')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockApiPut).toHaveBeenCalledWith('/groups/1', {
      name: 'backend-team-renamed',
      description: 'Renamed description',
    })
  })

  it('closes the edit modal via the close button and the Cancel button', async () => {
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()
    const editButton = wrapper.findAll('button').find((b) => b.text() === 'Edit')

    await editButton!.trigger('click')
    await wrapper.find('button.modal-close').trigger('click')
    await flushPromises()
    expect(wrapper.find('#edit-name').exists()).toBe(false)

    await editButton!.trigger('click')
    await clickButtonWithText(wrapper, 'Cancel')
    await flushPromises()
    expect(wrapper.find('#edit-name').exists()).toBe(false)

    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('cancels the delete confirm dialog, then deletes a group once confirmed', async () => {
    const mockApiDelete = apiClient.delete as ReturnType<typeof vi.fn>
    mockApiDelete.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()
    expect(wrapper.text()).toContain('backend-team')

    await cancelThenConfirmDelete(wrapper, mockApiDelete, '/groups/1')
  })

  it('toggles and saves group members', async () => {
    const mockApiPut = apiClient.put as ReturnType<typeof vi.fn>
    mockApiPut.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()

    const membersButton = wrapper.findAll('button').find((b) => b.text() === 'Members')
    await membersButton!.trigger('click')
    await flushPromises()

    const checkboxes = wrapper.findAll('input[type="checkbox"]')
    await checkboxes[0]!.trigger('change')

    const saveButton = wrapper.findAll('button').find((b) => b.text().includes('Save Members'))
    await saveButton!.trigger('click')
    await flushPromises()

    expect(mockApiPut).toHaveBeenCalledWith(
      '/groups/1/members',
      expect.objectContaining({ user_ids: expect.any(Array) }),
    )
  })

  // The Cancel button and BaseModal's close event (Escape, backdrop) are wired
  // separately, and either has to drop the pending selection unsaved.
  it.each([
    [
      'Cancel',
      async (w: ReturnType<typeof renderWithPlugins>): Promise<void> => {
        await clickButtonWithText(w, 'Cancel')
        await flushPromises()
      },
    ],
    ['a dismissal', dismissModal],
  ])('closes the members modal on %s without saving', async (_how, close) => {
    const wrapper = renderWithPlugins(GroupsView)
    await flushPromises()

    const membersButton = wrapper.findAll('button').find((b) => b.text() === 'Members')
    await membersButton!.trigger('click')
    await flushPromises()

    await close(wrapper)

    expect(wrapper.find('.members-list').exists()).toBe(false)
    expect(openModals(wrapper)).toHaveLength(0)
    expect(apiClient.put).not.toHaveBeenCalled()
  })
})
