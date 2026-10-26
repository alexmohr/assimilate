// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, seededIdByName, test } from './fixtures'

/**
 * Dependencies have their own page, reached from the sidebar. The demo seeds
 * nas-media and files-01 (see `.devcontainer/demo/seed-demo.sh`).
 */
test.describe('Dependencies page', () => {
  test('lists the dependencies from its sidebar entry and opens one', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.getByRole('link', { name: 'Dependencies', exact: true }).click()
    await expect(page).toHaveURL(/\/dependencies$/)
    await expect(page.getByRole('heading', { name: 'Dependencies' })).toBeVisible()

    const id = await seededIdByName(page, '/api/dependency-hosts', 'nas-media')
    await page.locator('a.entity-card', { hasText: 'files-01' }).first().waitFor()
    await page.locator('a.entity-card', { hasText: 'nas-media' }).click()
    await page.waitForURL(`**/dependency-hosts/${id}`)

    await page.locator('.detail-breadcrumb').getByRole('link', { name: 'Dependencies' }).click()
    await expect(page).toHaveURL(/\/dependencies$/)
  })

  test('the Agents page lists agents only', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await expect(page.getByRole('tab')).toHaveCount(0)
    await expect(page.locator('.entity-card', { hasText: 'nas-media' })).toHaveCount(0)
  })

  test("an old link to the Agents page's dependencies tab lands on the page", async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents?tab=dependencies')
    await expect(page).toHaveURL(/\/dependencies$/)
    await expect(page.getByRole('heading', { name: 'Dependencies' })).toBeVisible()
  })
})
