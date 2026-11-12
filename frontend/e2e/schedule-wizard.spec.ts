// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A new backup schedule through every step of the wizard: basics, sources,
 * targets, power, timing, retention, advanced and review. schedules.spec.ts
 * covers an integrity check, which skips retention and advanced; this one is
 * a backup, asks for database dumps on web-server-01 - which has them turned
 * off (see `.devcontainer/demo/seed-demo.sh`) - and is created paused, so it
 * never runs. The test removes it again.
 */
test.describe('New schedule wizard', () => {
  test('a paused backup schedule through every step, with its review', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules')
    await page.getByRole('link', { name: 'New', exact: true }).click()
    await expect(page).toHaveURL(/\/schedules\/new$/)

    const rail = page.getByRole('navigation', { name: 'Schedule steps' })
    const current = page.locator('.wizard-step--current')
    const foot = page.locator('.wizard-foot')
    const next = foot.getByRole('button', { name: 'Continue' })
    const create = foot.getByRole('button', { name: 'Create schedule', exact: true })

    // Basics. A backup has eight steps.
    await expect(rail.getByRole('button')).toHaveCount(8)
    await expect(next).toBeDisabled()
    await expect(foot).toContainText('Needs a name to continue.')
    const name = `e2e wizard backup ${Date.now()}`
    await page.locator('#schedule-name').fill(name)
    await expect(page.locator('#schedule-type')).toHaveValue('backup')
    const enabled = page.getByRole('switch', { name: 'Enabled' })
    if ((await enabled.getAttribute('aria-checked')) === 'true') await enabled.click()
    await expect(enabled).toHaveAttribute('aria-checked', 'false')
    await next.click()

    // Sources: a host and what to read from it.
    await expect(current).toContainText('Sources')
    await expect(foot).toContainText('Needs at least one host to continue.')
    await page.locator('.multi-select-trigger').click()
    await page.getByRole('checkbox', { name: 'Production Web Server' }).check()
    await page.locator('.multi-select-trigger').click()
    await page.locator('#backup-paths').fill('/etc')
    await next.click()

    // Targets: the first repository is preselected.
    await expect(current).toContainText('Targets')
    await expect(page.locator('.order-item')).toHaveCount(1)
    await next.click()

    // Power and Timing need nothing: each host's own wake setting, and a
    // prefilled cron expression.
    await expect(current).toContainText('Power')
    await next.click()
    await expect(current).toContainText('Timing')
    await next.click()

    // Retention: what is kept, prefilled.
    await expect(current).toContainText('Retention')
    await expect(page.locator('#keep-daily')).not.toHaveValue('')
    await next.click()

    // Advanced: asking for database dumps says up front that this host has
    // them turned off.
    await expect(current).toContainText('Advanced')
    await expect(page.getByTestId('database-dump-mismatch')).toHaveCount(0)
    await page.getByRole('switch', { name: 'Dump databases' }).click()
    const mismatch = page.getByTestId('database-dump-mismatch')
    await expect(mismatch).toContainText('No databases will be dumped on 1 host')
    await expect(mismatch.getByRole('link')).toHaveAttribute('href', /\/agents\/web-server-01\?/)
    await next.click()

    // Review: nothing missing, and every block says what will be made.
    await expect(current).toContainText('Review')
    await expect(page.locator('.wizard-note--success')).toContainText(
      'Each of 1 host writes 1 copy',
    )
    const review = page.locator('.review-grid')
    await expect(review).toContainText(name)
    await expect(review).toContainText('Backup')
    await expect(review).toContainText('Created paused')
    await expect(review).toContainText('Production Web Server')
    await expect(review).toContainText('/etc')

    // A block's Edit goes back to the step that owns it, and the rail returns.
    await review
      .locator('.review-block')
      .filter({ hasText: 'Sources' })
      .getByRole('button', { name: 'Edit' })
      .click()
    await expect(current).toContainText('Sources')
    await rail.getByRole('button', { name: /Review/ }).click()
    await expect(current).toContainText('Review')

    const [created] = await Promise.all([
      page.waitForResponse(
        (resp) => resp.url().endsWith('/api/schedules') && resp.request().method() === 'POST',
      ),
      create.click(),
    ])
    expect(created.ok()).toBe(true)
    expect(created.request().postDataJSON()).toMatchObject({
      name,
      enabled: false,
      database_backup_enabled: true,
    })
    const { id } = (await created.json()) as { id: number }
    try {
      // The new schedule's own page carries the same warning.
      await expect(page).toHaveURL(new RegExp(`/schedules/${id}$`))
      await expect(page.getByRole('heading', { name })).toBeVisible()
      await expect(page.getByTestId('database-dump-mismatch')).toBeVisible()
    } finally {
      const removed = await page.request.delete(`/api/schedules/${id}`)
      expect(removed.ok()).toBe(true)
    }
  })
})
