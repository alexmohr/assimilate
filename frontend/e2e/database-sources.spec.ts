// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { expect, loginAsAdmin, test } from './fixtures'

test.describe('database sources', () => {
  test('names the engine every source runs', async ({ page }) => {
    await loginAsAdmin(page)
    await page.goto('/agents/db-server-01?tab=settings&section=databases')

    const row = (name: string) => page.locator('tbody tr', { hasText: name })
    for (const [name, engine] of [
      ['shop/orders-db', 'PostgreSQL'],
      ['wiki-db', 'MariaDB'],
      ['chat/mongo', 'MongoDB'],
      ['MariaDB on this host', 'Host server'],
    ] as const) {
      await expect(row(name)).toContainText(engine)
    }
    await expect(row('chat/mongo')).toContainText('Annotated pod')
    await expect(row('wiki-db')).toContainText('Labelled container')
    await expect(row('MariaDB on this host')).toContainText(
      'as root through /run/mysqld/mysqld.sock',
    )
    await expect(page.getByText('Dumped as root through /run/mysqld/mysqld.sock')).toBeVisible()
  })
})
