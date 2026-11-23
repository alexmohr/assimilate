// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import EnrollmentTokenList from './EnrollmentTokenList.vue'
import type { EnrollmentTokenResponse } from '../types/generated'

const { mockList, mockRevoke } = vi.hoisted(() => ({
  mockList: vi.fn(),
  mockRevoke: vi.fn(),
}))

vi.mock('../api/enrollment', () => ({
  listEnrollmentTokens: mockList,
  revokeEnrollmentToken: mockRevoke,
}))

const IN_AN_HOUR = new Date(Date.now() + 3_600_000).toISOString()

function token(overrides: Partial<EnrollmentTokenResponse>): EnrollmentTokenResponse {
  return {
    id: 1,
    hint: 'enr_aaaa',
    created_by: 'admin',
    hostname: null,
    tag_ids: [],
    status: 'pending',
    created_at: '2026-11-22T10:00:00Z',
    expires_at: IN_AN_HOUR,
    used_by: null,
    ...overrides,
  }
}

async function mountList(tokens: EnrollmentTokenResponse[]) {
  mockList.mockResolvedValue({ tokens })
  const wrapper = renderWithPlugins(EnrollmentTokenList, { props: { version: 0 } })
  await flushPromises()
  return wrapper
}

describe('EnrollmentTokenList', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows nothing while there are no tokens', async () => {
    const wrapper = await mountList([])

    expect(wrapper.text()).toBe('')
  })

  it('counts the pending tokens in its header', async () => {
    const wrapper = await mountList([
      token({ id: 1 }),
      token({ id: 2, status: 'used', used_by: 'db-02' }),
      token({ id: 3 }),
    ])

    expect(wrapper.text()).toContain('2 pending')
  })

  it('says where each token stands, and offers Revoke only on a pending one', async () => {
    const wrapper = await mountList([
      token({ id: 1, hostname: 'db-02' }),
      token({ id: 2, status: 'used', used_by: 'web-01' }),
      token({ id: 3, status: 'expired' }),
      token({ id: 4, status: 'revoked' }),
    ])
    await wrapper.get('button').trigger('click')

    const rows = wrapper.findAll('[data-testid="enrollment-token-row"]')
    expect(rows).toHaveLength(4)
    expect(rows[0]?.text()).toContain('db-02')
    expect(rows[0]?.text()).toContain('Pending')
    expect(rows[1]?.text()).toContain('Used by web-01')
    expect(rows[2]?.text()).toContain('Expired')
    expect(rows[3]?.text()).toContain('Revoked')
    expect(wrapper.findAll('[data-testid="enrollment-token-row"] button')).toHaveLength(1)
  })

  it('revokes a token and reads the list again', async () => {
    mockRevoke.mockResolvedValue(undefined)
    const wrapper = await mountList([token({ id: 5 })])
    await wrapper.get('button').trigger('click')

    await wrapper.get('[data-testid="enrollment-token-row"] button').trigger('click')
    await flushPromises()

    expect(mockRevoke).toHaveBeenCalledWith(5)
    expect(mockList).toHaveBeenCalledTimes(2)
  })

  it('shows why a token could not be revoked', async () => {
    mockRevoke.mockRejectedValue(new Error('already used'))
    const wrapper = await mountList([token({ id: 6 })])
    await wrapper.get('button').trigger('click')

    await wrapper.get('[data-testid="enrollment-token-row"] button').trigger('click')
    await flushPromises()

    expect(wrapper.get('.form-error').text()).toContain('already used')
  })

  it('reads the list again when its version changes', async () => {
    const wrapper = await mountList([token({})])

    await wrapper.setProps({ version: 1 })
    await flushPromises()

    expect(mockList).toHaveBeenCalledTimes(2)
  })

  it('shows why the tokens could not be loaded', async () => {
    mockList.mockRejectedValue(new Error('nope'))
    const wrapper = renderWithPlugins(EnrollmentTokenList, { props: { version: 0 } })
    await flushPromises()
    await wrapper.get('button').trigger('click')

    expect(wrapper.find('.form-error').exists()).toBe(true)
  })
})
