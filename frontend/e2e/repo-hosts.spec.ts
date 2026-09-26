// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A repository host: the machine borg writes to, with its address, pinned SSH
 * host key, power and availability set once for every repository on it. The
 * demo's database-hourly, media-weekly and stale-report-repo share the
 * "localhost" host, which the seed marks as not always online; server-daily
 * has the "demo" host to itself (see `.devcontainer/demo/seed-demo.sh`).
 */
test.describe('Repository hosts', () => {
  test('a repository links to its host, which lists every repository on it', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.locator('.entity-card').filter({ hasText: 'media-weekly' }).first().click()
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Settings' }).click()

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('SSH user')
    await pane.getByRole('link', { name: /^localhost:22$/ }).click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repo-hosts\/\d+/)
    await expect(page.locator('.detail-name')).toHaveText('localhost')
    await expect(page.locator('.detail-header')).toContainText('Not always online')

    // Connection: where the host is reached, and the key every repository on
    // it is verified against - pinned when the demo repositories were added.
    const hostPane = page.locator('.settings-pane')
    await expect(hostPane).toContainText('Hostname')
    await expect(hostPane).toContainText('SSH host key')
    await expect(hostPane).not.toContainText('Not pinned yet')

    await page.locator('.settings-nav-item', { hasText: 'Repositories' }).click()
    for (const name of ['database-hourly', 'media-weekly', 'stale-report-repo']) {
      await expect(page.locator('.settings-pane').getByRole('link', { name })).toBeVisible()
    }
    await expect(
      page.locator('.settings-pane').getByRole('link', { name: 'server-daily' }),
    ).toHaveCount(0)

    // A host its repositories still use cannot be removed.
    await page.locator('.settings-nav-item', { hasText: 'Danger zone' }).click()
    await expect(page.getByRole('button', { name: 'Remove host' })).toBeDisabled()
  })

  test('the grouped repository list flags a host that is not always online', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    const sleeping = page.locator('.pool-host', { hasText: 'localhost' })
    await expect(sleeping).toContainText('Not always online')
    await expect(sleeping.getByRole('link', { name: 'localhost' })).toHaveAttribute(
      'href',
      /\/repo-hosts\/\d+/,
    )
    await expect(page.locator('.pool-host', { hasText: 'demo' })).not.toContainText(
      'Not always online',
    )
  })
})
