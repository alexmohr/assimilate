// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

/**
 * Conventions for forms, icons and empty/loading states.
 *
 * See `docs/contributing/ui-design-audit.md` (F-13, F-15, F-16, F-17).
 */

const SRC = resolve(process.cwd(), 'src')

function sourceFiles(ext: string[]): string[] {
  const out: string[] = []
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) {
        if (entry.name !== 'generated') walk(full)
      } else if (ext.some((e) => entry.name.endsWith(e))) {
        out.push(full)
      }
    }
  }
  walk(SRC)
  return out.sort()
}

const VUE = sourceFiles(['.vue']).filter((f) => !f.endsWith('.test.vue'))

/** Attribute text of every opening tag of the given names, quote-aware. */
function openingTags(text: string, names: string[]): string[] {
  const out: string[] = []
  let i = 0
  while (i < text.length) {
    const m = /<(input|select|textarea)\b/.exec(text.slice(i))
    if (!m) break
    const start = i + m.index
    if (!names.includes(m[1])) {
      i = start + 1
      continue
    }
    let j = start + 1
    let quote: string | null = null
    while (j < text.length) {
      const c = text[j]
      if (quote) {
        if (c === quote) quote = null
      } else if (c === '"' || c === "'") {
        quote = c
      } else if (c === '>') {
        break
      }
      j += 1
    }
    out.push(text.slice(start, j + 1))
    i = j + 1
  }
  return out
}

describe('form conventions', () => {
  it('uses one form vocabulary', () => {
    // `.field` / `.field-label` / `.input` are defined in style.css. The
    // parallel `.form-group` / `.form-label` / `.form-input` set, and
    // UsersView's bare-element styling, are gone.
    const retired = ['form-group', 'form-label', 'form-input', 'msg-error', 'msg-success']
    const offenders: string[] = []
    for (const f of VUE.concat(sourceFiles(['.css']))) {
      const text = readFileSync(f, 'utf-8')
      for (const cls of retired) {
        if (new RegExp(`class="(?:[^"]*\\s)?${cls}(?:\\s[^"]*)?"`).test(text)) {
          offenders.push(`${relative(SRC, f)}: ${cls}`)
        }
      }
    }
    expect(offenders).toEqual([])
  })

  it('gives every text control the shared .input class', () => {
    const offenders: string[] = []
    for (const f of VUE) {
      for (const tag of openingTags(readFileSync(f, 'utf-8'), ['input', 'select', 'textarea'])) {
        // Checkboxes, radios and file pickers are native controls with their
        // own affordance; `.input` is for text-shaped fields.
        if (/type="(checkbox|radio|file)"/.test(tag)) continue
        if (/class="(?:[^"]*\s)?input(?:\s[^"]*)?"/.test(tag)) continue
        offenders.push(`${relative(SRC, f)}: ${tag.split('\n')[0]}`)
      }
    }
    expect(offenders).toEqual([])
  })
})

describe('icon conventions', () => {
  it('renders icons from the icon library, not as font glyphs', () => {
    // Entity glyphs take their weight from the system font, so they never
    // match the 2px stroke around them and they shift between platforms.
    const glyphs = /&(times|larr|rarr|hellip|#8635|#9881|#9788|#9789|#9888|#10003|#9656);/
    const offenders: string[] = []
    for (const f of VUE) {
      const text = readFileSync(f, 'utf-8')
      // placeholder="" text legitimately contains &#10; newlines
      const template = text.replace(/placeholder="[^"]*"/g, '')
      if (glyphs.test(template)) offenders.push(relative(SRC, f))
    }
    expect(offenders).toEqual([])
  })

  it('sizes icons from a fixed set', () => {
    // 12 inline-with-small-text, 14 inline and in controls, 16 headings,
    // 20 section headers, 40 empty states.
    const allowed = new Set(['12', '14', '16', '20', '40'])
    const offenders: string[] = []
    for (const f of VUE) {
      for (const m of readFileSync(f, 'utf-8').matchAll(/:size="(\d+)"/g)) {
        if (!allowed.has(m[1])) offenders.push(`${relative(SRC, f)}: ${m[1]}`)
      }
    }
    expect(offenders).toEqual([])
  })
})

describe('state conventions', () => {
  it('punctuates busy labels one way', () => {
    const offenders: string[] = []
    for (const f of VUE) {
      if (readFileSync(f, 'utf-8').includes('…')) offenders.push(relative(SRC, f))
    }
    expect(offenders).toEqual([])
  })

  it('keeps EmptyState as the only block-level empty state', () => {
    // A bare centred sentence gives the user nothing to act on. `.state-msg`
    // stays for errors and for the inline widget variant.
    const offenders: string[] = []
    for (const f of VUE) {
      const text = readFileSync(f, 'utf-8')
      for (const m of text.matchAll(/class="state-msg"[\s\S]{0,200}?>([\s\S]*?)</g)) {
        const body = m[1].trim()
        if (/^No .*(yet|created|configured)\b/i.test(body)) {
          offenders.push(`${relative(SRC, f)}: ${body.slice(0, 50)}`)
        }
      }
    }
    expect(offenders).toEqual([])
  })
})

describe('view size', () => {
  it('keeps every view under the size at which shared styles get re-declared', () => {
    // F-24: five views were over 2,000 lines and the largest was 3,359. Every
    // duplication in Part II of the audit lived in a file like that, because a
    // 1,000-line scoped stylesheet is where re-declaring `.panel` feels
    // cheaper than importing it. Split on a tab or dialog boundary rather than
    // raising this number.
    const LIMIT = 1800
    const offenders: string[] = []
    for (const f of VUE) {
      const lines = readFileSync(f, 'utf-8').split('\n').length
      if (lines > LIMIT) offenders.push(`${relative(SRC, f)}: ${lines} lines`)
    }
    expect(offenders).toEqual([])
  })
})
