// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { VueWrapper } from '@vue/test-utils'

/**
 * Finding controls by their label rather than by position.
 *
 * Seven specs had grown their own copy of `dialogButton`, which is the same
 * three lines every time because `BaseModal` teleports: the dialog's nodes
 * live on `document.body`, not inside the wrapper.
 */
export function dialogButton(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll<HTMLButtonElement>('.modal-dialog button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!match) throw new Error(`no dialog button labelled "${label}"`)
  return match
}

/** The first button in the wrapper whose text matches, for a label that
    changes with state ("Sync now" / "Syncing..."). */
export function findButton(wrapper: VueWrapper, label: RegExp): ReturnType<VueWrapper['find']> {
  const match = wrapper.findAll('button').find((b) => label.test(b.text()))
  if (!match) throw new Error(`no button matching ${label}`)
  return match
}
