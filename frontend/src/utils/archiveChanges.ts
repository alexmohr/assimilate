// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ArchiveEntryResponse, ContentEntryResponse as ContentEntry } from '../types/generated'
import { isDirectoryEntry } from './archiveSelection'
import { comparableArchives } from './fileDiff'

/** How a file differs from the same path in the previous archive of its machine. */
export type ChangeKind = 'new' | 'changed'

/**
 * The archive `current` is compared with for its change badges: the newest
 * one of the same machine that is older than it. The oldest archive has
 * none, so nothing in it is marked.
 */
export function previousArchive(
  archives: readonly ArchiveEntryResponse[],
  current: ArchiveEntryResponse,
): ArchiveEntryResponse | null {
  const started = Date.parse(current.start)
  return (
    comparableArchives(archives, current).find((archive) => Date.parse(archive.start) < started) ??
    null
  )
}

/**
 * How `entry` differs from what its folder held in the previous archive,
 * `before` by path, or null when it is unchanged, a folder, or the previous
 * folder is not known. A file counts as changed when its size or its
 * modification time differs.
 */
export function changeOf(
  entry: ContentEntry,
  before: ReadonlyMap<string, ContentEntry> | undefined,
): ChangeKind | null {
  if (before === undefined || isDirectoryEntry(entry)) return null
  const old = before.get(entry.path)
  if (old === undefined) return 'new'
  return Number(old.size) !== Number(entry.size) || old.mtime !== entry.mtime ? 'changed' : null
}
