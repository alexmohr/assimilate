// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { DeploymentMode, SystemModeResponse } from '../types/generated'
import { getSystemMode } from '../api/system'
import { useAuthStore } from '../stores/auth'
import { useSystemModeStore } from '../stores/systemMode'
import { navigationGuard, router } from './index'

vi.mock('../api/system', () => ({
  getSystemMode: vi.fn(),
}))

const SERVER_ONLY_PATHS = ['/users', '/admin/groups', '/admin/roles', '/tunnels']

function signInAs(role: string): void {
  setActivePinia(createPinia())
  useAuthStore().user = {
    id: 1,
    username: role,
    role,
    must_change_password: false,
    session_expires_at: null,
    remember_me: false,
    can_upgrade_agent: true,
    can_view_wake_secrets: true,
    totp_enabled: false,
  }
}

async function signInAsAdminIn(mode: DeploymentMode): Promise<void> {
  signInAs('admin')
  vi.mocked(getSystemMode).mockResolvedValue({ mode })
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

  // The sidebar hides server-only entries by mode, so a page that renders
  // before the mode is known would flash them in until it loads.
  it('loads the deployment mode before any signed-in page renders', async () => {
    // Start from another page, then sign in afresh so the mode is unknown.
    await signInAsAdminIn('server')
    signInAs('admin')
    vi.mocked(getSystemMode).mockClear()
    let answer: (response: SystemModeResponse) => void = () => {}
    vi.mocked(getSystemMode).mockReturnValue(
      new Promise((resolve) => {
        answer = resolve
      }),
    )

    const navigation = router.push('/archives')
    await vi.waitFor(() => expect(getSystemMode).toHaveBeenCalledTimes(1))
    await Promise.resolve()

    expect(router.currentRoute.value.path).not.toBe('/archives')

    answer({ mode: 'desktop' })
    await navigation

    expect(router.currentRoute.value.path).toBe('/archives')
    expect(useSystemModeStore().mode).toBe('desktop')
  })

  it.each(SERVER_ONLY_PATHS)(
    'turns a non-admin away from %s without waiting on the deployment mode',
    async (path) => {
      signInAs('viewer')
      vi.mocked(getSystemMode).mockResolvedValue({ mode: 'server' })

      const result = await navigationGuard(router.resolve(path))

      expect(result).toEqual({ name: 'dashboard' })
      expect(getSystemMode).not.toHaveBeenCalled()
    },
  )

  it('fetches the deployment mode only once across navigations', async () => {
    await signInAsAdminIn('desktop')

    await router.push('/archives')
    await router.push('/profile')

    expect(getSystemMode).toHaveBeenCalledTimes(1)
  })
})
