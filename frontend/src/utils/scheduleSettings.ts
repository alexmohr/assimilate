// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The sections of a schedule's Settings tab, in sub-nav order. Kept out of
 * the component so the detail view can parse `?section=` into the union
 * before handing it back down, rather than passing a wide `string` around.
 * Mirrors `utils/agentSettings.ts`.
 */
export const SCHEDULE_SETTINGS_SECTIONS = ['general', 'targets', 'retention', 'advanced'] as const

export type ScheduleSettingsSection = (typeof SCHEDULE_SETTINGS_SECTIONS)[number]

export function isScheduleSettingsSection(value: unknown): value is ScheduleSettingsSection {
  return SCHEDULE_SETTINGS_SECTIONS.some((section) => section === value)
}
