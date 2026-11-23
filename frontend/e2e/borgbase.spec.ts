// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'
import type { Locator, Page } from '@playwright/test'

/**
 * The BorgBase account panel on the System page. The demo server talks to a stand-in for the
 * BorgBase API (`.devcontainer/demo/borgbase-stand-in.py`) that accepts the token `demo-token`
 * - the seed stores it - and refuses every other one.
 */

async function openBorgbasePanel(page: Page): Promise<Locator> {
  await loginAsAdmin(page)
  await page.goto('/system')
  const panel = await openSettingsSection(page, 'BorgBase')
  await expect(panel.getByTestId('borgbase-repo').first()).toBeVisible({ timeout: 15_000 })
  return panel
}

test('the account panel lists the repositories with their hosts and usage', async ({ page }) => {
  const panel = await openBorgbasePanel(page)

  const laptop = panel.getByTestId('borgbase-repo').filter({ hasText: 'laptop' })
  await expect(laptop).toContainText('ab12cd34.repo.borgbase.com')
  await expect(laptop).toContainText('12.5')
  await expect(laptop).toContainText('500')
  const nas = panel.getByTestId('borgbase-repo').filter({ hasText: 'nas' })
  await expect(nas).toContainText('Not enforced')
})

test('a token BorgBase refuses is not stored and the stored one keeps working', async ({
  page,
}) => {
  const panel = await openBorgbasePanel(page)

  await panel.locator('#borgbase-token').fill('not-the-token')
  await panel.getByRole('button', { name: 'Save token' }).click()

  await expect(panel.locator('.form-error')).toBeVisible()
  await expect(panel.getByTestId('borgbase-repo').first()).toBeVisible()
})

test('a repository is created for the servers key and shows how to add it', async ({ page }) => {
  const panel = await openBorgbasePanel(page)
  const name = `e2e-offsite-${Date.now()}`

  await panel.locator('#borgbase-repo-name').fill(name)
  await panel.locator('#borgbase-repo-region').selectOption('us')
  await panel.locator('#borgbase-repo-access').selectOption('append_only')
  await Promise.all([
    page.waitForResponse(
      (resp) =>
        resp.url().endsWith('/api/borgbase/repos') &&
        resp.request().method() === 'POST' &&
        resp.status() === 200,
    ),
    panel.getByRole('button', { name: 'Create repository' }).click(),
  ])

  const created = panel.getByTestId('borgbase-created')
  await expect(created).toContainText(name)
  await expect(created).toContainText('.repo.borgbase.com')
  await expect(created).toContainText('./repo')
  await expect(panel.getByTestId('borgbase-repo').filter({ hasText: name })).toBeVisible()
})
