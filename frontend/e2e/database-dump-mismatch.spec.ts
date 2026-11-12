// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, openSettingsSection, scheduleIdByName, test } from './fixtures'

/**
 * A backup dumps databases only when the schedule asks for them and the host
 * allows them. The seed's disabled "Database dumps" schedule asks for them on
 * web-server-01 (dumps turned off) and db-server-01 (dumps on), so its page
 * warns about web-server-01 alone (see `.devcontainer/demo/seed-demo.sh`).
 */
test.describe('Database dumps a host has turned off', () => {
  test("the schedule's page names the host, linking to its database settings", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Database dumps')
    await page.goto(`/schedules/${id}`)

    const warning = page.getByTestId('database-dump-mismatch')
    await expect(warning).toBeVisible({ timeout: 15_000 })
    await expect(warning).toContainText('No databases will be dumped on 1 host')
    await expect(warning).toContainText('database dumps are turned off on')

    // web-server-01, by the name the agent list gives it; db-server-01 allows
    // dumps, so it is not named.
    const hosts = warning.getByRole('link')
    await expect(hosts).toHaveCount(1)
    await expect(hosts).toHaveText(/Production Web Server|web-server-01/)
    await expect(hosts).toHaveAttribute(
      'href',
      /^\/agents\/web-server-01\?tab=sources&section=databases/,
    )

    // The link lands on the host's database settings, which name the schedule
    // that asks for what the host does not allow.
    await hosts.click()
    await expect(page).toHaveURL(/\/agents\/web-server-01\?tab=sources&section=databases/)
    const asked = page.getByTestId('database-dump-asked')
    await expect(asked).toBeVisible({ timeout: 15_000 })
    await expect(asked.getByRole('link', { name: 'Database dumps' })).toHaveAttribute(
      'href',
      `/schedules/${id}`,
    )
  })

  test("the schedule's Advanced settings warn under Dump databases", async ({ page }) => {
    await loginAsAdmin(page)
    const id = await scheduleIdByName(page, 'Database dumps')
    await page.goto(`/schedules/${id}?tab=settings&section=advanced`)

    const advanced = await openSettingsSection(page, 'Advanced')
    await expect(advanced.getByRole('switch', { name: 'Dump databases' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    const warning = advanced.getByTestId('database-dump-mismatch')
    await expect(warning).toContainText('No databases will be dumped on 1 host')
    await expect(warning.getByRole('link')).toHaveAttribute('href', /\/agents\/web-server-01\?/)

    // Not asking for dumps leaves nothing to warn about. Not saved.
    await advanced.getByRole('switch', { name: 'Dump databases' }).click()
    await expect(advanced.getByTestId('database-dump-mismatch')).toHaveCount(0)
  })
})
