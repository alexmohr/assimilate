// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ApiTokenTable from './ApiTokenTable.vue'

vi.mock('../utils/format', () => ({
  formatDate: vi.fn((iso: string | null, fallback = '\u2014') => {
    if (!iso) return fallback
    return new Date(iso).toLocaleString('en-US', {
      timeZone: 'UTC',
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    })
  }),
}))

import { formatDate } from '../utils/format'

interface TestToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

const mockTokens: TestToken[] = [
  {
    id: 1,
    user_id: 1,
    name: 'CI pipeline',
    created_at: '2026-01-01T00:00:00Z',
    last_used_at: null,
  },
  {
    id: 2,
    user_id: 1,
    name: 'deploy-bot',
    created_at: '2026-01-02T00:00:00Z',
    last_used_at: '2026-05-01T00:00:00Z',
  },
]

const mockFormatDate = formatDate as ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  mockFormatDate.mockImplementation((iso, fallback) => {
    if (!iso) return fallback ?? '\u2014'
    return new Date(iso).toLocaleString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    })
  })
})

describe('ApiTokenTable', () => {
  it('renders table headers', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const headers = wrapper.findAll('th')
    expect(headers.map((h) => h.text())).toEqual(['Name', 'Created', 'Last used', 'Actions'])
  })

  it('renders one row per token', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const rows = wrapper.findAll('tbody tr')
    expect(rows.length).toBe(2)
  })

  it('renders token name in each row', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const nameCells = wrapper.findAll('.cell-name')
    expect(nameCells[0].text()).toBe('CI pipeline')
    expect(nameCells[1].text()).toBe('deploy-bot')
  })

  it('renders formatted created date', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const dateCells = wrapper.findAll('.cell-date')
    const createdText = dateCells[0].text()
    expect(createdText).toContain('2026')
    expect(createdText).toContain('Jan')
  })

  it('renders "Never" when last_used_at is null', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const dateCells = wrapper.findAll('.cell-date')
    expect(dateCells[1].text()).toContain('Never')
  })

  it('renders formatted last_used_at when present', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const dateCells = wrapper.findAll('.cell-date')
    const lastUsedText = dateCells[3].text()
    expect(lastUsedText).toContain('2026')
    expect(lastUsedText).toContain('May')
  })

  it('renders a delete button per row with Trash2 icon', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const deleteButtons = wrapper.findAll('button.btn-danger-text')
    expect(deleteButtons.length).toBe(2)
  })

  it('emits delete event with correct token when first row button is clicked', async () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const deleteButtons = wrapper.findAll('button.btn-danger-text')

    await deleteButtons[0].trigger('click')

    expect(wrapper.emitted('delete')).toBeTruthy()
    const emittedArgs = wrapper.emitted('delete')?.[0] as TestToken[] | undefined
    expect(emittedArgs?.[0]?.id).toBe(1)
    expect(emittedArgs?.[0]?.name).toBe('CI pipeline')
  })

  it('emits delete event with correct token when second row button is clicked', async () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: mockTokens } })
    const deleteButtons = wrapper.findAll('button.btn-danger-text')

    await deleteButtons[1].trigger('click')

    expect(wrapper.emitted('delete')).toBeTruthy()
    const emittedArgs = wrapper.emitted('delete')?.[0] as TestToken[] | undefined
    expect(emittedArgs?.[0]?.id).toBe(2)
    expect(emittedArgs?.[0]?.name).toBe('deploy-bot')
  })

  it('renders empty table body when tokens prop is empty', () => {
    const wrapper = mount(ApiTokenTable, { props: { tokens: [] } })
    const rows = wrapper.findAll('tbody tr')
    expect(rows.length).toBe(0)
  })

  it('calls formatDate for each token created_at', () => {
    mount(ApiTokenTable, { props: { tokens: mockTokens } })
    expect(mockFormatDate).toHaveBeenCalledWith('2026-01-01T00:00:00Z')
    expect(mockFormatDate).toHaveBeenCalledWith('2026-01-02T00:00:00Z')
  })

  it('calls formatDate with Never fallback for null last_used_at', () => {
    mount(ApiTokenTable, { props: { tokens: mockTokens } })
    expect(mockFormatDate).toHaveBeenCalledWith(null, 'Never')
  })
})
