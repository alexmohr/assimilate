// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The project website (website/) in a real browser: rendered by
// scripts/render_website.py into a temporary directory, then served through
// request routing on an https origin, so no server is needed and the page is
// a secure context with the real clipboard API. The unit tests for the same
// script are in src/website/site.test.ts.

import { execFileSync } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { BrowserContext, Page } from '@playwright/test'
import { expect, test } from './fixtures'

const ORIGIN = 'https://assimilate.website.test'
const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '../..')

let siteDir = ''

test.beforeAll(() => {
  siteDir = mkdtempSync(join(tmpdir(), 'assimilate-website-'))
  execFileSync('python3', ['scripts/render_website.py', 'website', siteDir], { cwd: REPO_ROOT })
})

test.afterAll(() => {
  rmSync(siteDir, { recursive: true, force: true })
})

async function serveWebsite(context: BrowserContext): Promise<void> {
  await context.route(`${ORIGIN}/**`, async (route) => {
    const path = decodeURIComponent(new URL(route.request().url()).pathname)
    let file = join(siteDir, path)
    if (existsSync(file) && statSync(file).isDirectory()) file = join(file, 'index.html')
    // Links into /docs/ point at the separately built MkDocs site.
    if (!existsSync(file)) return route.fulfill({ status: 404, body: 'not found' })
    await route.fulfill({ path: file })
  })
}

async function open(page: Page, path = '/'): Promise<string[]> {
  const errors: string[] = []
  page.on('pageerror', (err) => errors.push(err.message))
  await serveWebsite(page.context())
  await page.goto(`${ORIGIN}${path}`)
  return errors
}

test.describe('Project website', () => {
  test('every page loads without script errors', async ({ page }) => {
    for (const path of ['/', '/compare/', '/privacy/']) {
      const errors = await open(page, path)
      await expect(page.locator('#site-nav')).toBeAttached()
      expect(errors, `script errors on ${path}`).toEqual([])
    }
  })

  test('the menu button opens and closes the navigation on a phone', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await open(page)
    const toggle = page.locator('.nav-toggle')
    const nav = page.locator('#site-nav')

    await expect(toggle).toBeVisible()
    await toggle.click()
    await expect(nav).toHaveClass(/\bopen\b/)
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')

    await toggle.click()
    await expect(nav).not.toHaveClass(/\bopen\b/)
    await expect(toggle).toHaveAttribute('aria-expanded', 'false')
  })

  test('the quick start copy button copies the compose file', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: ORIGIN })
    await open(page)
    const button = page.locator('[data-copy="compose"]')
    const expected = (await page.locator('#compose').innerText()).trim()

    await button.click()
    await expect(button).toHaveText('Copied')
    const copied = await page.evaluate(() => navigator.clipboard.readText())
    expect(copied).toBe(expected)
    // Only the compose file, so it can be saved as docker-compose.yml as is.
    expect(copied).toMatch(/^# docker-compose\.yml\nservices:\n/)
    expect(copied).toMatch(/\n {2}ssh_keys:$/)
    await expect(button).toHaveText('Copy', { timeout: 5_000 })
  })

  test('the quick start commands copy on their own', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: ORIGIN })
    await open(page)
    const button = page.locator('[data-copy="start-commands"]')

    await button.click()
    await expect(button).toHaveText('Copied')
    const copied = await page.evaluate(() => navigator.clipboard.readText())
    expect(copied.split('\n').slice(1)).toEqual([
      'export ASSIMILATE_SECRET_KEY=$(openssl rand -hex 32)',
      'docker compose up -d',
    ])
    expect(copied).not.toContain('services:')
  })

  test('sections below the fold reveal as they scroll into view', async ({ page }) => {
    await open(page)
    await expect(page.locator('html')).toHaveClass(/\bjs\b/)
    const reveals = page.locator('[data-reveal]')
    // Pin one element by position: a `.pending` locator would move on to the
    // next pending element as soon as this one is revealed.
    const index = await reveals.evaluateAll((els) =>
      els.findIndex((el) => el.classList.contains('pending')),
    )
    expect(index, 'some section starts below the fold').toBeGreaterThanOrEqual(0)
    const section = reveals.nth(index)

    await section.scrollIntoViewIfNeeded()
    await expect(section).not.toHaveClass(/\bpending\b/)
  })

  test('nothing is hidden for reduced-motion visitors', async ({ browser }) => {
    const context = await browser.newContext({ reducedMotion: 'reduce' })
    const page = await context.newPage()
    await open(page)
    await expect(page.locator('[data-reveal]').first()).toBeAttached()
    await expect(page.locator('[data-reveal].pending')).toHaveCount(0)
    await expect(page.locator('html')).not.toHaveClass(/\bjs\b/)
    await context.close()
  })
})
