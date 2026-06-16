// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, test } from './fixtures'
import type { Page } from '@playwright/test'

async function loginAsAdmin(page: Page): Promise<void> {
  await page.goto('/login')
  await page.locator('input[type="text"], input[name="username"]').fill('admin')
  await page.locator('input[type="password"]').fill('admin')
  await page.locator('button[type="submit"]').click()
  await page.waitForURL((url) => !new URL(url).pathname.startsWith('/login'), { timeout: 30_000 })
}

test('repo schedules tab loads without redirecting to the error page', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await page.getByText('server-daily').click()
  await page.waitForURL(/\/repos\/\d+/)

  await page.getByRole('button', { name: 'Schedules' }).click()
  await page.waitForURL(/tab=schedules/)
  // Give the schedules/agents/health fetches time to settle and render.
  await page.waitForTimeout(2_000)

  await expect(page).not.toHaveURL(/\/error/)
  await expect(
    page.locator('.tab-content').filter({ hasText: /Schedule|No schedules/ }),
  ).toBeVisible()
})
