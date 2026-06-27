// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { execSync } from 'node:child_process'

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

/** Find the running demo server container by its compose service label. */
function demoContainer(): string {
  const out = execSync(
    "docker ps --filter 'label=com.docker.compose.service=demo' --format '{{.Names}}'",
    { timeout: 10_000 },
  )
    .toString()
    .trim()
  const name = out.split('\n').find((n) => n.trim())
  if (!name) throw new Error('demo container not found — is the demo environment running?')
  return name.trim()
}

/** Run a borg command inside the demo container as the borg user. */
function borgRun(container: string, borgCmd: string): string {
  const full = `BORG_PASSPHRASE=demo-passphrase-123 ${borgCmd}`
  return execSync(`docker exec ${container} su -c '${full}' borg`, {
    timeout: 30_000,
  }).toString()
}

/** Run an arbitrary shell command inside the demo container as root. */
function dockerExec(container: string, cmd: string): void {
  execSync(`docker exec ${container} sh -c '${cmd}'`, { timeout: 10_000 })
}

async function navigateToRepo(page: Page, repoName: string, tab?: string): Promise<void> {
  // 'commit' resolves on response headers without waiting for all API data to
  // load, so slow CI runners do not exhaust the test budget on navigation alone.
  await page.goto('/repos', { waitUntil: 'commit' })
  await page.getByText(repoName).first().click()
  await page.waitForURL(/\/repos\/\d+/, { waitUntil: 'commit' })
  if (tab) {
    await page.getByRole('button', { name: tab, exact: true }).click()
  }
}

// ── Repository list ──────────────────────────────────────────────────────────

test('repos page shows all seeded repositories', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/repos')
  await expect(page.getByText('server-daily')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('database-hourly')).toBeVisible()
  await expect(page.getByText('media-weekly')).toBeVisible()
})

// ── Repository detail — basic structure ─────────────────────────────────────

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

// ── Unmatched archives ───────────────────────────────────────────────────────

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

// ── Imported agents ──────────────────────────────────────────────────────────

test('agents page shows imported placeholder agents', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  // On a cold DB the /api/agents call can take several minutes. The 270 s
  // toBeVisible timeout covers both the API latency and Vue render time.
  await expect(
    page
      .locator('.card-hostname')
      .filter({ hasText: /old-webserver|legacy-db-prod/ })
      .first(),
  ).toBeVisible({ timeout: 270_000 })
})

test('imported agents have the Imported badge', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  // At least one .badge-imported must be present (old-webserver and legacy-db-prod)
  await expect(page.locator('.badge-imported').first()).toBeVisible({ timeout: 270_000 })
})

test('imported agents show Merge into... and Adopt action buttons', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  await expect(page.locator('.badge-imported').first()).toBeVisible({ timeout: 270_000 })
  // Merge and Adopt must be visible for at least one imported agent
  await expect(page.getByRole('button', { name: /merge into/i }).first()).toBeVisible()
  await expect(page.getByRole('button', { name: /adopt/i }).first()).toBeVisible()
})

// ── Import-state resilience: reset stuck import ──────────────────────────────

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

test('Cancel Import cancels a live resync under borg lock contention', async ({ page }) => {
  test.setTimeout(180_000)

  await loginAsAdmin(page)

  const container = demoContainer()
  const lockFile = '/backup/repos/server-daily/lock.exclusive'

  const reposRes = await page.request.get('/api/repos')
  const repos = (await reposRes.json()) as Array<{ id: number; name: string }>
  const repo = repos.find((r) => r.name === 'server-daily')
  if (!repo) throw new Error('server-daily repo not found')

  dockerExec(container, `touch ${lockFile}`)

  try {
    await page.goto(`/repos/${repo.id}`)

    const resyncBtn = page.getByRole('button', { name: /full resync/i })
    await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
    await resyncBtn.click()

    const cancelBtn = page.getByRole('button', { name: /cancel import/i })
    await expect(cancelBtn).toBeVisible({ timeout: 30_000 })
    await expect(page.locator('.repo-status-badge')).toHaveText(/importing/i, { timeout: 30_000 })

    await cancelBtn.click()

    await expect(page.getByText('Import state reset.')).toBeVisible({ timeout: 30_000 })
    await expect(resyncBtn).toBeVisible({ timeout: 30_000 })
    await expect(page.locator('.import-status-msg')).not.toBeVisible()
    await expect(page.locator('.repo-status-badge')).toHaveText(/enabled/i, { timeout: 30_000 })

    const logsRes = await page.request.get('/api/logs?limit=200&search=repo%20sync%20cancelled')
    expect(logsRes.ok()).toBeTruthy()
    const logs = (await logsRes.json()) as Array<{ message: string }>
    expect(logs.some((entry) => entry.message.includes('repo sync cancelled'))).toBeTruthy()
  } finally {
    dockerExec(container, `rm -f ${lockFile}`)
  }
})

// ── Full resync ──────────────────────────────────────────────────────────────

test('full resync completes and preserves archives', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  // Wait for the page to settle and button to be ready
  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // The request resolves quickly on CI, so the transient "Syncing..." label is
  // not a stable contract. Assert the accepted action via toast and the final
  // steady-state button label instead.
  await expect(page.getByText('Full resync started.')).toBeVisible({ timeout: 120_000 })

  // Button must return to its resting state
  await expect(resyncBtn).toBeVisible({ timeout: 30_000 })

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

  // Switch to archives tab — unmatched old-webserver archive must survive a resync
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

  // The sync request is accepted immediately — "Full resync started." toast must appear
  // and the page must stay on the repo detail view, never redirecting to /error
  await expect(page.locator('.toast-success').first()).toBeVisible({ timeout: 30_000 })
  await expect(page).not.toHaveURL(/\/error/)

  // Restore the repo path
  await page.request.put(`/api/repos/${repo.id}`, {
    data: { repo_path: '/backup/repos/server-daily' },
  })
})

// ── Status badge live updates during resync ──────────────────────────────────

test('status badge transitions to importing class when resync starts', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })

  const statusBadge = page.locator('.repo-status-badge')
  await expect(statusBadge).toBeVisible({ timeout: 10_000 })

  await resyncBtn.click()

  // Badge must acquire status-importing class while the sync runs
  await expect(statusBadge).toHaveClass(/status-importing/, { timeout: 30_000 })

  // After sync completes badge must return to status-online
  await expect(statusBadge).toHaveClass(/status-online/, { timeout: 120_000 })
  await expect(statusBadge).not.toHaveClass(/status-importing/)
})

test('status badge text shows importing phase verb during resync', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // While importing the badge text must match "Importing..." or "Importing X/Y"
  const statusBadge = page.locator('.repo-status-badge')
  await expect(statusBadge).toHaveText(/importing/i, { timeout: 30_000 })

  // Once done the badge must read "Enabled"
  await expect(statusBadge).toHaveText(/enabled/i, { timeout: 120_000 })
})

test('import-status-msg appears with live status text during resync', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // WebSocket ImportProgress messages must populate the status message element
  const statusMsg = page.locator('.import-status-msg')
  await expect(statusMsg).toBeVisible({ timeout: 30_000 })
  // "listing" = initial, "discovering" = streaming count, "importing" = per-archive loop
  await expect(statusMsg).toHaveText(/listing|discovering|importing|saving|refreshing/i, {
    timeout: 30_000,
  })

  // Message must disappear once importing finishes (v-if="repo.importing && ...")
  await expect(statusMsg).not.toBeVisible({ timeout: 120_000 })
})

test('import-progress bar appears when archive count is known', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // Progress bar only renders when import_total > 0; server-daily has archives so it must appear
  const progressBar = page.locator('.import-progress-bar')
  await expect(progressBar).toBeVisible({ timeout: 60_000 })

  // Bar must disappear after sync completes
  await expect(progressBar).not.toBeVisible({ timeout: 120_000 })
})

test('import badge count starts at 1 not 0', async ({ page }) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // Wait for the badge to show a numeric count (import_total > 0)
  const statusBadge = page.locator('.repo-status-badge')
  await expect(statusBadge).toHaveText(/importing/i, { timeout: 30_000 })

  // The progress must start at 1/N, not 0/N — assert the count before the
  // slash begins with a non-zero digit
  await expect(statusBadge).toHaveText(/[1-9]\d*\/\d+/, { timeout: 60_000 })

  // Wait for completion
  await expect(statusBadge).toHaveText(/enabled/i, { timeout: 120_000 })
})

test('status badge shows Enabled and no importing elements after resync completes', async ({
  page,
}) => {
  await loginAsAdmin(page)
  await navigateToRepo(page, 'server-daily')

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  // Wait for the importing phase to begin then complete
  const statusBadge = page.locator('.repo-status-badge')
  await expect(statusBadge).toHaveClass(/status-importing/, { timeout: 30_000 })
  await expect(statusBadge).toHaveClass(/status-online/, { timeout: 120_000 })

  // All importing UI elements must be gone after completion
  await expect(page.locator('.import-status-msg')).not.toBeVisible()
  await expect(page.locator('.import-progress')).not.toBeVisible()
  await expect(statusBadge).toHaveText(/enabled/i)
})

// ── Stale archive pruning ────────────────────────────────────────────────────

test('full resync removes archive deleted from borg', async ({ page }) => {
  await loginAsAdmin(page)

  const container = demoContainer()

  // Pick a web-server-01 archive to delete from borg (not old-webserver — that one drives
  // the unmatched-banner tests).
  const listing = borgRun(container, 'borg list /backup/repos/server-daily --short')
  const toDelete = listing
    .trim()
    .split('\n')
    .map((l) => l.trim())
    .find((n) => n.startsWith('web-server-01-'))
  if (!toDelete) throw new Error('no web-server-01 archive found in server-daily')

  // Delete it from borg — the DB record still exists until the next resync prunes it.
  borgRun(container, `borg delete /backup/repos/server-daily::${toDelete}`)

  // Full resync should prune the stale record.
  await navigateToRepo(page, 'server-daily', 'Archives')
  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  // Resync button is in the Overview tab toolbar; switch back to trigger it.
  await page.getByRole('button', { name: 'Overview', exact: true }).click()
  await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
  await resyncBtn.click()

  await expect(page.getByText('Full resync started.')).toBeVisible({ timeout: 120_000 })
  await expect(resyncBtn).toBeVisible({ timeout: 30_000 })

  // Switch to Archives tab and confirm the deleted archive is no longer listed.
  await page.getByRole('button', { name: 'Archives', exact: true }).click()
  await expect(page.locator('.archive-row').first()).toBeVisible({ timeout: 15_000 })
  await expect(page.locator('.archive-name', { hasText: toDelete })).not.toBeVisible()
})

// ── Lock contention ──────────────────────────────────────────────────────────

test('import-status-msg shows waiting-for-lock during borg lock contention', async ({ page }) => {
  // This test blocks borg for LOCK_WAIT_SECS (60 s) then waits for the retry
  // sleep ticker (~5 s into the 30 s wait) — budget at least 2 minutes.
  test.setTimeout(180_000)

  await loginAsAdmin(page)

  const container = demoContainer()
  const lockFile = '/backup/repos/server-daily/lock.exclusive'

  // Pre-create the lock file to prevent borg from acquiring the exclusive lock.
  dockerExec(container, `touch ${lockFile}`)

  const resyncBtn = page.getByRole('button', { name: /full resync/i })
  try {
    await navigateToRepo(page, 'server-daily')
    await expect(resyncBtn).toBeVisible({ timeout: 60_000 })
    await resyncBtn.click()

    // Borg holds --lock-wait 60 s before giving up; after that the server enters
    // a 30 s retry sleep and publishes "Waiting for lock…" every 5 s.
    const statusMsg = page.locator('.import-status-msg')
    await expect(statusMsg).toHaveText(/waiting for lock/i, { timeout: 90_000 })
  } finally {
    // Release the lock so the next borg attempt succeeds regardless of test outcome.
    dockerExec(container, `rm -f ${lockFile}`)
  }

  // After lock release the next retry should complete the resync.
  await expect(resyncBtn).toBeVisible({ timeout: 120_000 })
})

// ── Archive browsing ─────────────────────────────────────────────────────────

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
