// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The sections of a dependency's page, in sub-nav order. Kept out of the
 * component so the view can parse `?section=` into the union before handing it
 * down, rather than passing a wide `string` around. Mirrors `repoHostSettings.ts`.
 */
export const DEPENDENCY_HOST_SECTIONS = ['connection', 'power', 'used-by', 'danger'] as const

export type DependencyHostSection = (typeof DEPENDENCY_HOST_SECTIONS)[number]

export function isDependencyHostSection(value: unknown): value is DependencyHostSection {
  return DEPENDENCY_HOST_SECTIONS.some((section) => section === value)
}
