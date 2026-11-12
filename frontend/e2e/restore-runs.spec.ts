// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, loginAsViewer, seededIdByName, test } from './fixtures'
import type { Page } from '@playwright/test'

interface RestoreRun {
  id: string
  status: string
}

/**
 * The row of restore `id`. Other restores (seeded ones, and those an earlier
 * attempt of a test left behind) can share its host and status, so a row is
 * only ever picked by id.
 */
function restoreRow(page: Page, id: string): ReturnType<Page['locator']> {
  return page.getByTestId('restore-runs').locator(`[data-restore-id="${id}"]`)
}

/** The Activity log's type pills: All, Backup, System, Restores, ... */
function activityType(page: Page): ReturnType<Page['locator']> {
  return page.getByRole('group', { name: 'Activity type' })
}

async function openRestoresTab(page: Page): Promise<void> {
  await page.goto('/activity')
  await activityType(page).getByRole('button', { name: 'Restores' }).click()
  await expect(page.getByTestId('restore-runs')).toBeVisible()
}

/**
 * Opens a restore's row, where its status is named (the row itself shows it
 * as a dot), unless it is open already.
 */
async function openRow(row: ReturnType<Page['locator']>): Promise<void> {
  const summary = row.locator('.log-row-summary')
  if ((await summary.getAttribute('aria-expanded')) !== 'true') await summary.click()
  await expect(row.locator('.log-row-detail')).toBeVisible()
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
    await expect(table).toContainText('No space left on device')
    await expect(table).toContainText('the whole archive')

    // Each row names its outcome once opened.
    const rows = table.locator('.log-row')
    const statuses: string[] = []
    const count = await rows.count()
    for (let i = 0; i < count; i++) {
      await openRow(rows.nth(i))
      statuses.push(await rows.nth(i).locator('.log-row-detail').innerText())
    }
    expect(statuses.join('\n')).toContain('Restored')
    expect(statuses.join('\n')).toContain('Cancelled')
  })

  test('a restore for an offline host waits for it and can be cancelled', async ({ page }) => {
    await loginAsAdmin(page)
    await openRestoresTab(page)

    const run = await startRestore(page, 'offline-due-01')
    expect(run.status).toBe('pending')

    // The new restore arrives at the top of the list as it is recorded.
    const row = restoreRow(page, run.id)
    await expect(page.getByTestId('restore-runs').locator('.log-row').first()).toHaveAttribute(
      'data-restore-id',
      run.id,
    )
    await expect(row).toContainText('offline-due-01')
    await openRow(row)
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
    const row = restoreRow(page, run.id)
    await expect(row).toContainText('web-server-01')
    await openRow(row)
    await expect(row.locator('.log-row-detail')).toContainText(/Restored|Failed/, {
      timeout: 60_000,
    })
    const finished = await page.request.get(`/api/restores/${run.id}`)
    expect(['success', 'failed']).toContain(((await finished.json()) as RestoreRun).status)
  })

  test('a restore to an unsafe folder is refused and never recorded', async ({ page }) => {
    await loginAsAdmin(page)
    const repoId = await seededIdByName(page, '/api/repos', 'server-daily')
    const archives = await page.request.get(`/api/repos/${repoId}/archives`)
    const [archive] = (await archives.json()) as { name: string }[]
    const targets = ['tmp/e2e-unsafe-restore', '/tmp/e2e-unsafe/../etc']

    for (const target of targets) {
      const response = await page.request.post(
        `/api/repos/${repoId}/archives/${encodeURIComponent(archive!.name)}/restore`,
        { data: { paths: ['etc/hosts'], target_path: target, hostname: 'web-server-01' } },
      )
      expect(response.status(), `${target} must be refused`).toBe(400)
    }

    const runs = (await (await page.request.get('/api/restores')).json()) as {
      target_path: string | null
    }[]
    expect(runs.filter((run) => targets.includes(run.target_path ?? ''))).toEqual([])
  })

  test('is not offered to viewers', async ({ page }) => {
    await loginAsViewer(page)
    await page.goto('/activity?category=restores')

    await expect(activityType(page).locator('[aria-pressed="true"]')).toHaveText('All')
    await expect(activityType(page).getByRole('button', { name: 'Restores' })).toHaveCount(0)
    expect((await page.request.get('/api/restores')).status()).toBe(403)
  })
})
