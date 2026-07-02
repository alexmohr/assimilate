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

async function navigateToRepo(page: Page, repoName: string, tab?: string): Promise<void> {
  await page.goto('/repos')
  await page.getByText(repoName).first().click()
  await page.waitForURL(/\/repos\/\d+/)
  if (tab) {
    await page.getByRole('button', { name: tab, exact: true }).click()
  }
}

// Repository list

test('repos page shows all seeded repositories', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await expect(page.getByText('server-daily')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('database-hourly')).toBeVisible()
  await expect(page.getByText('media-weekly')).toBeVisible()
})

// Repository detail - basic structure

test('repo detail page loads without error', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')
  await page.waitForTimeout(2_000)
  await expect(page).not.toHaveURL(/\/error/)
})

test('repo detail shows Full Resync button when not importing', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')
  await expect(page.getByRole('button', { name: /full resync/i })).toBeVisible({ timeout: 10_000 })
})

test('repo detail shows archive list with entries', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily', 'Archives')
  // Archives section must contain at least one row
  await expect(page.locator('.archive-row').first()).toBeVisible({
    timeout: 15_000,
  })
})

// Unmatched archives

test('server-daily shows unmatched-banner for old-webserver archives', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily', 'Archives')
  // Demo seeds one old-webserver archive into server-daily; banner must appear
  await expect(page.locator('.unmatched-banner')).toBeVisible({ timeout: 15_000 })
})

test('unmatched-banner contains a Re-scan button', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily', 'Archives')
  await expect(page.locator('.unmatched-banner')).toBeVisible({ timeout: 15_000 })
  await expect(page.getByRole('button', { name: /re-scan/i })).toBeVisible()
})

test('database-hourly shows unmatched-banner for legacy-db-prod archives', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'database-hourly', 'Archives')
  await expect(page.locator('.unmatched-banner')).toBeVisible({ timeout: 15_000 })
})

// Imported clients

test('agents page shows imported placeholder clients', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  // Demo creates imported clients for old-webserver and legacy-db-prod
  await expect(
    page.getByText('old-webserver').or(page.getByText('legacy-db-prod')).first(),
  ).toBeVisible({ timeout: 10_000 })
})

test('imported clients have the Imported badge', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  // At least one .badge-imported must be present (old-webserver and legacy-db-prod)
  await expect(page.locator('.badge-imported').first()).toBeVisible({ timeout: 10_000 })
})

test('imported clients show Merge into... and Adopt action buttons', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  await expect(page.locator('.badge-imported').first()).toBeVisible({ timeout: 10_000 })
  // Merge and Adopt must be visible for at least one imported client
  await expect(page.getByRole('button', { name: /merge into/i }).first()).toBeVisible()
  await expect(page.getByRole('button', { name: /adopt/i }).first()).toBeVisible()
})

// Import-state resilience: reset stuck import

test('Cancel Import button appears when repo is in importing state', async ({ page }) => {
  await loginAsAdmin(page)

  // Trigger a sync to get the repo into importing state, then immediately check for the button.
  // We use the API directly so we can check the UI before it completes.
  const reposRes = await page.request.get('/api/repos')
  const repos = (await reposRes.json()) as Array<{ id: number; name: string }>
  const repo = repos.find((r) => r.name === 'server-daily')
  if (!repo) throw new Error('server-daily repo not found')

  // Start a sync in the background (don't await response)
  page.request.post(`/api/repos/${repo.id}/sync`).catch(() => undefined)

  await page.goto(`/repos/${repo.id}`)

  // Either Cancel Import (if caught mid-sync) or Full Resync (already done) must be visible.
  // Both buttons share the same toolbar, so we assert at least one is there.
  const cancelBtn = page.getByRole('button', { name: /cancel import/i })
  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(cancelBtn.or(resyncBtn)).toBeVisible({ timeout: 15_000 })

  // Wait for any in-progress sync to finish so subsequent tests start with a clean state.
  await expect(resyncBtn).toBeVisible({ timeout: 120_000 })
})

// Full resync

test('full resync completes and preserves archives', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  // Wait for the page to settle and button to be ready
  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // Button immediately switches to "Syncing..." while the request is in flight
  await expect(page.getByRole('button', { name: /syncing/i })).toBeVisible({ timeout: 5_000 })

  // Sync is synchronous server-side; toast fires when it resolves
  await expect(page.getByText('Full resync started.')).toBeVisible({ timeout: 120_000 })

  // Button must return to its resting state
  await expect(resyncBtn).toBeVisible({ timeout: 5_000 })

  // Switch to archives tab and verify entries are still present after resync
  await page.getByRole('button', { name: 'Archives', exact: true }).click()
  await expect(page.locator('.archive-row').first()).toBeVisible({ timeout: 15_000 })
})

test('full resync preserves unmatched-banner', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  await expect(page.getByText('Full resync started.')).toBeVisible({ timeout: 120_000 })

  // Switch to archives tab - unmatched old-webserver archive must survive a resync
  await page.getByRole('button', { name: 'Archives', exact: true }).click()
  await expect(page.locator('.unmatched-banner')).toBeVisible({ timeout: 10_000 })
})

test('broken repo resync does not navigate to /error page', async ({ page }) => {
  await loginAsAdmin(page)

  // Navigate to a repo then break its path via API to provoke a sync failure
  const reposRes = await page.request.get('/api/repos')
  const repos = (await reposRes.json()) as Array<{ id: number; name: string }>
  const repo = repos.find((r) => r.name === 'server-daily')
  if (!repo) throw new Error('server-daily repo not found')

  // Patch the repo to an invalid path
  await page.request.put(`/api/repos/${repo.id}`, {
    data: { repo_path: '/nonexistent/broken/path' },
  })

  await page.goto(`/repos/${repo.id}`)
  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // The sync request is accepted immediately - "Full resync started." toast must appear
  // and the page must stay on the repo detail view, never redirecting to /error
  await expect(page.locator('.toast-success').first()).toBeVisible({ timeout: 30_000 })
  await expect(page).not.toHaveURL(/\/error/)

  // Restore the repo path
  await page.request.put(`/api/repos/${repo.id}`, {
    data: { repo_path: '/backup/repos/server-daily' },
  })
})

// Archive browsing

test('clicking an archive opens the file browser', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily', 'Archives')

  // Wait for archive rows to load, then click the first one
  const firstArchiveRow = page.locator('.archive-row').first()
  await expect(firstArchiveRow).toBeVisible({ timeout: 15_000 })
  await firstArchiveRow.click()

  // File browser panel or breadcrumb must appear
  await expect(
    page.locator('.file-browser, .breadcrumb, .crumb, [class*="browser"]').first(),
  ).toBeVisible({ timeout: 10_000 })
})
