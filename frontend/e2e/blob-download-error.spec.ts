// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, openSeededWebArchive, test } from './fixtures'

test.describe('blob download error extraction', () => {
  test('failed blob download surfaces real server error message with error_id', async ({
    page,
  }) => {
    await openSeededWebArchive(page)

    // A folder downloads as one tar the server streams - the blob request
    // whose failure body is itself a Blob.
    await page.getByRole('checkbox', { name: 'Select tmp' }).check()

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

    await page
      .getByTestId('browser-selection')
      .getByRole('button', { name: 'Download', exact: true })
      .click()

    // The decoded server error, with its error_id, not axios's bare status line.
    await expect(page.getByText(/archive already being downloaded/i)).toBeVisible({
      timeout: 10_000,
    })
    await expect(page.getByText(/e2e-blob-err-001/i)).toBeVisible({ timeout: 5_000 })
  })
})
