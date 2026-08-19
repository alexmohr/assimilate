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
