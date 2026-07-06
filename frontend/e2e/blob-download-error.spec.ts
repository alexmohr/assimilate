// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('blob download error extraction', () => {
  test('failed blob download surfaces real server error message with error_id', async ({
    page,
  }) => {
    await loginAsAdmin(page)

    // Navigate to the archives page
    await page.goto('/archives')
    await page.waitForURL('/archives')

    // Select a repo from the dropdown (demo seeds repos)
    const repoSelect = page.locator('.repo-selector select')
    await expect(repoSelect).toBeVisible({ timeout: 15_000 })
    // Pick the first non-placeholder option (index 1)
    await repoSelect.selectOption({ index: 1 })

    // Wait for archives to load (the Restore button is disabled until archives appear)
    const openRestoreBtn = page.locator('.panel-actions').getByRole('button', { name: 'Restore' })
    await expect(openRestoreBtn).toBeEnabled({ timeout: 15_000 })

    // Open the Restore Wizard
    await openRestoreBtn.click()

    // --- Step 1: Select an archive ---
    const archiveSelect = page.locator('.step-content select')
    await expect(archiveSelect).toBeVisible({ timeout: 5_000 })
    // Pick the first archive option (index 1, index 0 is the placeholder)
    const options = archiveSelect.locator('option')
    const optionCount = await options.count()
    expect(optionCount).toBeGreaterThan(1)
    await archiveSelect.selectOption({ index: 1 })

    // Click Next
    await page.getByRole('button', { name: 'Next' }).click()

    // --- Step 2: Enter paths ---
    const pathsTextarea = page.locator('.step-content textarea')
    await expect(pathsTextarea).toBeVisible({ timeout: 5_000 })
    await pathsTextarea.fill('/etc/nginx/nginx.conf')

    // Click Next
    await page.getByRole('button', { name: 'Next' }).click()

    // --- Step 3: Restore method (download is default) ---
    // Verify the download radio is selected
    const downloadRadio = page.locator('input[type="radio"][value="download"]')
    await expect(downloadRadio).toBeChecked()

    // Click Next
    await page.getByRole('button', { name: 'Next' }).click()

    // --- Step 4: Confirm ---
    await expect(page.getByText('Confirm Restore')).toBeVisible({ timeout: 5_000 })

    // Mock the download endpoint to return a blob error
    const errorPayload = JSON.stringify({
      error: 'archive already being downloaded',
      error_id: 'e2e-blob-err-001',
    })
    await page.route('**/api/repos/*/archives/*/download', (route) =>
      route.fulfill({
        status: 409,
        contentType: 'application/json',
        body: errorPayload,
      }),
    )

    // Click the Restore button inside the wizard modal
    await page
      .getByRole('dialog', { name: 'Restore Files' })
      .getByRole('button', { name: 'Restore' })
      .click()

    // Verify the error message shows the decoded server error with error_id
    await expect(page.getByText(/archive already being downloaded/i)).toBeVisible({
      timeout: 10_000,
    })
    await expect(page.getByText(/e2e-blob-err-001/i)).toBeVisible({ timeout: 5_000 })
  })
})
