// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A host whose agent cannot find borg (docs/agents.md#borg-not-installed).
 * The demo's build-runner-01 container sets BORG_BINARY to a path that does
 * not exist, so its agent reports a missing borg when it connects; the other
 * demo agents have borg.
 */
test.describe('Agent without borg', () => {
  test('the hosts list flags the host and its page explains the fix', async ({ page }) => {
    await loginAsAdmin(page)

    await page.goto('/agents')
    const row = page.locator('.list-row').filter({ hasText: 'build-runner-01' }).first()
    await expect(row.locator('.entity-issue-chip')).toHaveText('Borg not installed', {
      timeout: 30_000,
    })

    await row.locator('.entity-issue-chip').click()
    const notice = page.getByTestId('borg-notice')
    await expect(notice).toContainText('Borg is not installed on this host')
    await expect(notice).toContainText('BORG_BINARY')
  })

  test('Check again asks the agent to look for borg again', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/build-runner-01')

    const request = page.waitForResponse(
      (response) =>
        response.url().endsWith('/api/agents/build-runner-01/check-borg') &&
        response.request().method() === 'POST',
    )
    await page
      .getByTestId('borg-notice')
      .getByRole('button', { name: 'Check again' })
      .click({ timeout: 30_000 })

    expect((await request).status()).toBe(202)
    // The agent answers the same: still missing.
    await expect(page.getByTestId('borg-notice')).toBeVisible()
  })

  test('a host whose agent has borg shows no warning', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/web-server-01')
    await page.waitForLoadState('networkidle')
    await expect(page.getByText('web-server-01').first()).toBeVisible()
    await expect(page.getByTestId('borg-notice')).toHaveCount(0)
  })
})
