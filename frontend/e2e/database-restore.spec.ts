// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expandAllArchiveGroups, expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

interface ArchiveRow {
  name: string
  start: string
}

interface RestoreRun {
  id: string
  status: string
  target_path: string | null
}

/**
 * Opens database-hourly's newest db-server-01 archive, which the demo agent
 * writes with database dumps beside its files, and switches to its
 * Databases view.
 */
async function openDatabasesView(page: Page): Promise<void> {
  await page.goto('/repos', { waitUntil: 'commit' })
  await page.getByText('database-hourly', { exact: true }).click()
  await page.waitForURL(/\/repos\/\d+/, { waitUntil: 'commit' })
  const repoId = Number(/\/repos\/(\d+)/.exec(page.url())?.[1])

  const archives = await page.request.get(`/api/repos/${repoId}/archives`)
  expect(archives.ok()).toBeTruthy()
  const [newest] = ((await archives.json()) as ArchiveRow[])
    .filter((archive) => archive.name.startsWith('db-server-01-backup-'))
    .sort((a, b) => b.start.localeCompare(a.start))
  expect(newest, 'database-hourly must hold a seeded db-server-01 archive').toBeDefined()

  await page.getByRole('tab', { name: 'Archives', exact: true }).click()
  await expandAllArchiveGroups(page)
  await page.getByText(newest!.name, { exact: true }).first().click()

  await page.locator('.browser-view .segmented-option', { hasText: 'Databases' }).click()
  await expect(page.getByTestId('archive-database-dumps')).toBeVisible({ timeout: 30_000 })
}

test.describe('database restore', () => {
  test('lists the dumps an archive holds, with each source named', async ({ page }) => {
    await loginAsAdmin(page)
    await openDatabasesView(page)

    const dumps = page.getByTestId('archive-database-dumps')
    await expect(page.locator('.browser-title-label')).toHaveText('Databases')
    for (const text of [
      'orders',
      'payments',
      'shop/orders-db',
      'pod orders-db-2',
      'gitea',
      'gitea/db',
      'container gitea-db-1',
      'inventory',
      'PostgreSQL on this host',
    ]) {
      await expect(dumps).toContainText(text)
    }
  })

  test('restores a dump into a new database and lists the restore', async ({ page }) => {
    await loginAsAdmin(page)
    await openDatabasesView(page)

    await page
      .getByTestId('archive-database-dumps')
      .locator('button[title="Restore orders"]')
      .click()

    const dialog = page.getByRole('dialog', { name: 'Restore database' })
    await expect(dialog.locator('#db-restore-host')).toHaveValue('db-server-01')
    await expect(dialog.locator('#db-restore-name')).toHaveValue('orders_restored')

    // The server checks the name too; the dialog says why before it is sent.
    await dialog.locator('#db-restore-name').fill('pg_orders')
    await expect(dialog.getByTestId('db-restore-name-problem')).toContainText('reserved')
    await expect(dialog.getByTestId('db-restore-submit')).toBeDisabled()
    await dialog.locator('#db-restore-name').fill('orders_restored')

    const started = page.waitForResponse(
      (response) =>
        response.url().includes('/databases/restore') && response.request().method() === 'POST',
    )
    await dialog.getByTestId('db-restore-submit').click()
    const response = await started
    expect(response.status()).toBe(202)
    const run = (await response.json()) as RestoreRun
    expect(run.target_path).toBeNull()

    // The demo has no database server behind the agent, so the restore may
    // fail there; either way the dialog follows it to an answer.
    await expect(dialog.getByTestId('db-restore-status')).toBeVisible()
    await dialog.getByRole('button', { name: 'Done' }).click()

    await page.goto('/activity')
    await page.locator('.segmented-option', { hasText: 'Restores' }).click()
    const restore = page.getByTestId('restore-runs').locator(`tr[data-restore-id="${run.id}"]`)
    await expect(restore).toContainText('orders into orders_restored on shop/orders-db')
    await expect(restore).not.toContainText('null')
  })

  test('replacing a database needs its name typed', async ({ page }) => {
    await loginAsAdmin(page)
    await openDatabasesView(page)

    await page
      .getByTestId('archive-database-dumps')
      .getByRole('button', { name: 'Restore' })
      .first()
      .click()
    const dialog = page.getByRole('dialog', { name: 'Restore database' })
    await dialog.locator('.segmented-option', { hasText: 'Replace orders' }).click()

    await expect(dialog.getByTestId('db-restore-replace-warning')).toContainText('one transaction')
    const submit = dialog.getByTestId('db-restore-submit')
    await expect(submit).toHaveText('Replace orders')
    await expect(submit).toBeDisabled()
    await dialog.locator('#db-restore-confirm').fill('orders')
    await expect(submit).toBeEnabled()

    // Not sent: replacing is destructive, and this test only checks the guard.
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await expect(dialog).toHaveCount(0)
  })
})
