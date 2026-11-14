// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/** The sections of an agent's Sources tab, in rail order. */
export const SOURCES_SECTIONS = ['files', 'vms', 'databases', 'used-by'] as const

export type SourcesSection = (typeof SOURCES_SECTIONS)[number]

export function isSourcesSection(value: unknown): value is SourcesSection {
  return SOURCES_SECTIONS.some((section) => section === value)
}
