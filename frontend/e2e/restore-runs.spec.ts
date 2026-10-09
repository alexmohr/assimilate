// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, loginAsViewer, seededIdByName, test } from './fixtures'
import type { Page } from '@playwright/test'

interface RestoreRun {
  id: string
  status: string
}

async function openRestoresTab(page: Page): Promise<void> {
  await page.goto('/activity')
  await page.locator('.segmented-option', { hasText: 'Restores' }).click()
  await expect(page.getByTestId('restore-runs')).toBeVisible()
}

/** Records a restore of `etc/hosts` from server-daily onto `hostname`. */
async function startRestore(page: Page, hostname: string): Promise<RestoreRun> {
  const repoId = await seededIdByName(page, '/api/repos', 'server-daily')
  const archives = await page.request.get(`/api/repos/${repoId}/archives`)
  expect(archives.ok()).toBeTruthy()
  const [archive] = (await archives.json()) as { name: string }[]
  expect(archive, 'server-daily must hold a seeded archive').toBeDefined()

  const response = await page.request.post(
    `/api/repos/${repoId}/archives/${encodeURIComponent(archive!.name)}/restore`,
    { data: { paths: ['etc/hosts'], target_path: '/tmp/e2e-restore', hostname } },
  )
  // Recorded and handed on in the background, not waited for.
  expect(response.status()).toBe(202)
  return (await response.json()) as RestoreRun
}

test.describe('restore runs', () => {
  test('the Restores tab lists the seeded restores with their outcome', async ({ page }) => {
    await loginAsAdmin(page)
    await openRestoresTab(page)

    const table = page.getByTestId('restore-runs')
    await expect(table).toContainText('Restored')
    await expect(table).toContainText('No space left on device')
    await expect(table).toContainText('Cancelled')
    await expect(table).toContainText('the whole archive')
  })

  test('a restore for an offline host waits for it and can be cancelled', async ({ page }) => {
    await loginAsAdmin(page)
    await openRestoresTab(page)

    const run = await startRestore(page, 'offline-due-01')
    expect(run.status).toBe('pending')

    // The new restore arrives at the top of the list as it is recorded.
    const row = page.getByTestId('restore-runs').locator('tbody tr').first()
    await expect(row).toContainText('offline-due-01')
    await expect(row).toContainText('Waiting for agent')

    await row.getByRole('button', { name: 'Cancel' }).click()
    await expect(row).toContainText('Cancelled')
    await expect(row.getByRole('button', { name: 'Cancel' })).toHaveCount(0)

    const cancelled = await page.request.get(`/api/restores/${run.id}`)
    expect(((await cancelled.json()) as RestoreRun).status).toBe('cancelled')
  })

  test('a restore onto a connected host runs on its agent and ends', async ({ page }) => {
    await loginAsAdmin(page)
    await openRestoresTab(page)

    const run = await startRestore(page, 'web-server-01')

    // Whatever borg makes of the target, the agent's answer settles it: it
    // does not stay waiting or running.
    const row = page.getByTestId('restore-runs').locator('tbody tr').first()
    await expect(row).toContainText('web-server-01')
    await expect(row).toContainText(/Restored|Failed/, { timeout: 60_000 })
    const finished = await page.request.get(`/api/restores/${run.id}`)
    expect(['success', 'failed']).toContain(((await finished.json()) as RestoreRun).status)
  })

  test('is not offered to viewers', async ({ page }) => {
    await loginAsViewer(page)
    await page.goto('/activity?category=restores')

    await expect(page.locator('.segmented-option.active')).toHaveText('All')
    await expect(page.locator('.segmented-option', { hasText: 'Restores' })).toHaveCount(0)
    expect((await page.request.get('/api/restores')).status()).toBe(403)
  })
})
