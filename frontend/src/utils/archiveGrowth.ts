// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ArchiveEntryResponse, GrowthNode } from '../types/generated'
import { comparableArchives } from './fileDiff'

/** How many of a host's earlier archives its usual growth is taken over. */
export const USUAL_GROWTH_WINDOW = 7

/**
 * How much the host's archives before `archive` in the same repository added
 * on average, at backup time, or `null` when it has none.
 *
 * Only the same host's archives count: in a repository several hosts write
 * to, the archive just before this one may be another machine's, and its
 * growth says nothing about this one.
 */
export function usualGrowth(
  archive: ArchiveEntryResponse,
  archives: readonly ArchiveEntryResponse[],
): number | null {
  const earlier = comparableArchives(archives, archive)
    .filter((other) => other.start < archive.start)
    .slice(0, USUAL_GROWTH_WINDOW)
  if (earlier.length === 0) return null
  return earlier.reduce((sum, other) => sum + other.deduplicated_size, 0) / earlier.length
}

/**
 * How many times the usual growth `added` is, rounded to one decimal, or
 * `null` when there is no usual growth to compare with.
 */
export function growthFactor(added: number, usual: number | null): number | null {
  if (usual === null || usual <= 0) return null
  return Math.round((added / usual) * 10) / 10
}

/**
 * The entries of the folder at `path` (a list of node names from the top),
 * and the folders on the way there. Stops at the deepest folder that exists,
 * so a path left over from another breakdown opens as far as it still goes.
 */
export function folderAt(
  children: readonly GrowthNode[],
  path: readonly string[],
): { entries: readonly GrowthNode[]; trail: readonly GrowthNode[] } {
  const trail: GrowthNode[] = []
  let entries = children
  for (const name of path) {
    const folder = entries.find((node) => node.kind === 'directory' && node.name === name)
    if (!folder) break
    trail.push(folder)
    entries = folder.children
  }
  return { entries, trail }
}

/** `bytes` as a share of `total`, for display: `42%`, `<1%`, or `0%`. */
export function shareLabel(bytes: number, total: number): string {
  if (total <= 0 || bytes <= 0) return '0%'
  const percent = (bytes / total) * 100
  if (percent < 1) return '<1%'
  return `${Math.round(percent)}%`
}

/** How many files a node's bytes are in, in words. */
export function filesLabel(files: number): string {
  return `${files.toLocaleString()} ${files === 1 ? 'file' : 'files'}`
}
