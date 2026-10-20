// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ArchiveEntryResponse } from '../types/generated'

/**
 * An archive together with the repository it lives in.
 *
 * An archive name is only unique within a repository: a schedule that writes
 * into several targets puts the same name into each, and an agent backing up
 * to two repositories has two archive lists. Every screen that lists archives
 * from more than one repository - an agent's Backups tab, a multi-target
 * schedule's - hands the one explorer a single list, so each entry carries
 * which copy it is, and everything that acts on one (browse, restore, delete)
 * acts against that repository.
 */
export interface RepoArchive extends ArchiveEntryResponse {
  repo_id: number
  repo_name: string
}

/** A repository as the archive list names it. */
export interface ArchiveRepo {
  id: number
  name: string
}

/** Tags every archive of one repository's list with that repository. */
export function inRepo(
  archives: readonly ArchiveEntryResponse[],
  repo: ArchiveRepo,
): RepoArchive[] {
  return archives.map((archive) => ({ ...archive, repo_id: repo.id, repo_name: repo.name }))
}

/** The identity of one copy: its repository and its name. */
export function archiveKey(archive: Pick<RepoArchive, 'repo_id' | 'name'>): string {
  return `${archive.repo_id}/${archive.name}`
}

/** Whether `a` and `b` name the same copy, for a selection that may be null. */
export function sameArchive(
  a: Pick<RepoArchive, 'repo_id' | 'name'> | null | undefined,
  b: Pick<RepoArchive, 'repo_id' | 'name'> | null | undefined,
): boolean {
  return a != null && b != null && a.repo_id === b.repo_id && a.name === b.name
}

/**
 * The repositories a list spans, in the order they first appear - the order
 * the caller listed them in - each with how many archives it holds.
 */
export function reposOf(archives: readonly RepoArchive[]): (ArchiveRepo & { count: number })[] {
  const repos = new Map<number, ArchiveRepo & { count: number }>()
  for (const archive of archives) {
    const repo = repos.get(archive.repo_id)
    if (repo) repo.count += 1
    else repos.set(archive.repo_id, { id: archive.repo_id, name: archive.repo_name, count: 1 })
  }
  return [...repos.values()]
}
