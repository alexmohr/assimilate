// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { FileHistoryEntry, FileHistoryResponse } from '../types/generated'
import { formatDateShort } from './format'

/** The key of the file as it is on its host now, among a history's choices. */
export const LIVE = 'live'

/**
 * One version a file history can compare: the file as it is now, or one
 * stored version, named by its newest archive (any of a version's archives
 * holds the same file).
 */
export interface VersionChoice {
  key: string
  /** The archive to read it from; `null` for the file as it is now. */
  archiveName: string | null
}

/** A history entry with its key, as the dialog lists it. */
export type ListedEntry = FileHistoryEntry & { key: string }

/** Every entry of `history`, keyed by its newest archive. */
export function listedEntries(history: FileHistoryResponse): ListedEntry[] {
  return history.entries.map((entry) => ({ ...entry, key: entry.archives[0]?.name ?? '' }))
}

/**
 * What can be compared, newest first: the file as it is now when its host is
 * online, then each stored version. When the archive the history was opened
 * from is not among them (it is not indexed yet), it is added, so there is
 * always something to compare.
 */
export function versionChoices(history: FileHistoryResponse, openedFrom: string): VersionChoice[] {
  const live: VersionChoice[] = history.agent_online ? [{ key: LIVE, archiveName: null }] : []
  const versions = history.entries.flatMap((entry): VersionChoice[] =>
    entry.kind === 'version' && entry.archives[0] !== undefined
      ? [{ key: entry.archives[0].name, archiveName: entry.archives[0].name }]
      : [],
  )
  const covered = history.entries.some(
    (entry) =>
      entry.kind === 'version' && entry.archives.some((archive) => archive.name === openedFrom),
  )
  const opened: VersionChoice[] = covered ? [] : [{ key: openedFrom, archiveName: openedFrom }]
  return [...live, ...versions, ...opened]
}

/**
 * The pair a history opens on: the newest stored version against the file as
 * it is now, which answers "what changed since the last backup?" - or, with
 * the host offline, the two newest stored versions.
 */
export function defaultPair(choices: readonly VersionChoice[]): {
  from: string | null
  to: string | null
} {
  const [newest, next] = choices
  if (newest === undefined) return { from: null, to: null }
  if (next === undefined) return { from: newest.key, to: null }
  return { from: next.key, to: newest.key }
}

/** The span of dates a run of archives covers, newest last. */
export function archiveSpan(entry: FileHistoryEntry): string {
  const newest = entry.archives[0]
  const oldest = entry.archives[entry.archives.length - 1]
  if (newest === undefined || oldest === undefined) return ''
  if (newest.name === oldest.name) return formatDateShort(newest.start)
  return `${formatDateShort(oldest.start)} to ${formatDateShort(newest.start)}`
}

/** How many archives a run covers, as a phrase. */
export function archiveCount(entry: FileHistoryEntry): string {
  const count = entry.archives.length
  return count === 1 ? '1 archive' : `${count} archives`
}
