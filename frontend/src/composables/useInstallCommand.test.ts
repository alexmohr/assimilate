// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DEFAULT_LIFETIME_SECS, useInstallCommand } from './useInstallCommand'
import type { CreateEnrollmentTokenResponse } from '../types/generated'

const { mockCreate, mockRevoke } = vi.hoisted(() => ({
  mockCreate: vi.fn(),
  mockRevoke: vi.fn(),
}))

vi.mock('../api/enrollment', () => ({
  createEnrollmentToken: mockCreate,
  revokeEnrollmentToken: mockRevoke,
}))

const CREATED: CreateEnrollmentTokenResponse = {
  plaintext: 'enr_secret',
  token: {
    id: 9,
    hint: 'enr_secr',
    created_by: 'admin',
    hostname: null,
    tag_ids: [],
    status: 'pending',
    created_at: '2026-11-22T10:00:00Z',
    expires_at: '2026-11-22T11:00:00Z',
    used_by: null,
  },
}

describe('useInstallCommand', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('creates a token for an hour unless told otherwise', async () => {
    mockCreate.mockResolvedValue(CREATED)
    const install = useInstallCommand()

    await install.generate()

    expect(mockCreate).toHaveBeenCalledWith({
      lifetime_secs: DEFAULT_LIFETIME_SECS,
      hostname: undefined,
      tag_ids: [],
    })
    expect(install.created.value).toEqual(CREATED)
    expect(install.loading.value).toBe(false)
  })

  it('sends the lifetime, a trimmed hostname and the chosen tags', async () => {
    mockCreate.mockResolvedValue(CREATED)
    const install = useInstallCommand()
    install.lifetime.value = 900
    install.hostname.value = '  db-02  '
    install.tagIds.value = [3, 5]

    await install.generate()

    expect(mockCreate).toHaveBeenCalledWith({
      lifetime_secs: 900,
      hostname: 'db-02',
      tag_ids: [3, 5],
    })
  })

  it('reports a failure and creates nothing', async () => {
    mockCreate.mockRejectedValue(new Error('boom'))
    const install = useInstallCommand()

    await install.generate()

    expect(install.created.value).toBeNull()
    expect(install.error.value).toBeTruthy()
    expect(install.loading.value).toBe(false)
  })

  it('revokes the token it created and asks for the list to be read again', async () => {
    mockCreate.mockResolvedValue(CREATED)
    mockRevoke.mockResolvedValue(undefined)
    const install = useInstallCommand()
    await install.generate()
    const before = install.version.value

    await install.revoke()

    expect(mockRevoke).toHaveBeenCalledWith(9)
    expect(install.version.value).toBeGreaterThan(before)
  })

  it('revokes nothing when no token was created', async () => {
    const install = useInstallCommand()

    await install.revoke()

    expect(mockRevoke).not.toHaveBeenCalled()
  })

  it('keeps the error when the token cannot be revoked any more', async () => {
    mockCreate.mockResolvedValue(CREATED)
    mockRevoke.mockRejectedValue(new Error('already used'))
    const install = useInstallCommand()
    await install.generate()

    await install.revoke()

    expect(install.error.value).toBeTruthy()
  })

  it('forgets everything on reset', async () => {
    mockCreate.mockResolvedValue(CREATED)
    const install = useInstallCommand()
    install.hostname.value = 'db-02'
    install.tagIds.value = [1]
    await install.generate()

    install.reset()

    expect(install.created.value).toBeNull()
    expect(install.hostname.value).toBe('')
    expect(install.tagIds.value).toEqual([])
    expect(install.lifetime.value).toBe(DEFAULT_LIFETIME_SECS)
  })
})
