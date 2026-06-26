// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

// The demo environment builds both the server and all agents from the same
// commit, so agent versions always match the server's advertised version.
// No Upgrade button should be shown for any connected agent in the demo.

test('agents page loads and shows connected demo agents', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  await expect(page.getByText('web-server-01')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('db-server-01')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText('media-store-01')).toBeVisible({ timeout: 10_000 })
})

test('no Upgrade button shown for connected demo agents', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/agents')
  // Wait for agents to render.
  await expect(page.locator('.card-hostname').first()).toBeVisible({ timeout: 10_000 })
  // All demo agents are built from the same binary, so no agent should be behind.
  await expect(page.getByRole('button', { name: 'Upgrade' })).not.toBeVisible()
})
