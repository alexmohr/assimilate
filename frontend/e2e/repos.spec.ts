// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { Route } from '@playwright/test'
import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Repositories management journey', () => {
  test('repo list page shows known demo repositories', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const repoRows = page.locator('a.list-row')
    await expect(repoRows.first()).toBeVisible()

    const text = await page.locator('body').innerText()
    const hasRepo =
      text.includes('server-daily') ||
      text.includes('database-hourly') ||
      text.includes('media-weekly') ||
      text.includes('lz4') ||
      text.includes('zstd') ||
      text.includes('repokey')
    expect(hasRepo).toBe(true)
  })

  test('repo detail page shows compression and encryption info', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1')
    await page.waitForLoadState('networkidle')

    const text = await page.locator('body').innerText()

    const hasCompression = text.includes('lz4') || text.includes('zstd') || text.includes('none')
    expect(hasCompression).toBe(true)

    const hasEncryption =
      text.includes('repokey') ||
      text.includes('blake2') ||
      text.includes('authenticated') ||
      text.includes('none') ||
      text.includes('encryption') ||
      text.includes('Encryption')
    expect(hasEncryption).toBe(true)
  })

  // The five cards that used to stack on Overview now sit behind a settings
  // sub-nav, so simply loading /repos/1 no longer mounts them - and no longer
  // calls the endpoints they read. Walking the rail keeps that server-side
  // path exercised (quota, tags, the repository record and its SSH host key)
  // and covers the navigation itself, which only had the danger pane before.
  test('the repository Settings rail opens each of its panes', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=settings&section=repository')
    await page.waitForLoadState('networkidle')

    const rail = page.locator('.settings-nav')
    const pane = page.locator('.settings-pane')

    await expect(rail.locator('.settings-nav-item[aria-current="true"]')).toHaveText('Repository')
    await expect(pane).toContainText('SSH user')

    // The rail names each pane, so a pane proves it mounted by its own
    // content rather than by a heading repeating the rail item.
    await rail.getByRole('button', { name: 'Storage quota' }).click()
    await pane.getByRole('button', { name: 'Help: space limits' }).click()
    await expect(pane).toContainText('How much space this repository may use')
    await expect(page).toHaveURL(/section=quota/)

    await rail.getByRole('button', { name: 'Tags' }).click()
    await expect(pane.getByPlaceholder('New tag name')).toBeVisible()
    await expect(page).toHaveURL(/section=tags/)

    await rail.getByRole('button', { name: 'Borg console' }).click()
    await expect(pane.getByLabel('borg command')).toBeVisible()
    await expect(page).toHaveURL(/section=console/)
  })

  // offsite-imported is seeded by a config import, so it arrives without a
  // passphrase and held as importing. Runs against the demo's real borg: a
  // wrong passphrase must be refused by borg itself, and the right one (every
  // demo repository shares it) stored and the repository released.
  test('an admin sets the passphrase of a config-imported repository', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.getByText('offsite-imported').first().click()
    await page.waitForURL(/\/repos\/\d+/)
    await page.locator('.overflow-toggle').click()
    await page.getByRole('menuitem', { name: 'Set passphrase' }).click()

    const dialog = page.locator('.modal-dialog')
    const field = dialog.getByLabel('Passphrase')
    await field.fill('not-the-passphrase')
    await dialog.getByRole('button', { name: 'Save passphrase' }).click()
    await expect(dialog.locator('.form-error')).toContainText('incorrect', { timeout: 30_000 })

    await field.fill('demo-passphrase-123')
    await dialog.getByRole('button', { name: 'Save passphrase' }).click()
    await expect(page.getByText('Passphrase saved.')).toBeVisible({ timeout: 30_000 })
    await expect(dialog).toBeHidden()
    await expect(page.locator('.repo-status-badge')).toHaveText('Enabled')
  })

  test('clicking a repo from the list navigates to detail page', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const firstRow = page.locator('a.list-row').first()
    await expect(firstRow).toBeVisible()
    await firstRow.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repos\/\d+/)
  })

  test('repo detail shows associated schedules or archives info', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1')
    await page.waitForLoadState('networkidle')

    const text = await page.locator('body').innerText()
    const hasRelatedInfo =
      text.includes('schedule') ||
      text.includes('Schedule') ||
      text.includes('archive') ||
      text.includes('Archive') ||
      text.includes('backup') ||
      text.includes('Backup')
    expect(hasRelatedInfo).toBe(true)
  })

  test('a repo row with unmatched archives leads to the archives tab', async ({ page }) => {
    await page.route('**/api/repos/stats', async (route: Route) => {
      const response = await route.fetch()
      const repos = (await response.json()) as Array<Record<string, unknown>>
      // A quota breach outranks unmatched archives in the row's one status, so
      // the repository loses its quota here to let the unmatched count show.
      const target = repos.find((r) => r.name === 'server-daily')
      if (target) {
        target.unmatched_count = 3
        target.quota = null
      }
      return route.fulfill({
        status: response.status(),
        contentType: 'application/json',
        body: JSON.stringify(repos),
      })
    })

    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const row = page.locator('a.list-row', { hasText: 'server-daily' }).first()
    await expect(row.locator('.badge')).toHaveText('3 unmatched')

    await row.click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repos\/\d+\?tab=archives/)
  })

  test('breaking a repository lock shows a success result in the confirmation dialog', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos/1?tab=settings&section=danger')
    await page.waitForLoadState('networkidle')

    const dangerZone = page.locator('.settings-pane')
    const breakLockBtn = dangerZone.getByRole('button', { name: 'Break lock', exact: true })
    await expect(breakLockBtn).toBeVisible()
    await breakLockBtn.click()

    await expect(page.locator('.modal-title')).toHaveText('Break repository lock')
    await expect(page.locator('.break-lock-warning').first()).toContainText(
      'stale local cache lock',
    )

    await page.getByRole('button', { name: 'Yes, break lock', exact: true }).click()

    // Demo repo has no active lock, so borg break-lock is a safe no-op that
    // still reports success - the exact wording isn't asserted since it's
    // borg's own message, just that the dialog reflects a result and not an
    // error.
    await expect(page.locator('.break-lock-success')).toBeVisible({ timeout: 15_000 })
    await expect(page.locator('.form-error')).not.toBeVisible()

    // Scoped to the footer: BaseModal's own dismiss control also exposes the
    // accessible name "Close" (F-22 gave it one), so an unscoped lookup is
    // ambiguous. This targets the same footer button the test always meant.
    await page.locator('.modal-footer').getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.locator('.modal-title')).not.toBeVisible()
  })

  test('the Repositories page groups by host by default with a shared storage pool', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    // The quota filter pills are always visible: All, At risk, No quota.
    const pills = page.locator('.filter-pills .filter-pill')
    await expect(pills).toHaveCount(3)
    await expect(pills.filter({ hasText: 'All' })).toBeVisible()

    // The demo's database-hourly/media-weekly repos share the "localhost" ssh_host
    // and a configured server quota (see .devcontainer/demo/seed-demo.sh), and
    // group by host is the default view - no need to toggle it on.
    const poolHeader = page.locator('.pool-header', { hasText: 'localhost' })
    await expect(poolHeader).toBeVisible()
    await expect(poolHeader.locator('.pool-track')).toBeVisible()

    // Each demo repo also has its own quota configured, so its row shows a usage bar
    // on its own scale rather than the shared pool scale.
    await expect(page.locator('a.list-row .progress-track').first()).toBeVisible()

    // media-weekly's demo quota (warn_bytes: 1) is always in a breached state, so it
    // should still be visible as an at-risk repo once the filter narrows the view.
    await pills.filter({ hasText: 'At risk' }).click()
    await page.waitForLoadState('networkidle')
    const atRisk = page.locator('a.list-row', { hasText: 'media-weekly' })
    await expect(atRisk).toBeVisible()
    await expect(atRisk.locator('.badge')).toHaveText(/(Near|Over) quota/)
  })
})
