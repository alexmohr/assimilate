// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ArchiveDatabaseDump, DatabaseRestore, RestoreRun } from '../types/generated'
import { makeRestoreRun } from './restoreRun'

/** A dump of `orders` from the `shop/orders-db` CloudNativePG cluster, for tests to adjust. */
export function makeDatabaseDump(
  overrides: Partial<ArchiveDatabaseDump> = {},
): ArchiveDatabaseDump {
  return {
    manifest_path: 'var/lib/assimilate/database-dumps/manifest.json',
    archive_path: 'var/lib/assimilate/database-dumps/shop/orders-db/orders.dump',
    runtime: 'kubernetes',
    namespace: 'shop',
    name: 'orders-db',
    kind: 'cnpg_cluster',
    engine: 'postgresql',
    pod: 'orders-db-1',
    database: 'orders',
    path: 'shop/orders-db/orders.dump',
    format: 'pg_custom',
    ...overrides,
  }
}

/** A restore of `orders` into `orders_restored` on `db-01`, running, for tests to adjust. */
export function makeDatabaseRestoreRun(
  database: Partial<DatabaseRestore> = {},
  overrides: Partial<RestoreRun> = {},
): RestoreRun {
  return makeRestoreRun({
    hostname: 'db-01',
    paths: ['var/lib/assimilate/database-dumps/shop/orders-db/orders.dump'],
    target_path: null,
    database: {
      source: { runtime: 'kubernetes', namespace: 'shop', name: 'orders-db' },
      database: 'orders',
      archive_path: 'var/lib/assimilate/database-dumps/shop/orders-db/orders.dump',
      format: 'pg_custom',
      target: { mode: 'new_database', name: 'orders_restored' },
      ...database,
    },
    ...overrides,
  })
}
