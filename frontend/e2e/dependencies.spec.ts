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
    await expect(page.getByRole('heading', { level: 1, name: 'Dependencies' })).toBeVisible()

    const id = await seededIdByName(page, '/api/dependency-hosts', 'nas-media')
    const rows = page.locator('[data-testid="dependency-row"]')
    await rows.filter({ hasText: 'files-01' }).first().waitFor()
    await rows.filter({ hasText: 'nas-media' }).getByRole('link', { name: 'nas-media' }).click()
    await page.waitForURL(`**/dependency-hosts/${id}`)

    await page.locator('.detail-breadcrumb').getByRole('link', { name: 'Dependencies' }).click()
    await expect(page).toHaveURL(/\/dependencies$/)
  })

  test('the Agents page lists agents only', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.waitForLoadState('networkidle')
    await expect(page.getByRole('tab')).toHaveCount(0)
    await expect(page.getByText('nas-media', { exact: true })).toHaveCount(0)
  })

  test('shows the runs waiting on a dependency and links to its Used by', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/dependencies')
    const id = await seededIdByName(page, '/api/dependency-hosts', 'nas-media')

    // The demo's "Media share nightly" was skipped because nas-media did not
    // answer, and waits on it to catch up.
    const waiting = page.locator('.dependency-waiting')
    await expect(waiting.first()).toContainText('Media share nightly')
    await expect(waiting.first()).toContainText('waiting since')

    const row = page.locator('[data-testid="dependency-row"]', { hasText: 'nas-media' })
    await row.locator('.dependency-used-link').first().click()
    await page.waitForURL(`**/dependency-hosts/${id}?section=used-by`)
    await expect(page.locator('a.agent-row', { hasText: 'Media share nightly' })).toHaveCount(1)
  })

  test('filters the list down to the dependencies runs are waiting on', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/dependencies')
    const rows = page.locator('[data-testid="dependency-row"]')
    await rows.first().waitFor()
    await page.getByRole('button', { name: /^Runs waiting · \d+$/ }).click()
    await expect(rows.filter({ hasText: 'nas-media' })).toHaveCount(1)
    await expect(rows.filter({ hasText: 'files-01' })).toHaveCount(0)
  })

  test("an old link to the Agents page's dependencies tab lands on the page", async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents?tab=dependencies')
    await expect(page).toHaveURL(/\/dependencies$/)
    await expect(page.getByRole('heading', { level: 1, name: 'Dependencies' })).toBeVisible()
  })
})
