// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import {
  adminRoutes,
  expect,
  loginAsOperator,
  loginAsViewer,
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
