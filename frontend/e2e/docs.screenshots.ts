// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Captures every screenshot under docs/assets/screenshots from the seeded demo
// environment, so they can be refreshed in one run instead of by hand:
//
//   .devcontainer/start.sh --demo            # in one terminal
//   cd frontend && npm run screenshots       # once the seed has finished
//
// Every shot uses the same 1280x800 viewport at 2x density in the light
// theme (see playwright.screenshots.config.ts). Pages and tabs are reached
// through their URLs; ids are looked up by name through the API so the
// script does not depend on the order the seed creates things in.

import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { expect, test, type Locator, type Page } from '@playwright/test'
import { loginAsAdmin, seededIdByName } from './fixtures'

const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), '../../docs/assets/screenshots')
/** Images used only by the project website (website/), not by the docs. */
const WEBSITE_DIR = join(dirname(fileURLToPath(import.meta.url)), '../../website/assets/shots')

interface Named {
  id: number
  name?: string | null
}

/** Waits for loading indicators to clear and late layout to settle. */
async function settle(page: Page): Promise<void> {
  await page.waitForLoadState('networkidle')
  await expect(page.locator('.spinner-wrapper[role="status"]')).toHaveCount(0, {
    timeout: 15_000,
  })
  // Charts animate in and fonts swap late; give both a moment.
  await page.waitForTimeout(800)
}

async function shot(page: Page, name: string, opts: { fullPage?: boolean } = {}): Promise<void> {
  await settle(page)
  await page.screenshot({
    path: join(OUT_DIR, `${name}.png`),
    fullPage: opts.fullPage ?? false,
    animations: 'disabled',
    caret: 'hide',
  })
}

async function shotOf(locator: Locator, name: string): Promise<void> {
  await shotOfTo(locator, OUT_DIR, name)
}

async function shotOfTo(locator: Locator, dir: string, name: string): Promise<void> {
  await settle(locator.page())
  await locator.scrollIntoViewIfNeeded()
  await locator.screenshot({ path: join(dir, `${name}.png`), animations: 'disabled' })
}

/** Waits until an opened archive's file tree has been indexed and listed. */
async function waitForIndex(page: Page): Promise<void> {
  // Wait for the browser's finished state (the file table, or an empty
  // directory) rather than for the indexing note to go away: the note only
  // appears a moment after the archive opens, so its absence proves nothing.
  // An indexing error never reaches either state and fails the wait.
  const listed = page.locator('table.browser-table').or(page.getByText('Empty directory.'))
  await expect(listed.first()).toBeVisible({ timeout: 120_000 })
  await settle(page)
}

interface AgentStatus {
  hostname: string
  is_connected: boolean
}

/** The demo agents that run as real containers; the rest are offline by design. */
const LIVE_AGENTS = ['web-server-01', 'db-server-01', 'media-store-01']

/**
 * The seed restarts the server once, and agents wait a minute before they
 * reconnect - screenshots taken in that gap show every host offline.
 */
async function waitForLiveAgents(page: Page): Promise<void> {
  await expect
    .poll(
      async () => {
        const res = await page.request.get('/api/agents')
        const agents = (await res.json()) as AgentStatus[]
        return LIVE_AGENTS.every((name) =>
          agents.some((agent) => agent.hostname === name && agent.is_connected),
        )
      },
      { timeout: 120_000, intervals: [2_000] },
    )
    .toBe(true)
}

async function visit(page: Page, path: string): Promise<void> {
  await page.goto(path)
  await settle(page)
}

test.describe.configure({ mode: 'serial' })

test('login', async ({ page }) => {
  await visit(page, '/login')
  await shot(page, 'login')
})

test.describe('signed in', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsAdmin(page)
    await waitForLiveAgents(page)
  })

  test('dashboard', async ({ page }) => {
    await visit(page, '/')
    await shot(page, 'dashboard-hero')
    await shot(page, 'dashboard-full', { fullPage: true })
    await shotOf(
      page
        .locator('section, .panel, .card')
        .filter({ has: page.getByText('Backup stats', { exact: true }) })
        .last(),
      'dashboard-backup-stats',
    )
  })

  test('agents', async ({ page }) => {
    await visit(page, '/agents')
    await shot(page, 'hosts')

    await visit(page, '/agents/web-server-01')
    await shot(page, 'host-detail')

    await visit(page, '/agents?tab=dependencies')
    await shot(page, 'dependency-hosts')

    await visit(page, '/agents/edge-proxy')
    await shot(page, 'host-domain-picker')

    // db-server-01 carries the seeded default file change patterns.
    await visit(page, '/agents/db-server-01?tab=settings&section=defaults')
    await shot(page, 'host-file-change-patterns')

    await visit(page, '/agents/media-store-01?tab=settings&section=power')
    await shot(page, 'agent-power')

    // The seeded power-managed run (see seed-demo.sh): open each run's
    // detail until one shows the wake packet sent to media-store-01.
    await visit(page, '/agents/media-store-01?tab=logs')
    const details = page.getByRole('button', { name: 'Show detail' })
    const count = await details.count()
    for (let i = 0; i < count; i++) {
      await details.nth(i).click()
      await settle(page)
      if (await page.getByText('Sent Wake-on-LAN packet to 3C:97:0E:2B:9A:44').first().isVisible())
        break
      await page.getByRole('button', { name: 'Hide detail' }).first().click()
    }
    // Fail rather than capture a page with no run expanded.
    await expect(
      page.getByText('Sent Wake-on-LAN packet to 3C:97:0E:2B:9A:44').first(),
      'the seeded power-managed run is in the log',
    ).toBeVisible()
    await page.getByText('Power management', { exact: true }).first().scrollIntoViewIfNeeded()
    await shot(page, 'run-timeline')

    await visit(page, '/agents/db-server-01?tab=settings&section=vms')
    await shot(page, 'agent-vms')

    await page
      .getByRole('button', { name: /^restore$/i })
      .first()
      .click()
    await shot(page, 'vm-restore')
  })

  test('repositories', async ({ page }) => {
    await visit(page, '/repos')
    await shot(page, 'repositories')
    // The repositories are grouped by host by default; this one is just the
    // first host with a server quota, so its storage pool bar is the focus.
    const pool = page.locator('.host-group').filter({ has: page.locator('.pool-track') })
    await shotOf(pool.first(), 'repositories-host-quota')

    const hourly = await seededIdByName(page, '/api/repos', 'database-hourly')
    await visit(page, `/repos/${hourly}`)
    await shot(page, 'repo-detail')

    await visit(page, `/repos/${hourly}?tab=archives`)
    await shot(page, 'archives')

    await page.locator('.archive-row-body').first().click()
    await waitForIndex(page)
    await shot(page, 'archive-browse')

    const hostsRes = await page.request.get('/api/repo-hosts')
    const hosts = (await hostsRes.json()) as Named[]
    await visit(page, `/repo-hosts/${hosts[0]?.id ?? 1}`)
    await shot(page, 'repo-host-detail')
  })

  test('schedules', async ({ page }) => {
    await visit(page, '/schedules')
    await shot(page, 'schedules')

    await visit(page, '/schedules/new')
    await shot(page, 'schedule-wizard')

    const dual = await seededIdByName(page, '/api/schedules', 'Web server dual-target')
    await visit(page, `/schedules/${dual}`)
    await shot(page, 'schedule-detail')

    await visit(page, `/schedules/${dual}?tab=backups`)
    await page.locator('.archive-row-body').first().click()
    await waitForIndex(page)
    await shot(page, 'schedule-backups')

    await visit(page, `/schedules/${dual}?tab=settings&section=power`)
    await shot(page, 'schedule-power')

    const share = await seededIdByName(page, '/api/schedules', 'Media share nightly')
    await visit(page, `/schedules/${share}?tab=settings&section=dependencies`)
    await shot(page, 'schedule-dependencies')
  })

  test('notifications', async ({ page }) => {
    await visit(page, '/notifications')
    await shot(page, 'notifications')

    // Seeded channels, in display order: Ops Webhook, then Admin Email.
    const edit = page.getByRole('button', { name: 'Edit', exact: true })
    await edit.nth(1).click()
    await shot(page, 'notification-edit-email')
    await page.keyboard.press('Escape')

    await edit.nth(0).click()
    await shot(page, 'notification-edit-webhook')
    await page.keyboard.press('Escape')

    // Each channel card has two pencil buttons: events, then scope.
    const pencils = page.getByRole('button', { name: '\u270e' })
    await pencils.nth(0).click()
    await shot(page, 'notifications-events-modal')
    await page.keyboard.press('Escape')

    await pencils.nth(1).click()
    await shot(page, 'notifications-scope-modal')
    await page.keyboard.press('Escape')

    await page.getByRole('button', { name: 'New', exact: true }).click()
    await shot(page, 'notifications-wizard-step1')
    await page.keyboard.press('Escape')

    await page.getByRole('tab', { name: 'History' }).click()
    await settle(page)
    await page.getByText('Backup Failed').first().click()
    await shot(page, 'notifications-history-expanded')
  })

  test('settings pages', async ({ page }) => {
    for (const [path, name] of [
      ['/activity', 'activity'],
      ['/audit-log', 'audit-log'],
      ['/excludes', 'excludes'],
      ['/tunnels', 'tunnels'],
      ['/users', 'users'],
      ['/admin/groups', 'groups'],
      ['/admin/roles', 'roles'],
      ['/tokens', 'tokens'],
      ['/profile', 'profile'],
      ['/system', 'system'],
    ] as const) {
      await visit(page, path)
      await shot(page, name)
    }

    await visit(page, '/system')
    await page.getByText('Database Storage').first().scrollIntoViewIfNeeded()
    await shot(page, 'system-db')
  })
})

test.describe('website', () => {
  // The archive browser in the dark theme, cropped to the two panes, for the
  // website's restore section: a search in the archive list, and a
  // web-server-01 archive opened at the level that holds its etc/ and var/.
  test('restore crop', async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: 1280, height: 900 },
      deviceScaleFactor: 2,
      colorScheme: 'dark',
    })
    const page = await context.newPage()
    await loginAsAdmin(page)
    await waitForLiveAgents(page)
    const daily = await seededIdByName(page, '/api/repos', 'server-daily')
    await visit(page, `/repos/${daily}?tab=archives`)
    await page.getByText('Flat', { exact: true }).first().click()
    await page.getByPlaceholder('Search name or host').fill('web-server-01-backup')
    await settle(page)
    await page.locator('.archive-row-body:visible').first().click()
    await waitForIndex(page)

    // The demo agent archives a temporary directory, so the interesting
    // folders sit a couple of levels down: open single folders until a level
    // lists more than one entry.
    const rows = page.locator('.archive-browser-layout table tbody tr')
    for (let depth = 0; depth < 5; depth++) {
      const names = (await rows.allInnerTexts())
        .map((row) => row.split('\t')[0]?.trim() ?? '')
        .filter((name) => name.length > 0 && name !== '.' && name !== '..')
      if (names.length !== 1) break
      const only = names[0] as string
      await rows.filter({ hasText: only }).first().getByText(only, { exact: true }).click()
      await settle(page)
    }
    await shotOfTo(page.locator('.archive-browser-layout').first(), WEBSITE_DIR, 'restore-dark')
    await context.close()
  })

  // The seeded run that was skipped because the nas-media dependency did not
  // answer, with its timeline open, for the website's dependency-hosts section.
  test('dependency skip', async ({ page }) => {
    await loginAsAdmin(page)
    await waitForLiveAgents(page)
    await visit(page, '/agents/media-store-01?tab=logs')
    // Exactly one row: the skipped run of the schedule that needs nas-media.
    const skipped = page
      .locator('.agent-row')
      .filter({ has: page.locator('.badge', { hasText: /^skipped$/i }) })
      .filter({ hasText: 'Media share nightly' })
    await expect(skipped).toHaveCount(1)
    await skipped.getByRole('button', { name: 'Show detail' }).click()
    await settle(page)
    // The row's detail renders as its next sibling, not inside the row.
    const detail = skipped.locator('xpath=following-sibling::*[1]')
    await detail
      .getByText(/nas-media/)
      .first()
      .scrollIntoViewIfNeeded()
    await page.screenshot({
      path: join(WEBSITE_DIR, 'dependency-skip.png'),
      animations: 'disabled',
      caret: 'hide',
    })
  })

  // The responsive UI on a phone and a tablet, for the website's mobile section.
  for (const device of [
    {
      name: 'phone',
      viewport: { width: 390, height: 844 },
      scale: 3,
      path: '/',
      view: 'dashboard',
    },
    {
      name: 'tablet',
      viewport: { width: 820, height: 1180 },
      scale: 2,
      path: '/schedules',
      view: 'schedules',
    },
  ]) {
    test(`${device.name} views`, async ({ browser }) => {
      const context = await browser.newContext({
        viewport: device.viewport,
        deviceScaleFactor: device.scale,
        colorScheme: 'light',
        isMobile: true,
        hasTouch: true,
      })
      const page = await context.newPage()
      await loginAsAdmin(page)
      await waitForLiveAgents(page)
      await visit(page, device.path)
      await page.screenshot({
        path: join(WEBSITE_DIR, `${device.name}-${device.view}.png`),
        animations: 'disabled',
        caret: 'hide',
      })
      await context.close()
    })
  }
})
