// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsOperator, loginAsViewer, test } from './fixtures'

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

  for (const path of ['/users', '/admin/roles', '/admin/groups']) {
    test(`operator is redirected away from ${path}`, async ({ page }) => {
      await loginAsOperator(page)
      await page.goto(path)
      await page.waitForLoadState('networkidle')

      // requiresAdmin route guard redirects non-admins to the dashboard.
      await expect(page).toHaveURL('/')
    })
  }
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

  for (const path of ['/users', '/admin/roles', '/admin/groups']) {
    test(`viewer is redirected away from ${path}`, async ({ page }) => {
      await loginAsViewer(page)
      await page.goto(path)
      await page.waitForLoadState('networkidle')

      await expect(page).toHaveURL('/')
    })
  }
})
