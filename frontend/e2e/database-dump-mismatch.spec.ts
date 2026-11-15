// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, scheduleIdByName, test } from './fixtures'

/**
 * A backup dumps the database sources each host includes. The seed's disabled
 * "Database dumps" schedule takes them on web-server-01 (no sources) and
 * db-server-01 (several), so its page warns about web-server-01 alone (see
 * `.devcontainer/demo/seed-demo.sh`).
 */
test.describe('Databases a host has none of', () => {
  test("the schedule's page names the host, linking to its database settings", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Database dumps')
    await page.goto(`/schedules/${id}`)

    const warning = page.getByTestId('source-not-set-up-databases')
    await expect(warning).toBeVisible({ timeout: 15_000 })
    await expect(warning).toContainText('No databases will be dumped on 1 host')
    await expect(warning).toContainText('none are set up on')

    // web-server-01, by the name the agent list gives it; db-server-01 has
    // sources, so it is not named.
    const hosts = warning.getByRole('link')
    await expect(hosts).toHaveCount(1)
    await expect(hosts).toHaveText(/Production Web Server|web-server-01/)
    await expect(hosts).toHaveAttribute(
      'href',
      /^\/agents\/web-server-01\?tab=sources&section=databases/,
    )

    // The link lands on the host's database settings, which name the schedule
    // that asks for what the host does not have.
    await hosts.click()
    await expect(page).toHaveURL(/\/agents\/web-server-01\?tab=sources&section=databases/)
    const asked = page.getByTestId('database-dump-asked')
    await expect(asked).toBeVisible({ timeout: 15_000 })
    await expect(asked.getByRole('link', { name: 'Database dumps' })).toHaveAttribute(
      'href',
      `/schedules/${id}`,
    )
  })

  test("the schedule's Sources settings warn under Databases", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Database dumps')
    await page.goto(`/schedules/${id}?tab=settings&section=sources`)

    const sources = await openSettingsSection(page, 'Sources')
    await expect(sources.getByRole('switch', { name: 'Databases' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    const warning = sources.getByTestId('source-not-set-up-databases')
    await expect(warning).toContainText('No databases will be dumped on 1 host')
    await expect(warning.getByRole('link')).toHaveAttribute('href', /\/agents\/web-server-01\?/)

    // Not taking databases leaves nothing to warn about. Not saved.
    await sources.getByRole('switch', { name: 'Databases' }).click()
    await expect(sources.getByTestId('source-not-set-up-databases')).toHaveCount(0)
  })
})
