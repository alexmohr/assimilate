// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expandAllArchiveGroups, expect, loginAsAdmin, test } from './fixtures'

// media-store-01's media-weekly archives hold three symbolic links under
// mnt/media (see start-agent.sh): `latest` to the photos folder beside it,
// `photos/cover.jpg` to a photo in that folder, and `nas` to a path the
// archive does not hold.
test.describe('Symbolic links in the archive browser', () => {
  test('a link shows where it points, and opens what the archive holds', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/repos')
    await page.getByText('media-weekly', { exact: true }).click()
    await page.waitForURL(/\/repos\/\d+/, { waitUntil: 'commit' })
    await page.getByRole('tab', { name: 'Archives', exact: true }).click()
    await page.waitForLoadState('networkidle')
    await expandAllArchiveGroups(page)

    // The oldest of media-store-01's own archives: archive-browsing.spec.ts
    // deletes the newest archive here, and media-weekly also holds
    // web-server-01's dual-target copies, which have no media folder.
    await page
      .locator('.archive-row')
      .filter({ hasText: /media-store-01-backup-/ })
      .last()
      .locator('.archive-name')
      .click()

    const browserPanel = page.locator('.browser-panel').last()
    const crumbs = browserPanel.locator('.path-crumbs .crumb')
    const open = async (name: RegExp | string): Promise<void> => {
      const entry = browserPanel.locator('.name-text').filter({
        hasText: typeof name === 'string' ? new RegExp(`^${name}$`) : name,
      })
      await expect(entry).toBeVisible({ timeout: 120_000 })
      const depth = await crumbs.count()
      await entry.click()
      await expect(crumbs).toHaveCount(depth + 1)
    }
    // The demo backs up a mktemp -d directory under tmp.
    await open('tmp')
    await open(/^tmp\./)
    await open('mnt')
    await open('media')

    const row = (name: string) =>
      browserPanel.locator('tbody tr').filter({
        has: page.locator('.name-text', { hasText: new RegExp(`^${name}$`) }),
      })

    const nas = row('nas')
    await expect(nas.getByTestId('entry-link')).toContainText('/srv/nas/media')
    await expect(nas.getByTestId('entry-link')).toContainText('not in archive')
    await expect(nas.locator('button[data-testid="link-target"]')).toHaveCount(0)

    const latest = row('latest')
    await expect(latest.getByTestId('entry-link')).toContainText('Link')
    await expect(latest.getByRole('button', { name: 'Open link target photos' })).toBeVisible()
    await expect(latest.locator('.td-name')).toHaveAttribute(
      'title',
      /Symbolic link: .*latest → photos/,
    )

    await latest.click()
    await expect(crumbs.last()).toHaveText('photos')

    const cover = row('cover.jpg')
    await expect(cover.getByTestId('link-target')).toHaveText(/^img_\d+\.jpg$/)
    await cover.getByTestId('link-target').click()
    // The folder holding the target is the one already open.
    await expect(crumbs.last()).toHaveText('photos')
  })
})
