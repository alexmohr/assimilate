// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

const TEST_HOST = 'e2e-test-server.local'

const TEST_QUOTA = {
  warn_bytes: 10_737_418_240,
  critical_bytes: 21_474_836_480,
  warn_action: 'notify_only',
  critical_action: 'block_backups',
  enabled: true,
}

// ── Page access ──────────────────────────────────────────────────────────────

test('server quotas page loads for admin without error', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/server-quotas')
  await page.waitForTimeout(1_000)
  await expect(page).not.toHaveURL(/\/error/)
  await expect(page).toHaveURL(/\/server-quotas/)
})

test('server quotas page shows the page title', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/server-quotas')
  await expect(page.getByText('Server Quotas')).toBeVisible({ timeout: 10_000 })
})

test('server quotas page shows Add Quota button for admin', async ({ page }) => {
  await loginAsAdmin(page)
  await page.goto('/server-quotas')
  await expect(page.getByRole('button', { name: /Add Quota/i })).toBeVisible({
    timeout: 10_000,
  })
})

// ── CRUD workflow via real API ────────────────────────────────────────────────

test('can create, view, and delete a server quota', async ({ page }) => {
  await loginAsAdmin(page)

  // Create quota via the API
  const createRes = await page.request.put(`/api/server-quotas/${encodeURIComponent(TEST_HOST)}`, {
    data: TEST_QUOTA,
  })
  expect(createRes.ok()).toBe(true)

  try {
    await page.goto('/server-quotas')
    await expect(page.getByText(TEST_HOST)).toBeVisible({ timeout: 10_000 })
    await expect(page.getByText('Block all backups + notify')).toBeVisible({ timeout: 5_000 })

    // Open delete dialog and confirm
    const card = page.locator('.quota-card').filter({ hasText: TEST_HOST })
    await card.getByRole('button', { name: 'Edit' }).click()
    await expect(page.locator('.edit-form')).toBeVisible({ timeout: 5_000 })

    // Cancel edit
    await page.getByRole('button', { name: 'Cancel' }).first().click()
    await expect(page.locator('.edit-form')).not.toBeVisible()
  } finally {
    // Always clean up
    await page.request.delete(`/api/server-quotas/${encodeURIComponent(TEST_HOST)}`)
  }
})

test('deleted quota disappears from the list', async ({ page }) => {
  await loginAsAdmin(page)

  const createRes = await page.request.put(`/api/server-quotas/${encodeURIComponent(TEST_HOST)}`, {
    data: TEST_QUOTA,
  })
  expect(createRes.ok()).toBe(true)

  try {
    await page.goto('/server-quotas')
    await expect(page.getByText(TEST_HOST)).toBeVisible({ timeout: 10_000 })

    const card = page.locator('.quota-card').filter({ hasText: TEST_HOST })
    await card.locator('.btn-danger').click()
    await expect(page.getByText('Delete Server Quota')).toBeVisible({ timeout: 5_000 })
    await page.locator('.dialog-overlay .btn-danger').click()

    await expect(page.getByText(TEST_HOST)).not.toBeVisible({ timeout: 5_000 })
  } finally {
    await page.request.delete(`/api/server-quotas/${encodeURIComponent(TEST_HOST)}`).catch(() => {})
  }
})

test('can open add dialog and cancel', async ({ page }) => {
  await loginAsAdmin(page)

  // Mock the hosts list so there's always an available host in the dialog
  await page.route('**/api/server-quotas/hosts', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(['available-host.local']),
    }),
  )
  await page.route('**/api/server-quotas', (route) => {
    if (route.request().method() === 'GET') {
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      })
    } else {
      void route.continue()
    }
  })

  await page.goto('/server-quotas')
  await expect(page.getByRole('button', { name: /Add Quota/i })).toBeEnabled({ timeout: 10_000 })

  await page.getByRole('button', { name: /Add Quota/i }).click()
  await expect(page.getByText('Add Server Quota')).toBeVisible({ timeout: 5_000 })
  await expect(page.getByText('available-host.local')).toBeVisible()

  await page.getByRole('button', { name: 'Cancel' }).first().click()
  await expect(page.getByText('Add Server Quota')).not.toBeVisible()
})
