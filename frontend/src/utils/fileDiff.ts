// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  ArchiveEntryResponse,
  FileDiffLine,
  FileDiffResponse,
  FileDiffSide,
  LiveReadRefusal,
} from '../types/generated'
import { resolveArchiveDomain, resolveArchiveHost } from './archiveHost'
import { formatBytes } from './format'

/** How a file comparison lays the two versions out. */
export type DiffLayout = 'inline' | 'split'

/** One row of a side-by-side comparison: the base line on the left, the target on the right. */
export interface SplitRow {
  base: FileDiffLine | null
  target: FileDiffLine | null
}

/**
 * What a borg entry-type character names, as far as comparing goes: only a
 * regular file (`-`) has contents to compare. borg's characters are an
 * external format contract, so they are narrowed here, once.
 */
export function entryKind(type: string): 'file' | 'other' {
  return type === '-' ? 'file' : 'other'
}

/**
 * The archives `current` can be compared with, newest first: the other
 * archives of the same machine. A path names a file on one machine, so the
 * same path in another host's archive is a different file (or none at all).
 */
export function comparableArchives(
  archives: readonly ArchiveEntryResponse[],
  current: ArchiveEntryResponse,
): ArchiveEntryResponse[] {
  const host = resolveArchiveHost(current)
  const domain = resolveArchiveDomain(current)
  return archives
    .filter(
      (archive) =>
        archive.name !== current.name &&
        resolveArchiveHost(archive) === host &&
        resolveArchiveDomain(archive) === domain,
    )
    .sort((a, b) => Date.parse(b.start) - Date.parse(a.start))
}

/**
 * The archive a comparison opens against: the newest one older than
 * `current`, so the diff reads as what changed in it - or, for the oldest
 * archive, the newest of the rest.
 */
export function defaultComparison(
  archives: readonly ArchiveEntryResponse[],
  current: ArchiveEntryResponse,
): ArchiveEntryResponse | null {
  const others = comparableArchives(archives, current)
  const started = Date.parse(current.start)
  return others.find((archive) => Date.parse(archive.start) < started) ?? others[0] ?? null
}

/**
 * Pairs a hunk's lines for a side-by-side view. An unchanged line sits on
 * both sides; a run of removed lines followed by added ones is a change, so
 * its lines are paired off in order and whichever side runs out is padded.
 */
export function toSplitRows(lines: readonly FileDiffLine[]): SplitRow[] {
  const rows: SplitRow[] = []
  let removed: FileDiffLine[] = []
  let added: FileDiffLine[] = []

  const flush = (): void => {
    const length = Math.max(removed.length, added.length)
    for (let i = 0; i < length; i += 1) {
      rows.push({ base: removed[i] ?? null, target: added[i] ?? null })
    }
    removed = []
    added = []
  }

  for (const line of lines) {
    if (line.kind === 'removed') {
      // A removal after additions starts a new change.
      if (added.length > 0) flush()
      removed.push(line)
    } else if (line.kind === 'added') {
      added.push(line)
    } else {
      flush()
      rows.push({ base: line, target: line })
    }
  }
  flush()
  return rows
}

/** Why a file could not be read as it is on its host now, as a sentence. */
export function liveRefusalReason(reason: LiveReadRefusal, hostname: string): string {
  switch (reason) {
    case 'not_in_sources':
      return `The file is outside every backup source configured for ${hostname}, so its agent does not read it.`
    case 'not_found':
      return `The file no longer exists on ${hostname}.`
    case 'not_regular_file':
      return `On ${hostname} the path is now a directory, a link or another special file.`
    case 'unreadable':
      return `The agent on ${hostname} could not read the file.`
    case 'agent_offline':
      return `${hostname} is offline, so the file cannot be read as it is now.`
    case 'timed_out':
      return `The agent on ${hostname} did not answer in time.`
  }
}

/** The sentence shown instead of a diff when one version cannot be compared. */
export function unavailableReason(
  response: Exclude<FileDiffResponse, { status: 'text' }>,
  sideNames: Record<FileDiffSide, string>,
  hostname = 'the host',
): string {
  if (response.status === 'live_unavailable') return liveRefusalReason(response.reason, hostname)
  const name = sideNames[response.side]
  if (response.status === 'binary') {
    return `The version in ${name} is not text, so it cannot be compared line by line.`
  }
  return `The version in ${name} is larger than ${formatBytes(response.limit_bytes)}, the most the server compares.`
}
