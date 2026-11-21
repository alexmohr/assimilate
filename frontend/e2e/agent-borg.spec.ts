// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, mockMissingBorg, test } from './fixtures'

/**
 * A host whose agent cannot find borg (docs/agents.md#borg-not-installed).
 * The demo's agents all have borg, so the server's report for web-server-01
 * is faked; the "Check again" request goes to the real server and agent.
 */
test.describe('Agent without borg', () => {
  test('the hosts list flags the host and its page explains the fix', async ({ page }) => {
    await mockMissingBorg(page, 'web-server-01')
    await loginAsAdmin(page)

    await page.goto('/agents')
    const row = page.locator('.list-row').filter({ hasText: 'web-server-01' }).first()
    await expect(row.locator('.entity-issue-chip')).toHaveText('Borg not installed')

    await row.locator('.entity-issue-chip').click()
    const notice = page.getByTestId('borg-notice')
    await expect(notice).toContainText('Borg is not installed on this host')
    await expect(notice).toContainText('BORG_BINARY')
  })

  test('Check again asks the agent to look for borg', async ({ page }) => {
    await mockMissingBorg(page, 'web-server-01')
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01')

    const request = page.waitForResponse(
      (response) =>
        response.url().endsWith('/api/agents/web-server-01/check-borg') &&
        response.request().method() === 'POST',
    )
    await page.getByTestId('borg-notice').getByRole('button', { name: 'Check again' }).click()

    expect((await request).status()).toBe(202)
  })

  test('a host whose agent has borg shows no warning', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01')
    await page.waitForLoadState('networkidle')
    await expect(page.getByText('web-server-01').first()).toBeVisible()
    await expect(page.getByTestId('borg-notice')).toHaveCount(0)
  })
})
