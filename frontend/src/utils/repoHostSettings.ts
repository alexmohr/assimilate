// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The sections of a repository host's page, in sub-nav order. Kept out of the
 * component so the view can parse `?section=` into the union before handing it
 * down, rather than passing a wide `string` around. Mirrors `repoSettings.ts`.
 */
export const REPO_HOST_SECTIONS = ['connection', 'power', 'repositories', 'danger'] as const

export type RepoHostSection = (typeof REPO_HOST_SECTIONS)[number]

export function isRepoHostSection(value: unknown): value is RepoHostSection {
  return REPO_HOST_SECTIONS.some((section) => section === value)
}
