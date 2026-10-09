// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Page } from '@playwright/test'

const SECRET = 'e2e-very-secret-value'

function segment(page: Page, label: string): ReturnType<Page['locator']> {
  return page.locator('.segmented-option', { hasText: label })
}

/**
 * Makes the frontend log something of its own: the Server Logs tab logs a
 * failed fetch, and an uncaught error carrying a credential goes through the
 * global error capture. Both stay inside one page load, which is the
 * buffer's lifetime.
 */
async function produceBrowserLogs(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === '/api/logs',
    (route) => route.fulfill({ status: 500, body: 'boom' }),
  )
  await page.goto('/activity')
  await segment(page, 'Server Logs').click()
  await expect(page.locator('.state-msg')).toBeVisible()
  await page.evaluate((secret) => {
    setTimeout(() => {
      throw new Error(`e2e-uncaught token=${secret}`)
    }, 0)
  }, SECRET)
}

test.describe('browser logs', () => {
  test('shows, filters and clears what the frontend logged, with secrets redacted', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await produceBrowserLogs(page)

    await segment(page, 'Browser logs').click()
    const table = page.locator('.log-panel')
    await expect(table.getByText('fetchLogs failed')).toBeVisible()
    await expect(
      table.locator('.cell-msg-log').getByText('e2e-uncaught token=[REDACTED]'),
    ).toBeVisible()
    await expect(page.locator('body')).not.toContainText(SECRET)

    await page.locator('input.search-input').fill('e2e-uncaught')
    await expect(table.getByText('fetchLogs failed')).toHaveCount(0)
    await expect(
      table.locator('.cell-msg-log').getByText('e2e-uncaught token=[REDACTED]'),
    ).toBeVisible()

    await page.getByRole('button', { name: 'Clear logs' }).click()
    await expect(page.locator('.state-msg')).toHaveText(
      'Nothing has been logged in this browser tab yet.',
    )
  })

  test('fits a phone-width viewport', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 })
    await loginAsAdmin(page)
    await produceBrowserLogs(page)

    await segment(page, 'Browser logs').click()
    await expect(page.locator('.log-panel').getByText('fetchLogs failed')).toBeVisible()
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    )
    expect(overflow).toBeLessThanOrEqual(0)
  })
})
