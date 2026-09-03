// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('Dashboard widgets', () => {
  test('summary stat widgets are visible', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await expect(
      page.getByText('Online agents').or(page.getByText('ONLINE AGENTS')).first(),
    ).toBeVisible()
    await expect(page.getByText('Overdue').or(page.getByText('OVERDUE')).first()).toBeVisible()
    await expect(
      page.getByText('Last backup').or(page.getByText('LAST BACKUP')).first(),
    ).toBeVisible()
    await expect(
      page.getByText('Total storage').or(page.getByText('TOTAL STORAGE')).first(),
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

  test('hides NeedsAttention panel when no findings exist', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    // The demo environment has findings, so dismiss them all via the API to
    // verify the panel hides.
    const resp = await page.request.get('/api/stats/dashboard-overview')
    const body = (await resp.json()) as { findings: Array<{ id: string }> }
    const findingIds = body.findings.map((f) => f.id)

    try {
      for (const id of findingIds) {
        await page.request.post(`/api/stats/findings/${id}/dismiss`)
      }

      await page.goto('/')
      await page.waitForLoadState('networkidle')

      await expect(page.locator('#needs-attention')).toHaveCount(0)
    } finally {
      // Restore dismissed findings so they remain visible in the demo environment.
      for (const id of findingIds) {
        await page.request.delete(`/api/stats/findings/${id}/dismiss`)
      }
    }
  })

  test('bounds a Needs Attention row no matter how long the reason is', async ({ page }) => {
    await loginAsAdmin(page)

    const resp = await page.request.get('/api/stats/dashboard-overview')
    expect(resp.ok()).toBe(true)
    const overview = (await resp.json()) as Record<string, unknown> & {
      findings: Array<{ reason: string }>
    }
    expect(overview.findings.length).toBeGreaterThan(0)

    // The server caps an agent-supplied message (borg stderr, import errors) at
    // 200 characters plus the ellipsis, so the payload itself stays bounded.
    for (const finding of overview.findings) {
      expect([...finding.reason].length).toBeLessThanOrEqual(203)
    }

    // Which finding kind the demo currently surfaces depends on its live backup
    // state, and only some kinds carry an agent message at all - so serve the
    // panel an oversized reason directly to prove the row clamps it regardless.
    const longReason = `borg: ${'stderr overflow '.repeat(200)}`
    overview.findings[0].reason = longReason
    await page.route('**/api/stats/dashboard-overview', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(overview),
      }),
    )

    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const reasons = page.locator('#needs-attention .finding-reason')
    await expect(reasons.first()).toBeVisible()
    await expect(reasons.first()).toHaveAttribute('title', longReason)

    // Every rendered reason stays within a two-line ceiling.
    const count = await reasons.count()
    for (let i = 0; i < count; i++) {
      const metrics = await reasons.nth(i).evaluate((el) => {
        const style = getComputedStyle(el)
        const lineHeight = parseFloat(style.lineHeight)
        return {
          height: el.getBoundingClientRect().height,
          lineHeight: Number.isNaN(lineHeight) ? parseFloat(style.fontSize) * 1.5 : lineHeight,
        }
      })
      expect(metrics.height).toBeLessThanOrEqual(metrics.lineHeight * 2 + 2)
    }
  })

  test('dashboard shows recent activity section', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    const activityHeading = page.getByRole('heading', { name: 'Recent activity' })
    await expect(activityHeading).toBeVisible()
    await expect(activityHeading.locator('..').getByText('db-server-01').first()).toBeVisible()
  })

  test('dashboard shows backup stats section', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await expect(page.getByRole('heading', { name: 'Backup stats' })).toBeVisible()
  })

  // The reset the Failed tile exists for: acknowledging the runs in view has
  // to take them out of the count, out of the button's own reason to exist,
  // and into the reviewed line - without deleting anything.
  test('marks the failures in view as reviewed and clears the failed tile', async ({ page }) => {
    await loginAsAdmin(page)

    // Everything the reset is about to acknowledge, so the demo environment
    // can be put back exactly as it was for the tests that follow.
    const outstanding = await page.request.get(
      '/api/stats/activity?days=30&acknowledged=unacknowledged',
    )
    expect(outstanding.ok()).toBe(true)
    const rows = (await outstanding.json()) as Array<{ id: number; status: string }>
    const reviewedIds = rows
      .filter((row) => row.status === 'failed' || row.status === 'warning')
      .map((row) => row.id)
    expect(reviewedIds.length).toBeGreaterThan(0)

    try {
      await page.goto('/')
      await page.waitForLoadState('networkidle')

      const panel = page
        .locator('section.panel')
        .filter({ has: page.getByRole('heading', { name: 'Backup stats' }) })
      const failedTile = panel.locator('.mini-stat').nth(2)
      await expect(failedTile.locator('.stat-value')).not.toHaveText('0')

      await panel.locator('.stats-actions button').click()
      await page.locator('.modal-footer').getByRole('button', { name: 'Mark reviewed' }).click()

      await expect(failedTile.locator('.stat-value')).toHaveText('0')
      await expect(failedTile.locator('.stat-sub')).toContainText('reviewed')
      // Nothing left in view to review, so the button retires itself.
      await expect(panel.locator('.stats-actions')).toHaveCount(0)

      // Reviewed, not deleted: the runs are still in the feed.
      const kept = await page.request.get('/api/stats/activity?days=30&acknowledged=acknowledged')
      const keptIds = ((await kept.json()) as Array<{ id: number }>).map((row) => row.id)
      for (const id of reviewedIds) {
        expect(keptIds).toContain(id)
      }
    } finally {
      for (const id of reviewedIds) {
        await page.request.delete(`/api/stats/activity/${id}/acknowledge`)
      }
    }
  })

  test('backup calendar renders all seven day columns without clipping', async ({ page }) => {
    // The calendar used to share a three-column row, which left it narrower
    // than its own month grid: the grid overflowed and the panel's clip took
    // Saturday off the dashboard entirely.
    await loginAsAdmin(page)
    for (const width of [1280, 1600]) {
      await page.setViewportSize({ width, height: 900 })
      await page.goto('/')
      await page.waitForLoadState('networkidle')

      const grid = page.locator('.cal-grid')
      await expect(grid).toBeVisible()

      // `offsetLeft` is relative to the offset parent, not the grid, so the day
      // cells are measured against the grid's own box instead.
      const layout = await grid.evaluate((el) => {
        const gridRight = el.getBoundingClientRect().right
        return {
          overflow: el.scrollWidth - el.clientWidth,
          headers: [...el.querySelectorAll('.cal-header-cell')].map((cell) => ({
            day: cell.textContent?.trim() ?? '',
            overhang: cell.getBoundingClientRect().right - gridRight,
          })),
        }
      })

      expect(
        layout.overflow,
        `calendar grid overflows its panel at ${width}px`,
      ).toBeLessThanOrEqual(1)
      expect(layout.headers.map((h) => h.day)).toEqual([
        'Sun',
        'Mon',
        'Tue',
        'Wed',
        'Thu',
        'Fri',
        'Sat',
      ])
      for (const header of layout.headers) {
        expect(header.overhang, `${header.day} is clipped at ${width}px`).toBeLessThanOrEqual(1)
      }
    }
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
