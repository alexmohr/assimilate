// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  databaseRestoreScope,
  dumpPlace,
  newDatabaseNameProblem,
  newDatabaseLocation,
  replaceWarning,
  restoredDatabaseName,
  restoresIntoNewDatabase,
} from './databaseRestore'
import { makeDatabaseDump, makeDatabaseRestoreRun } from '../test-utils/databaseDump'

describe('newDatabaseNameProblem', () => {
  it('accepts what the server accepts', () => {
    for (const name of ['orders_restored', 'a', 'Orders-2', 'x'.repeat(63), 'pgdata']) {
      expect(newDatabaseNameProblem(name)).toBeNull()
    }
  })

  it('says why a name is refused', () => {
    expect(newDatabaseNameProblem('')).toBe('Name the new database.')
    expect(newDatabaseNameProblem('x'.repeat(64))).toBe('A database name is at most 63 characters.')
    expect(newDatabaseNameProblem('orders restored')).toBe(
      'Use only ASCII letters, digits, _ and -.',
    )
    expect(newDatabaseNameProblem('bestellungen_ä')).toBe(
      'Use only ASCII letters, digits, _ and -.',
    )
    expect(newDatabaseNameProblem('-orders')).toBe('A database name may not start with -.')
    expect(newDatabaseNameProblem('pg_orders')).toBe(
      'Names starting with pg_ are reserved by PostgreSQL.',
    )
  })
})

describe('restoredDatabaseName', () => {
  it('suffixes the database, kept within the length PostgreSQL allows', () => {
    const named = (database: string) => restoredDatabaseName(makeDatabaseDump({ database }))
    expect(named('orders')).toBe('orders_restored')
    const long = named('x'.repeat(60))
    expect(long).toHaveLength(63)
    expect(newDatabaseNameProblem(long)).toBeNull()
  })

  it('suggests a SQLite file by its name alone', () => {
    const sqlite = (database: string) =>
      restoredDatabaseName(makeDatabaseDump({ engine: 'sqlite', database }))
    expect(sqlite('/data/db.sqlite3')).toBe('db_restored')
    expect(sqlite('/srv/app/state')).toBe('state_restored')
    expect(newDatabaseNameProblem(sqlite('/data/db.sqlite3'))).toBeNull()
  })
})

describe('newDatabaseLocation', () => {
  it('names the SQLite file a restore creates, next to the original', () => {
    const dump = makeDatabaseDump({ engine: 'sqlite', database: '/data/db.sqlite3' })
    expect(newDatabaseLocation(dump, 'copy')).toBe(
      'Created as /data/copy.sqlite3, next to /data/db.sqlite3',
    )
    const bare = makeDatabaseDump({ engine: 'sqlite', database: '/srv/app/state' })
    expect(newDatabaseLocation(bare, 'copy')).toBe(
      'Created as /srv/app/copy, next to /srv/app/state',
    )
    expect(newDatabaseLocation(makeDatabaseDump(), 'copy')).toBe('Created next to orders')
  })
})

describe('dumpPlace', () => {
  it('names the pod, container or socket the dump ran through', () => {
    expect(dumpPlace({ runtime: 'kubernetes', pod: 'orders-db-1' })).toBe('pod orders-db-1')
    expect(dumpPlace({ runtime: 'docker', pod: 'wiki-db-1' })).toBe('container wiki-db-1')
    expect(dumpPlace({ runtime: 'host', pod: '/var/run/postgresql' })).toBe(
      'through /var/run/postgresql',
    )
    expect(dumpPlace({ runtime: 'host', pod: '' })).toBe('')
  })
})

describe('databaseRestoreScope', () => {
  it('names the database, where it went and its source', () => {
    const restore = (overrides: Parameters<typeof makeDatabaseRestoreRun>[0]) =>
      makeDatabaseRestoreRun(overrides).database!
    expect(databaseRestoreScope(restore({}))).toBe('orders into orders_restored on shop/orders-db')
    expect(databaseRestoreScope(restore({ target: { mode: 'original' } }))).toBe(
      'orders (replaced) on shop/orders-db',
    )
    expect(
      databaseRestoreScope(
        restore({ source: { runtime: 'docker', namespace: '', name: 'wiki-db' } }),
      ),
    ).toBe('orders into orders_restored on wiki-db')
    expect(
      databaseRestoreScope(
        restore({ source: { runtime: 'host', namespace: '', name: 'host' }, database: 'app' }),
      ),
    ).toBe("app into orders_restored on the host's PostgreSQL")
    expect(
      databaseRestoreScope(
        restore({ source: { runtime: 'host', namespace: '', name: 'mariadb' }, database: 'wiki' }),
      ),
    ).toBe("wiki into orders_restored on the host's MariaDB")
  })
})

describe('replaceWarning', () => {
  it('promises one transaction only for PostgreSQL', () => {
    expect(replaceWarning(makeDatabaseDump())).toContain('dropped from orders')
    expect(replaceWarning(makeDatabaseDump())).toContain('in one transaction')
    for (const engine of ['mysql', 'mariadb', 'mongodb'] as const) {
      const warning = replaceWarning(makeDatabaseDump({ engine, database: 'wiki' }))
      expect(warning).not.toContain('transaction')
      expect(warning).toContain('dropped from wiki')
      expect(warning).toContain('leaves some replaced and others not')
    }
  })

  it('names what each engine replaces', () => {
    expect(replaceWarning(makeDatabaseDump({ engine: 'mariadb' }))).toMatch(/^Every table /)
    expect(replaceWarning(makeDatabaseDump({ engine: 'mongodb' }))).toMatch(/^Every collection /)
  })

  it('promises SQLite that a failed restore leaves the file alone', () => {
    const warning = replaceWarning(makeDatabaseDump({ engine: 'sqlite', database: '/data/app.db' }))
    expect(warning).toContain('copied over /data/app.db in one step')
    expect(warning).toContain('leaves it as it was')
  })
})

describe('replaceWarning for the key-value and time series engines', () => {
  it('says Redis and Valkey replace every key without a restart', () => {
    for (const engine of ['redis', 'valkey'] as const) {
      const warning = replaceWarning(makeDatabaseDump({ engine, database: 'instance' }))
      expect(warning).toContain('Every key on the server is replaced')
      expect(warning).toContain('without a restart')
      expect(warning).toContain('partly replaced')
    }
  })

  it('says InfluxDB 2 replaces the whole instance, users and tokens included', () => {
    const warning = replaceWarning(makeDatabaseDump({ engine: 'influxdb', database: 'instance' }))
    expect(warning).toContain('The whole instance is replaced')
    expect(warning).toContain('users and tokens included')
  })

  it('warns that an InfluxDB 1 database is dropped before it is restored', () => {
    const warning = replaceWarning(makeDatabaseDump({ engine: 'influxdb1', database: 'telegraf' }))
    expect(warning).toContain('telegraf is dropped and then restored')
    expect(warning).toContain('leaves it missing')
  })
})

describe('restoresIntoNewDatabase', () => {
  it('is false only for the engines that dump their whole server', () => {
    expect(restoresIntoNewDatabase('redis')).toBe(false)
    expect(restoresIntoNewDatabase('valkey')).toBe(false)
    expect(restoresIntoNewDatabase('influxdb')).toBe(false)
    for (const engine of [
      'postgresql',
      'mysql',
      'mariadb',
      'mongodb',
      'sqlite',
      'influxdb1',
    ] as const) {
      expect(restoresIntoNewDatabase(engine)).toBe(true)
    }
  })
})
