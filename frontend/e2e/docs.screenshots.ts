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

import { fileURLToPath, pathToFileURL } from 'node:url'
import { dirname, join } from 'node:path'
import { expect, test, type Locator, type Page } from '@playwright/test'
import {
  expandAllArchiveGroups,
  loginAsAdmin,
  seededIdByName,
  seededNginxComparison,
} from './fixtures'
import type { SystemModeResponse } from '../src/types/generated/SystemModeResponse'

const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), '../../docs/assets/screenshots')
/** The desktop app's bundled startup page, shown until its server is up. */
const DESKTOP_SPLASH = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../desktop/splash/index.html',
)
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

/**
 * Opens a seeded dependency's Used by section and waits for its rows. files-01
 * is needed by media-store-01's backup defaults, by every backup into the
 * "localhost" repository host, and so by schedules on more than one agent.
 */
async function visitDependencyUsedBy(page: Page, name: string): Promise<void> {
  const id = await seededIdByName(page, '/api/dependency-hosts', name)
  await visit(page, `/dependency-hosts/${id}?section=used-by`)
  await expect(page.locator('a.agent-row').first()).toBeVisible()
  await settle(page)
}

test.describe.configure({ mode: 'serial' })

test('login', async ({ page }) => {
  await visit(page, '/login')
  await shot(page, 'login')
})

// The desktop app's startup page is a static file bundled into the app, so it
// is opened straight from desktop/splash rather than through the demo. It
// carries no Content Security Policy of its own (the app applies one), so its
// inline script runs from a file URL too.
test('desktop startup page', async ({ page }) => {
  await page.goto(pathToFileURL(DESKTOP_SPLASH).href)
  await expect(page.getByText('Starting the backup service on this computer')).toBeVisible()
  await shot(page, 'desktop-startup')

  // The same hook the app calls when startup fails (report_startup_error in
  // desktop/src/app.rs), with one of its real messages and the Linux logs folder.
  await page.evaluate(() => {
    const hooks = window as unknown as {
      showStartupError: (message: string, logDir: string) => void
    }
    hooks.showStartupError(
      'the local server did not become healthy within 120s',
      '/home/user/.local/share/assimilate/logs',
    )
  })
  await expect(page.getByText('Assimilate could not start.')).toBeVisible()
  await shot(page, 'desktop-startup-error')
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

  // The demo server runs in server mode; the desktop app's answer is faked so
  // the sidebar shows what the desktop window leaves out (see
  // e2e/system-mode.spec.ts), with the Settings group open.
  test('desktop mode', async ({ page }) => {
    const desktop: SystemModeResponse = { mode: 'desktop' }
    await page.route('**/api/system/mode', (route) => route.fulfill({ json: desktop }))
    await visit(page, '/')
    const nav = page.locator('.nav')
    await nav.getByRole('button', { name: 'Settings' }).click()
    await expect(nav.getByRole('link', { name: 'System' })).toBeVisible()
    await expect(nav.getByRole('link', { name: 'Users' })).toHaveCount(0)
    await expect(nav.getByRole('link', { name: 'Tunnels' })).toHaveCount(0)
    await expect(nav.getByRole('link', { name: 'Groups' })).toHaveCount(0)
    await expect(nav.getByRole('link', { name: 'Roles' })).toHaveCount(0)
    await shot(page, 'desktop-mode')
  })

  test('agents', async ({ page }) => {
    await visit(page, '/agents')
    await shot(page, 'hosts')

    await visit(page, '/agents/web-server-01')
    await shot(page, 'host-detail')

    await visit(page, '/agents?tab=dependencies')
    await shot(page, 'dependency-hosts')

    await visitDependencyUsedBy(page, 'files-01')
    await shot(page, 'dependency-used-by')

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

    // The seeded failed pre-backup hook run is the newest run of its copy, so
    // it is the one row on this log that offers Retry (see seed-demo.sh).
    await visit(page, '/agents/media-store-01?tab=logs')
    const retryable = page
      .locator('.agent-row')
      .filter({ has: page.getByRole('button', { name: 'Retry', exact: true }) })
    await expect(retryable, 'the seeded hook failure offers Retry').toHaveCount(1)
    await retryable.getByRole('button', { name: 'Show detail' }).click()
    await settle(page)
    await retryable.scrollIntoViewIfNeeded()
    await shot(page, 'run-log-retry')

    await visit(page, '/agents/db-server-01?tab=settings&section=vms')
    await shot(page, 'agent-vms')

    await page
      .getByRole('button', { name: /^restore$/i })
      .first()
      .click()
    await shot(page, 'vm-restore')

    await visit(page, '/agents/db-server-01?tab=settings&section=databases')
    await expect(page.getByText('shop/orders-db'), 'the seeded database sources').toBeVisible()
    await shot(page, 'agent-databases')

    // db-server-01 backs up into two repositories (see seed-demo.sh), so its
    // Backups tab is one list across both, with the newest archive open.
    await visit(page, '/agents/db-server-01?tab=backups')
    await expect(page.locator('.archive-repo-filter'), 'two repositories').toBeVisible({
      timeout: 30_000,
    })
    await page.locator('.archive-row-body:visible').first().click()
    await waitForIndex(page)
    await shot(page, 'agent-backups')

    // Picking files and restoring them: one file ticked, the dialog asking
    // where to, into a folder.
    await visit(page, '/agents/web-server-01?tab=backups')
    await page.getByPlaceholder('Search name, host or repository').fill('web-server-01-backup')
    await page.locator('.archive-row-body:visible').first().click()
    await waitForIndex(page)
    const browserPanel = page.locator('.browser-panel')
    for (const dir of ['tmp', 'tmp.web-server-01']) {
      await browserPanel.getByText(dir, { exact: true }).click()
      await expect(browserPanel.locator('.path-crumbs')).toContainText(dir)
    }
    await page.getByRole('checkbox', { name: 'Select restore-example.txt' }).check()
    await page.getByRole('checkbox', { name: 'Select etc' }).check()
    await shot(page, 'archive-selection')
    await page.getByTestId('restore-picked').click()
    const restoreDialog = page.getByRole('dialog', { name: 'Where to restore' })
    await restoreDialog.getByRole('radio', { name: /Into a folder/ }).check()
    await restoreDialog.getByLabel('Folder', { exact: true }).fill('/srv/restore')
    await shot(page, 'restore-dialog')
    await page.keyboard.press('Escape')
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

    // The newest db-server-01 archive also holds database dumps.
    const archives = (await (await page.request.get(`/api/repos/${hourly}/archives`)).json()) as {
      name: string
      start: string
    }[]
    const [withDumps] = archives
      .filter((archive) => archive.name.startsWith('db-server-01-backup-'))
      .sort((a, b) => b.start.localeCompare(a.start))
    await visit(page, `/repos/${hourly}?tab=archives`)
    await expandAllArchiveGroups(page)
    await page.getByText(withDumps!.name, { exact: true }).first().click()
    await page.locator('.browser-view .segmented-option', { hasText: 'Databases' }).click()
    await expect(page.getByText('shop/orders-db').first(), 'the seeded dumps').toBeVisible({
      timeout: 30_000,
    })
    await shot(page, 'archive-databases')

    // Two seeded web-server-01 archives whose nginx.conf differs (see
    // start-agent.sh), so the comparison has lines to show.
    const daily = await seededIdByName(page, '/api/repos', 'server-daily')
    const nginx = await seededNginxComparison(page, daily)
    await visit(page, `/repos/${daily}?tab=archives`)
    await page.getByText('Flat', { exact: true }).first().click()
    await page.getByPlaceholder('Search name, host or repository').fill(nginx.target)
    await settle(page)
    await page.locator('.archive-row-body:visible').first().click()
    await waitForIndex(page)
    const browserPanel = page.locator('.browser-panel').last()
    for (const dir of ['tmp', 'tmp.web-server-01', 'etc', 'nginx']) {
      await browserPanel.getByText(dir, { exact: true }).click()
      await expect(browserPanel.locator('.path-crumbs')).toContainText(dir)
    }
    await page.getByRole('button', { name: 'Compare nginx.conf with another archive' }).click()
    await page.getByTestId('file-diff-against').selectOption(nginx.base)
    await expect(page.getByTestId('file-diff-body'), 'the nginx.conf comparison').toBeVisible({
      timeout: 60_000,
    })
    await shot(page, 'file-diff')
    await page.keyboard.press('Escape')

    // The same file's history, opened on the newest seeded archive against
    // the edited copy start-agent.sh leaves on web-server-01.
    await page.getByRole('button', { name: 'History of nginx.conf' }).click()
    const history = page.getByRole('dialog', { name: 'History of nginx.conf' })
    await history
      .locator(`[data-archives~="${nginx.target}"]`)
      .getByRole('button', { name: 'From' })
      .click()
    await history.getByTestId('file-history-to-live').click()
    await expect(history.getByTestId('file-diff-body'), 'the live comparison').toBeVisible({
      timeout: 60_000,
    })
    await shot(page, 'file-history')
    await page.keyboard.press('Escape')

    // offsite-imported arrives from a config import without its passphrase;
    // the dialog is captured with one typed in, never submitted.
    const imported = await seededIdByName(page, '/api/repos', 'offsite-imported')
    await visit(page, `/repos/${imported}`)
    await page.locator('.overflow-toggle').click()
    await page.getByRole('menuitem', { name: 'Set passphrase' }).click()
    const dialog = page.locator('.modal-dialog')
    await dialog.getByLabel('Passphrase').fill('demo-passphrase-123')
    await shot(page, 'repo-set-passphrase')
    await page.keyboard.press('Escape')

    const hostsRes = await page.request.get('/api/repo-hosts')
    const hosts = (await hostsRes.json()) as (Named & { ssh_host: string })[]
    await visit(page, `/repo-hosts/${hosts[0]?.id ?? 1}`)
    await shot(page, 'repo-host-detail')

    // The NAS behind "localhost": one machine for the repository host and the
    // nas-media dependency, waking before a backup, not always online, with
    // the dual-target schedule that waits for it instead listed as an
    // exception (see seed-demo.sh).
    const nas = hosts.find((h) => h.ssh_host === 'localhost')
    await visit(page, `/repo-hosts/${nas?.id ?? 1}?section=power`)
    await shot(page, 'repo-host-power', { fullPage: true })
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
    await visit(page, `/schedules/${share}?tab=settings&section=power`)
    await page.getByText('Required dependencies', { exact: true }).scrollIntoViewIfNeeded()
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
      ['/database-storage', 'database-storage'],
      ['/server-quotas', 'server-quotas'],
    ] as const) {
      await visit(page, path)
      await shot(page, name)
    }

    // The seeded restores: one of each outcome, and one waiting for its agent.
    await visit(page, '/activity?category=restores')
    await expect(page.getByTestId('restore-runs')).toBeVisible()
    await shot(page, 'activity-restores')

    // The settings form runs well past one screen; the docs show all of it.
    await visit(page, '/system')
    await shot(page, 'system', { fullPage: true })

    // The Prometheus panel the seed enabled, with its example network.
    const metricsPanel = page.locator('.panel', {
      has: page.getByRole('heading', { name: 'Prometheus metrics' }),
    })
    await expect(page.locator('#metrics-enabled')).toBeChecked()
    await expect(page.locator('#metrics-networks')).toHaveValue(/192\.0\.2\.0\/24/)
    await shotOf(metricsPanel, 'system-metrics')
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
    await page.getByPlaceholder('Search name, host or repository').fill('web-server-01-backup')
    await settle(page)
    await page.locator('.archive-row-body:visible').first().click()
    await waitForIndex(page)

    // The demo agent archives a temporary directory, so the interesting
    // folders sit a couple of levels down: open single folders until a level
    // lists more than one entry.
    const rows = page.locator('.archive-browser-layout table tbody tr')
    for (let depth = 0; depth < 5; depth++) {
      // The name has its own element: the first cell is the row's checkbox.
      const names = (await rows.locator('.name-text').allInnerTexts())
        .map((name) => name.trim())
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

  // Every schedule and agent that needs files-01, and where each need comes
  // from, for the website's dependency-hosts section.
  test('dependency used by', async ({ page }) => {
    await loginAsAdmin(page)
    await waitForLiveAgents(page)
    await visitDependencyUsedBy(page, 'files-01')
    await page.screenshot({
      path: join(WEBSITE_DIR, 'dependency-used-by.png'),
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
