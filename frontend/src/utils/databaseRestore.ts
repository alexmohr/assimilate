// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { DatabaseSourceRef } from '../api/databases'
import type { ArchiveDatabaseDump, DatabaseRestore } from '../types/generated'
import { hostEngineLabel, sourceName } from './databaseRuntimes'

/** Longest database name PostgreSQL keeps, in bytes. */
export const MAX_DATABASE_NAME_LENGTH = 63

const DATABASE_NAME_CHARS = /^[A-Za-z0-9_-]+$/

/**
 * Why `name` may not name the database a restore creates, or null when it
 * may. The same rule the server applies: 1 to 63 ASCII letters, digits, `_`
 * and `-`, not starting with `-` or with `pg_`, which PostgreSQL reserves.
 */
export function newDatabaseNameProblem(name: string): string | null {
  if (name.length === 0) return 'Name the new database.'
  if (name.length > MAX_DATABASE_NAME_LENGTH) {
    return `A database name is at most ${MAX_DATABASE_NAME_LENGTH} characters.`
  }
  if (!DATABASE_NAME_CHARS.test(name)) {
    return 'Use only ASCII letters, digits, _ and -.'
  }
  if (name.startsWith('-')) return 'A database name may not start with -.'
  if (name.startsWith('pg_')) return 'Names starting with pg_ are reserved by PostgreSQL.'
  return null
}

/** The name a restore into a new database suggests for `database`. */
export function restoredDatabaseName(database: string): string {
  return `${database}_restored`.slice(0, MAX_DATABASE_NAME_LENGTH)
}

/**
 * What replacing `dump.database` with the dump does. PostgreSQL restores in
 * one transaction; the other engines replace one table or collection after
 * the other, so a restore that fails can leave the database half replaced.
 */
export function replaceWarning(dump: Pick<ArchiveDatabaseDump, 'engine' | 'database'>): string {
  const lost = 'Changes made since the backup to those objects are lost.'
  const partly = (objects: string): string =>
    `Every ${objects} the dump holds is dropped from ${dump.database} and recreated from the ` +
    `archive, one after the other: a restore that fails partway leaves some replaced and ` +
    `others not. ${lost}`
  switch (dump.engine) {
    case 'postgresql':
      return (
        `Every object the dump holds is dropped from ${dump.database} and recreated from the ` +
        `archive, in one transaction. ${lost}`
      )
    case 'mysql':
    case 'mariadb':
      return partly('table')
    case 'mongodb':
      return partly('collection')
  }
}

/** Where the dump ran when the archive was written: its pod, container or socket. */
export function dumpPlace(dump: Pick<ArchiveDatabaseDump, 'runtime' | 'pod'>): string {
  if (dump.pod === '') return ''
  switch (dump.runtime) {
    case 'kubernetes':
      return `pod ${dump.pod}`
    case 'docker':
      return `container ${dump.pod}`
    case 'host':
      return `through ${dump.pod}`
  }
}

/** A source as a sentence names it: the host's own server reads as such. */
function sourcePhrase(source: DatabaseSourceRef): string {
  switch (source.runtime) {
    case 'host':
      return `the host's ${hostEngineLabel(source.name)}`
    case 'kubernetes':
    case 'docker':
      return sourceName(source)
  }
}

/** What a database restore restores and where it writes it. */
export function databaseRestoreScope(restore: DatabaseRestore): string {
  const what =
    restore.target.mode === 'new_database'
      ? `${restore.database} into ${restore.target.name}`
      : `${restore.database} (replaced)`
  return `${what} on ${sourcePhrase(restore.source)}`
}
