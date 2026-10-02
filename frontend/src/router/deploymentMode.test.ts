// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { DeploymentMode } from '../types/generated'
import { getSystemMode } from '../api/system'
import { useAuthStore } from '../stores/auth'
import { router } from './index'

vi.mock('../api/system', () => ({
  getSystemMode: vi.fn(),
}))

const SERVER_ONLY_PATHS = ['/users', '/admin/groups', '/admin/roles', '/tunnels']

async function signInAsAdminIn(mode: DeploymentMode): Promise<void> {
  setActivePinia(createPinia())
  vi.mocked(getSystemMode).mockResolvedValue({ mode })
  useAuthStore().user = {
    id: 1,
    username: 'admin',
    role: 'admin',
    must_change_password: false,
    session_expires_at: null,
    remember_me: false,
    can_upgrade_agent: true,
    can_view_wake_secrets: true,
    totp_enabled: false,
  }
  await router.push('/excludes')
}

describe('router - deployment mode', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it.each(SERVER_ONLY_PATHS)('marks %s as server-only', (path) => {
    expect(router.resolve(path).meta.serverOnly).toBe(true)
  })

  it.each(SERVER_ONLY_PATHS)('redirects %s to the dashboard in desktop mode', async (path) => {
    await signInAsAdminIn('desktop')

    await router.push(path)

    expect(router.currentRoute.value.name).toBe('dashboard')
  })

  it.each(SERVER_ONLY_PATHS)('opens %s in server mode', async (path) => {
    await signInAsAdminIn('server')

    await router.push(path)

    expect(router.currentRoute.value.path).toBe(path)
  })
})
