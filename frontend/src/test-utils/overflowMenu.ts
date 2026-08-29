// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { flushPromises, type VueWrapper } from '@vue/test-utils'

/**
 * Driving a detail header's overflow menu. Every header spec needs the same
 * two lines to reach the actions it does not promote to the accented slot.
 */
export async function openMenu(wrapper: VueWrapper): Promise<void> {
  await wrapper.find('.overflow-toggle').trigger('click')
  await flushPromises()
}

export function menuLabels(wrapper: VueWrapper): string[] {
  return wrapper.findAll('.overflow-menu-item').map((i) => i.text().trim())
}

/**
 * Clicks the first open menu item whose label starts with `labelPrefix` -
 * for an item whose label carries a live count (e.g. "Clean up failed
 * backups (3)") and so can't be matched by exact text.
 */
export async function clickMenuItemStartingWith(
  wrapper: VueWrapper,
  labelPrefix: string,
): Promise<void> {
  await wrapper
    .findAll('.overflow-menu-item')
    .find((i) => i.text().trim().startsWith(labelPrefix))!
    .trigger('click')
}
