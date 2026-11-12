// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, test } from './fixtures'

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
    await page.locator('a.list-row').filter({ hasText: 'media-weekly' }).first().click()
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Settings' }).click()

    const pane = page.locator('.settings-pane')
    await expect(pane).toContainText('SSH user')
    await pane.getByRole('link', { name: /^localhost:22$/ }).click()
    await page.waitForLoadState('networkidle')

    await expect(page).toHaveURL(/\/repo-hosts\/\d+/)
    // The route transition keeps the repository page mounted while it fades out.
    await expect(page.locator('.repo-detail')).toHaveCount(0)
    await expect(page.locator('.detail-name')).toHaveText('localhost')
    // Not always online, named neutrally in the header.
    await expect(page.locator('.detail-header')).toContainText('Intermittent')

    // Connection: where the host is reached, and the key every repository on
    // it is verified against - pinned when the demo repositories were added.
    const hostPane = page.locator('.settings-pane')
    await expect(hostPane).toContainText('Hostname')
    await expect(hostPane).toContainText('SSH host key')
    await expect(hostPane).not.toContainText('Not pinned yet')

    const repositories = await openSettingsSection(page, 'Repositories')
    for (const name of ['database-hourly', 'media-weekly', 'stale-report-repo']) {
      await expect(repositories.getByRole('link', { name })).toBeVisible()
    }
    await expect(repositories.getByRole('link', { name: 'server-daily' })).toHaveCount(0)

    // A host its repositories still use cannot be removed.
    const danger = await openSettingsSection(page, 'Danger zone')
    await expect(danger.getByRole('button', { name: 'Remove host' })).toBeDisabled()
  })

  test('the grouped repository list marks a host that is not always online Intermittent', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')

    // Each host heads its repositories: a dot for its last check, its name
    // linking to the host, and - for one that may be away - "Intermittent".
    const header = (host: string) =>
      page.locator('.pool-header').filter({
        has: page.getByRole('link', { name: host, exact: true }),
      })
    const sleeping = header('localhost')
    await expect(sleeping.locator('.pool-note')).toContainText('Intermittent')
    await expect(sleeping.locator('.pool-note')).toHaveAttribute('title', /Can be offline/)
    await expect(sleeping.getByRole('link', { name: 'localhost', exact: true })).toHaveAttribute(
      'href',
      /\/repo-hosts\/\d+/,
    )
    await expect(sleeping.locator('.pool-host .status-dot')).toHaveAttribute('aria-label', /.+/)
    await expect(header('demo')).not.toContainText('Intermittent')
  })
})
