// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The sections of an agent's Settings tab, in sub-nav order. Kept out of the
 * component so the detail view can parse `?section=` into the union before
 * handing it back down, rather than passing a wide `string` around.
 */
export const SETTINGS_SECTIONS = [
  'identity',
  'defaults',
  'aliases',
  'power',
  'tags',
  'danger',
] as const

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

export function isSettingsSection(value: unknown): value is SettingsSection {
  return SETTINGS_SECTIONS.some((section) => section === value)
}
