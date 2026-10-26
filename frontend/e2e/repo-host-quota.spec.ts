// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Page } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A repository host's Storage quota: the limit every repository on the host
 * shares. The demo sets one on the "localhost" host (20 GiB warning, 30 GiB
 * critical, block backups) - see `.devcontainer/demo/seed-demo.sh`.
 */
async function localhostId(page: Page): Promise<number> {
  const hosts = (await (await page.request.get('/api/repo-hosts')).json()) as {
    id: number
    ssh_host: string
  }[]
  const host = hosts.find((h) => h.ssh_host === 'localhost')
  expect(host).toBeDefined()
  return host!.id
}

test.describe('Repository host storage quota', () => {
  test("shows the host's quota in its own section and saves an edit", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await localhostId(page)
    await page.goto(`/repo-hosts/${id}`)
    await page.waitForLoadState('networkidle')

    await page.locator('.settings-nav-item', { hasText: 'Storage quota' }).click()
    await expect(page).toHaveURL(/section=quota/)
    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('Block backups')
    await expect(pane).toContainText('limit')

    // Answer the save here so the demo's quota stays as seeded for other tests.
    let saved: Record<string, unknown> | null = null
    await page.route('**/api/server-quotas/localhost', async (route) => {
      if (route.request().method() !== 'PUT') return route.continue()
      saved = (await route.request().postDataJSON()) as Record<string, unknown>
      return route.fulfill({ status: 200, contentType: 'application/json', body: '{}' })
    })

    await pane.getByRole('button', { name: 'Edit' }).click()
    await pane.getByLabel('Warning action').selectOption('disable_schedule')
    await pane.getByRole('button', { name: 'Save' }).click()

    await expect(async () => {
      expect(saved).not.toBeNull()
      expect((saved as Record<string, unknown>).warn_action).toBe('disable_schedule')
      expect((saved as Record<string, unknown>).critical_action).toBe('block_backups')
    }).toPass({ timeout: 5_000 })
  })

  test("a repository's overview leads to its host's quota", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await localhostId(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.locator('.entity-card').filter({ hasText: 'media-weekly' }).first().click()
    await page.waitForLoadState('networkidle')

    const tile = page.locator('.tile', { hasText: 'Host quota' })
    await expect(tile).toContainText('on localhost')
    await tile.click()
    await expect(page).toHaveURL(new RegExp(`/repo-hosts/${id}\\?section=quota`))
    await expect(page.locator('.settings-pane')).toContainText('Block backups')
  })

  test('the old Server Quotas page is gone from settings and sends its links to the repositories', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.getByRole('button', { name: 'Settings' }).click()
    await expect(page.getByRole('link', { name: 'Database', exact: true })).toBeVisible()
    await expect(page.getByRole('link', { name: 'Server Quotas', exact: true })).toHaveCount(0)

    await page.goto('/server-quotas')
    await expect(page).toHaveURL(/\/repos$/)
  })
})
