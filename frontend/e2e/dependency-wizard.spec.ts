// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

/**
 * A new dependency is made on a page of its own, a wizard of four steps
 * (host, check, machine, review), which replaced the New dependency dialog.
 * The check is tested against 127.0.0.1 on the SSH port: the demo container's
 * own sshd, which the seeded files-01 dependency answers on too (see
 * `.devcontainer/demo/seed-demo.sh`). What the test creates it removes.
 */
test.describe('New dependency wizard', () => {
  test('walks the steps, tests the check and creates the dependency', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/dependencies')
    await page.getByRole('button', { name: 'New', exact: true }).click()
    await expect(page).toHaveURL(/\/dependencies\/new$/)
    await expect(page.getByRole('navigation', { name: 'Dependency steps' })).toBeVisible()

    // Scoped to the footer: the rail's steps are buttons too.
    const foot = page.locator('.wizard-foot')
    const next = foot.getByRole('button', { name: 'Continue' })
    const create = foot.getByRole('button', { name: 'Create dependency', exact: true })

    // Host: a step says what it still needs, and will not move on without it.
    await expect(create).toHaveCount(0)
    await expect(next).toBeDisabled()
    await expect(foot).toContainText('Needs a name and an address to continue.')
    const name = `e2e-wizard-dep-${Date.now()}`
    await page.locator('#dependency-name').fill(name)
    await page.locator('#dependency-address').fill('127.0.0.1')
    await page.locator('#dependency-description').fill('Made by the wizard e2e test')
    await expect(next).toBeEnabled()
    await next.click()

    // Check: which port says it is up, and a test of it before it exists.
    await expect(page.locator('.wizard-step--current')).toContainText('Check')
    await page
      .getByRole('radiogroup', { name: 'Check' })
      .getByRole('radio', { name: 'SSH' })
      .click()
    await page.getByRole('button', { name: 'Test connection' }).click()
    await expect(page.locator('.dependency-test .form-success')).toHaveText(
      '127.0.0.1 answered on port 22',
      { timeout: 30_000 },
    )
    await next.click()

    // Machine: not a repository host, the default.
    await expect(page.locator('.wizard-step--current')).toContainText('Machine')
    await expect(page.locator('#dependency-repo-host option:checked')).toHaveText(
      'Not a repository host',
    )
    await next.click()

    // Review: what will be created, each block linking back to its step.
    await expect(page.locator('.wizard-step--current')).toContainText('Review')
    const review = page.locator('.review-grid')
    await expect(review).toContainText(name)
    await expect(review).toContainText('127.0.0.1')
    await expect(review).toContainText('SSH · port 22')
    await expect(review).toContainText('127.0.0.1 answered on port 22')
    await expect(review).toContainText('Not a repository host')

    const [created] = await Promise.all([
      page.waitForResponse(
        (resp) =>
          resp.url().endsWith('/api/dependency-hosts') && resp.request().method() === 'POST',
      ),
      create.click(),
    ])
    expect(created.ok()).toBe(true)
    const { id } = (await created.json()) as { id: number }
    try {
      // A created dependency opens on its own page, and the list holds it.
      await expect(page).toHaveURL(new RegExp(`/dependency-hosts/${id}$`))
      await expect(page.locator('.detail-name')).toHaveText(name)

      await page.goto('/dependencies')
      await expect(page.locator('[data-testid="dependency-row"]', { hasText: name })).toBeVisible()
    } finally {
      const removed = await page.request.delete(`/api/dependency-hosts/${id}`)
      expect(removed.ok()).toBe(true)
    }
  })

  test('Cancel leaves without creating anything', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/dependencies/new')
    await page.locator('#dependency-name').fill('e2e-never-created')
    await page.locator('.wizard-foot').getByRole('button', { name: 'Cancel' }).click()
    await expect(page).toHaveURL(/\/dependencies$/)
    await expect(
      page.locator('[data-testid="dependency-row"]', { hasText: 'e2e-never-created' }),
    ).toHaveCount(0)
  })
})
