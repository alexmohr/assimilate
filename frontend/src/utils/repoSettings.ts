// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The sections of a repository's Settings tab, in sub-nav order. Kept out of
 * the component so the detail view can parse `?section=` into the union before
 * handing it back down, rather than passing a wide `string` around. Mirrors
 * `agentSettings.ts`.
 */
export const REPO_SETTINGS_SECTIONS = [
  'repository',
  'power',
  'quota',
  'tags',
  'console',
  'danger',
] as const

export type RepoSettingsSection = (typeof REPO_SETTINGS_SECTIONS)[number]

export function isRepoSettingsSection(value: unknown): value is RepoSettingsSection {
  return REPO_SETTINGS_SECTIONS.some((section) => section === value)
}
