// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

async function gotoBackupsTab(page: Awaited<ReturnType<typeof test.info>['page']>): Promise<void> {
  await loginAsAdmin(page)
  await page.goto('/schedules/1')
  await page.waitForLoadState('networkidle')
  await page.getByRole('tab', { name: 'Backups' }).click()
  await page.waitForTimeout(1000)
}

async function clickFirstArchiveRow(
  page: Awaited<ReturnType<typeof test.info>['page']>,
): Promise<boolean> {
  // Each archive row is a button of its own.
  const archiveRow = page.getByTestId('archive-row').first()
  const rowVisible = await archiveRow.isVisible({ timeout: 5_000 }).catch(() => false)
  if (!rowVisible) return false
  await archiveRow.click()
  await page.waitForTimeout(1000)
  return true
}

test.describe('Schedule backups tab - archive browser', () => {
  test('Backups tab is visible on backup-type schedule detail', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('tab', { name: 'Backups' })).toBeVisible()
  })

  test('save bar shows only on the Settings tab', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/schedules/1')
    await page.waitForLoadState('networkidle')

    // Overview is the default tab: no form, no save bar.
    await expect(page.locator('.save-bar')).not.toBeVisible()

    await page.getByRole('tab', { name: 'Settings' }).click()
    await page.waitForTimeout(500)
    await expect(page.locator('.save-bar')).toBeVisible()

    await page.getByRole('tab', { name: 'Backups' }).click()
    await page.waitForTimeout(500)
    await expect(page.locator('.save-bar')).not.toBeVisible()
  })

  test('backups tab shows empty state or archive list', async ({ page }) => {
    await gotoBackupsTab(page)

    // Either the archives panel title is visible (with data)
    // or the empty state message is shown (no archives yet)
    const panelTitle = page.locator('.panel-title').filter({ hasText: 'Archives' })
    const emptyState = page.locator('.empty-state').filter({ hasText: 'No backup archives' })
    await expect(panelTitle.or(emptyState).first()).toBeVisible({ timeout: 10_000 })
  })

  test('backups tab renders split layout structure', async ({ page }) => {
    await gotoBackupsTab(page)

    // The backups layout should be rendered (either with data or empty)
    const backupsLayout = page.locator('.archive-browser-layout')
    const tabContent = page
      .locator('.tab-content')
      .filter({ hasText: /Archives|No backup archives/ })
    await expect(backupsLayout.or(tabContent).first()).toBeVisible({ timeout: 10_000 })
  })

  test('file browser structure renders when archive is selected', async ({ page }) => {
    await gotoBackupsTab(page)

    const hasArchive = await clickFirstArchiveRow(page)
    if (!hasArchive) {
      test.skip()
      return
    }

    // The file browser should show with breadcrumb
    await expect(page.locator('.path-crumbs')).toBeVisible()
    await expect(page.locator('.path-crumbs .crumb-root')).toHaveText('/')
  })

  test('download buttons present in file browser when archive selected', async ({ page }) => {
    await gotoBackupsTab(page)

    if (!(await clickFirstArchiveRow(page))) {
      test.skip()
      return
    }

    // Downloading the whole archive is in the open archive's menu.
    const browser = page.locator('.archive-file-browser')
    await browser.getByRole('button', { name: 'Archive actions' }).click()
    await expect(browser.getByRole('menuitem', { name: 'Download' })).toBeVisible()
  })

  // The schedule tab used to render its own four-column table: no grouping, no
  // search, no delete. It now renders the same ArchiveExplorer as the
  // repository and Archives screens.
  test('backups tab renders the shared archive selector controls', async ({ page }) => {
    await gotoBackupsTab(page)

    if (
      !(await page
        .getByTestId('archive-row')
        .first()
        .isVisible()
        .catch(() => false))
    ) {
      test.skip()
      return
    }

    const list = page.locator('.archives-panel')
    await expect(list.locator('.archive-controls input')).toBeVisible()
    // Grouping and sort are in the list's menu.
    await list.getByRole('button', { name: 'List options' }).click()
    await expect(list.getByRole('menuitemradio', { name: 'Date newest first' })).toBeVisible()
    // Schedule 1 backs up one machine, so its archives are listed by day and
    // host grouping is not on offer: one group holding everything says nothing.
    // repo-archives-group-by-host.spec.ts covers the host groups and their
    // agent links on a repository several hosts share.
    await expect(list.getByRole('menuitemradio', { name: 'By day' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await expect(list.getByRole('menuitemradio', { name: 'By host' })).toHaveCount(0)
    await list.getByRole('button', { name: 'List options' }).click()
    await expect(list.locator('.row-group-label').first()).toBeVisible()
    await expect(list.locator('.group-hostname')).toHaveCount(0)
  })

  test('an admin can reach archive deletion from the backups tab', async ({ page }) => {
    await gotoBackupsTab(page)

    if (!(await clickFirstArchiveRow(page))) {
      test.skip()
      return
    }

    // Open the confirmation and cancel it: the point is that the control is
    // there and wired up, not that this run deletes a seeded archive. It is in
    // the open archive's menu.
    const browser = page.locator('.archive-file-browser')
    await browser.getByRole('button', { name: 'Archive actions' }).click()
    await browser.getByRole('menuitem', { name: 'Delete archive...' }).click()
    await expect(page.locator('.archive-delete-message')).toBeVisible()
    await page.locator('.modal-footer').getByRole('button', { name: 'Cancel' }).click()
    await expect(page.locator('.archive-delete-message')).not.toBeVisible()
  })

  test('file rows keep their actions inside the file browser panel', async ({ page }) => {
    // Beside the 380px archive list, a 1280px desktop leaves the file pane
    // narrow: the old file table scrolled sideways and each row's restore
    // button sat past the panel's edge. 1100 is the narrow-desktop band, and
    // 375 is a phone, where the open archive replaces the list. Restoring is
    // the selection bar's, so one entry is ticked to bring it up.
    await gotoBackupsTab(page)
    if (!(await clickFirstArchiveRow(page))) {
      test.skip()
      return
    }
    const firstRow = page.locator('.archive-file-browser').getByTestId('browser-entry').first()
    await expect(firstRow).toBeVisible({ timeout: 30_000 })
    await firstRow.getByTestId('pick-entry').check()
    await expect(page.getByTestId('browser-selection')).toBeVisible()

    for (const width of [1280, 1100, 375]) {
      await page.setViewportSize({ width, height: 800 })
      await expect(firstRow).toBeVisible({ timeout: 10_000 })

      const layout = await page.locator('.browser-panel').evaluate((panel) => {
        const box = panel.getBoundingClientRect()
        const rows = panel.querySelector('.archive-rows')
        return {
          overflow: rows ? rows.scrollWidth - rows.clientWidth : 0,
          buttons: [
            ...panel.querySelectorAll(
              '[data-testid="browser-entry"] button, .selection-bar button, .archive-head button',
            ),
          ].map((button) => {
            const b = button.getBoundingClientRect()
            return {
              title: (button.getAttribute('aria-label') ?? button.textContent ?? '').trim(),
              left: b.left - box.left,
              right: box.right - b.right,
            }
          }),
        }
      })

      expect(layout.overflow, `file rows scroll sideways at ${width}px`).toBeLessThanOrEqual(1)
      expect(layout.buttons.some((b) => b.title.startsWith('Restore'))).toBe(true)
      for (const button of layout.buttons) {
        expect(button.left, `"${button.title}" is clipped at ${width}px`).toBeGreaterThanOrEqual(-1)
        expect(button.right, `"${button.title}" is clipped at ${width}px`).toBeGreaterThanOrEqual(
          -1,
        )
      }
    }
  })

  test('breadcrumb navigation updates when navigating directories', async ({ page }) => {
    await gotoBackupsTab(page)

    if (!(await clickFirstArchiveRow(page))) {
      test.skip()
      return
    }

    // Breadcrumb should show root
    const breadcrumb = page.locator('.path-crumbs')
    await expect(breadcrumb).toBeVisible()

    // Try navigating into a directory if one exists
    const dirEntry = page.locator('.archive-file-browser .browser-open').first()
    const dirVisible = await dirEntry.isVisible({ timeout: 5_000 }).catch(() => false)
    if (!dirVisible) {
      test.skip()
      return
    }

    await dirEntry.click()
    await page.waitForTimeout(1000)
    await expect(breadcrumb).toBeVisible()
  })
})
