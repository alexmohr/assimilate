// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import {
  adminRoutes,
  expect,
  loginAsAdmin,
  loginAsOperator,
  loginAsViewer,
  seededIdByName,
  test,
  verifyRedirectFromAdminRoutes,
} from './fixtures'

test.describe('RBAC - operator permissions', () => {
  test('operator can view hosts, repositories, and schedules', async ({ page }) => {
    await loginAsOperator(page)

    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(/\/agents/)

    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(/\/repos/)

    await page.goto('/schedules')
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(/\/schedules/)
  })

  // login-role.spec.ts covers the equivalent viewer-role redirect scenario, so
  // only the operator role (untested elsewhere) is checked here.
  test('operator is redirected away from admin routes', async ({ page }) => {
    await loginAsOperator(page)
    await verifyRedirectFromAdminRoutes(page, ['/users', ...adminRoutes])
  })
})

test.describe('RBAC - viewer permissions', () => {
  test('viewer can view the dashboard and hosts list', async ({ page }) => {
    await loginAsViewer(page)

    await page.goto('/')
    await page.waitForLoadState('networkidle')
    await expect(page).not.toHaveURL(/\/login/)

    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await expect(page).toHaveURL(/\/agents/)
  })
})

test.describe('RBAC - repository grants decide what a viewer sees', () => {
  test('a viewer sees only the repository they may read, and nothing behind the others', async ({
    page,
  }) => {
    // The demo grants viewer1 can_view on server-daily alone.
    await loginAsAdmin(page)
    const hiddenRepoId = await seededIdByName(page, '/api/repos', 'database-hourly')
    await page.context().clearCookies()

    await loginAsViewer(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await expect(page.getByText('server-daily', { exact: true }).first()).toBeVisible()
    await expect(page.getByText('database-hourly', { exact: true })).toHaveCount(0)

    const repos = (await (await page.request.get('/api/repos')).json()) as { name: string }[]
    expect(repos.map((repo) => repo.name)).toEqual(['server-daily'])

    // A repository the viewer may not read answers like one that does not exist.
    expect((await page.request.get(`/api/repos/${hiddenRepoId}`)).status()).toBe(404)
    const activity = (await (await page.request.get('/api/stats/activity?limit=200')).json()) as {
      target_name: string
    }[]
    expect(activity.every((row) => row.target_name === 'server-daily')).toBe(true)
  })
})

test.describe('RBAC - a viewer sees only the dependencies of what they see', () => {
  test('lists files-01, needed by server-daily, but not nas-media, which only another schedule needs', async ({
    page,
  }) => {
    // files-01 is required by the repository host server-daily lives on;
    // nas-media only by "Media share nightly", into media-weekly.
    await loginAsAdmin(page)
    const hiddenId = await seededIdByName(page, '/api/dependency-hosts', 'nas-media')
    await page.context().clearCookies()

    await loginAsViewer(page)
    await page.goto('/dependencies')
    const rows = page.locator('[data-testid="dependency-row"]')
    await rows.filter({ hasText: 'files-01' }).first().waitFor()
    await expect(rows.filter({ hasText: 'nas-media' })).toHaveCount(0)

    const hosts = (await (await page.request.get('/api/dependency-hosts')).json()) as {
      name: string
    }[]
    expect(hosts.map((host) => host.name)).toEqual(['files-01'])
    // One the viewer may not see answers like one that does not exist.
    expect((await page.request.get(`/api/dependency-hosts/${hiddenId}`)).status()).toBe(404)
    expect((await page.request.get(`/api/dependency-hosts/${hiddenId}/usage`)).status()).toBe(404)
  })
})
