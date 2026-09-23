// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'
import type { Locator, Page } from '@playwright/test'

/**
 * Whether a host is waited for, set where the host is: the "When the host is
 * offline" section of a repository's and an agent's Power pane.
 *
 * The seed marks the demo's "not always on" pair - media-store-01 and the
 * media-weekly NAS it writes to - as not always online, and leaves a run
 * waiting on media-weekly (see `.devcontainer/demo/seed-demo.sh`), because
 * waiting out a real outage against a host that never answers is not something
 * an e2e run can do.
 */
test.describe('catch-up on the host', () => {
  async function openRepoPower(page: Page, name: string): Promise<void> {
    await page.goto('/repos')
    await page.waitForLoadState('networkidle')
    await page.locator('.entity-card').filter({ hasText: name }).first().click()
    await page.waitForLoadState('networkidle')
    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.locator('.settings-nav-item', { hasText: 'Power' }).click()
    await page.waitForLoadState('networkidle')
  }

  /** The availability section, below the power card in the same pane. */
  function availability(page: Page): Locator {
    return page.locator('.pane-section', { hasText: 'When the host is offline' })
  }

  test('a repository marked as not always online saves its interval and window', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await openRepoPower(page, 'media-weekly')

    const section = availability(page)
    await expect(section).toContainText('Host is not always online')
    await expect(section).toContainText('15 minutes')
    await expect(section).toContainText('3 days')

    await section.getByRole('button', { name: 'Edit' }).click()
    await expect(
      section.getByRole('switch', { name: 'Host is not always online' }),
    ).toHaveAttribute('aria-checked', 'true')
    await section.locator('#availability-recheck').fill('30')
    await section.getByLabel('Give-up window unit').selectOption('weeks')
    await section.locator('#availability-give-up').fill('1')
    await section.getByRole('button', { name: 'Save' }).click()

    await expect(section.getByRole('button', { name: 'Edit' })).toBeVisible()
    await expect(section).toContainText('30 minutes')
    await expect(section).toContainText('1 week')

    await page.reload()
    await page.waitForLoadState('networkidle')
    await expect(availability(page)).toContainText('30 minutes')
  })

  test('a repository lists what is waiting on it and can be checked on demand', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await openRepoPower(page, 'media-weekly')

    const section = availability(page)
    const waiting = section.locator('.agent-row', {
      hasText: 'Catch-up on an offline repository demo',
    })
    await expect(waiting).toBeVisible()
    await expect(waiting).toContainText('last checked')
    await expect(waiting).toContainText('next check')
    await expect(waiting).toContainText('giving up')

    // The demo's repository host does answer, so the check reports back rather
    // than leaving the button spinning.
    await section.getByRole('button', { name: 'Check now' }).click()
    await expect(page.locator('.toast').first()).toBeVisible({ timeout: 15_000 })
  })

  /** An agent reconnects on its own, so there is no interval and nothing to check. */
  test('an agent marked as not always online has a window but no re-check', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/media-store-01?tab=settings&section=power')
    await page.waitForLoadState('networkidle')

    const section = availability(page)
    await expect(section).toContainText('Host is not always online')
    await expect(section).toContainText('Yes')
    await expect(section).not.toContainText('Re-check every')
    await expect(section.getByRole('button', { name: 'Check now' })).toHaveCount(0)

    await section.getByRole('button', { name: 'Edit' }).click()
    await expect(section.locator('#availability-recheck')).toHaveCount(0)
    await expect(section.locator('#availability-give-up')).toBeVisible()
  })

  /** Off is the default: an unreachable always-on host is simply a failure. */
  test('a repository that is not marked says an unreachable host is a failure', async ({
    page,
  }) => {
    await loginAsAdmin(page)
    await openRepoPower(page, 'server-daily')

    const section = availability(page)
    await expect(section).toContainText('fails like any other error')
  })
})
