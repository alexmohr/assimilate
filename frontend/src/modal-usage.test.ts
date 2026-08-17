// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { SRC, vueFiles } from './test-utils/vueFiles'

/**
 * Every dialog in the app goes through `BaseModal`, which supplies the focus
 * trap, Escape handling, scroll lock, focus restore and dialog semantics.
 *
 * The frontend previously carried 43 hand-rolled `.overlay > .dialog` blocks
 * with none of that. See `docs/contributing/ui-design-audit.md` (F-12, F-21,
 * F-22).
 */

const BASE_MODAL = join(SRC, 'components', 'BaseModal.vue')

const FILES = vueFiles()

describe('modal usage', () => {
  it('has no hand-rolled overlay markup left', () => {
    const offenders = FILES.filter((f) => readFileSync(f, 'utf-8').includes('class="overlay"')).map(
      (f) => relative(SRC, f),
    )
    expect(offenders).toEqual([])
  })

  it('has no leftover .dialog-* structural classes', () => {
    const offenders: string[] = []
    for (const f of FILES) {
      const text = readFileSync(f, 'utf-8')
      for (const cls of ['dialog-header', 'dialog-body', 'dialog-footer', 'dialog-title']) {
        if (text.includes(`class="${cls}"`)) offenders.push(`${relative(SRC, f)}: ${cls}`)
      }
    }
    expect(offenders).toEqual([])
  })

  it('has no close buttons without an accessible name', () => {
    const offenders = FILES.filter((f) =>
      readFileSync(f, 'utf-8').includes('class="close-btn"'),
    ).map((f) => relative(SRC, f))
    expect(offenders).toEqual([])
  })

  it('keeps the dialog semantics in BaseModal', () => {
    const text = readFileSync(BASE_MODAL, 'utf-8')
    expect(text).toContain('role="dialog"')
    expect(text).toContain('aria-modal="true"')
    expect(text).toContain('aria-labelledby')
    expect(text).toContain('aria-label="Close"')
    expect(text).toContain('<Teleport to="body">')
  })

  it('routes every Teleported dialog through BaseModal', () => {
    // ToastContainer is the one non-dialog teleport: it is a status region,
    // not something the user interacts with modally.
    const allowed = new Set(['components/BaseModal.vue', 'components/ToastContainer.vue'])
    const offenders = FILES.filter((f) => readFileSync(f, 'utf-8').includes('<Teleport'))
      .map((f) => relative(SRC, f))
      .filter((f) => !allowed.has(f))
    expect(offenders).toEqual([])
  })
})
