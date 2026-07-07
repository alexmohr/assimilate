// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Dashboard widgets', () => {
  test('summary stat widgets are visible', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await expect(
      page.getByText('Online Agents').or(page.getByText('ONLINE AGENTS')).first(),
    ).toBeVisible()
    await expect(page.getByText('Overdue').or(page.getByText('OVERDUE')).first()).toBeVisible()
    await expect(
      page.getByText('Last Backup').or(page.getByText('LAST BACKUP')).first(),
    ).toBeVisible()
    await expect(
      page.getByText('Total Storage').or(page.getByText('TOTAL STORAGE')).first(),
    ).toBeVisible()
  })

  test('Online Agents stat uses correct field names from API', async ({ page }) => {
    await loginAsAdmin(page)
    const resp = await page.request.get('/api/stats/summary')
    expect(resp.ok()).toBe(true)
    const body = (await resp.json()) as Record<string, unknown>

    // These fields drove a 0/0 display bug - verify the API uses the correct names.
    expect(typeof body['online_agents']).toBe('number')
    expect(typeof body['total_agents']).toBe('number')
    expect(body['total_agents']).toBeGreaterThan(0)
  })

  test('dashboard shows recent activity section', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const activityHeading = page.getByRole('heading', { name: 'Recent Activity' })
    await expect(activityHeading).toBeVisible()
    await expect(activityHeading.locator('..').getByText('db-server-01').first()).toBeVisible()
  })

  test('dashboard shows backup stats section', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Backup Stats' })).toBeVisible()
  })
})

test.describe('Navigation sidebar', () => {
  test('Agents link navigates to /agents', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.getByRole('link', { name: 'Agents' }).click()
    await expect(page).toHaveURL(/\/agents/)
  })

  test('Repos link navigates to /repos', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.getByRole('link', { name: 'Repos' }).click()
    await expect(page).toHaveURL(/\/repos/)
  })

  test('Schedules link navigates to /schedules', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.getByRole('link', { name: 'Schedules', exact: true }).click()
    await expect(page).toHaveURL(/\/schedules/)
  })

  test('Activity link navigates to /activity', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.getByRole('link', { name: 'Activity' }).click()
    await expect(page).toHaveURL(/\/activity/)
  })

  test('Dashboard link returns to root', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents')
    await page.getByRole('link', { name: 'Dashboard' }).click()
    await expect(page).toHaveURL('/')
  })
})
