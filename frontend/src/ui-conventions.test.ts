// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { relative } from 'node:path'
import { SRC, sourceFiles, vueFiles } from './test-utils/vueFiles'

/**
 * Conventions for forms, icons and empty/loading states.
 */

const VUE = vueFiles().filter((f) => !f.endsWith('.test.vue'))

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
    const glyphs =
      /&(times|larr|rarr|hellip|#8635|#9881|#9788|#9789|#9888|#10003|#9656);|[\u2190-\u21FF\u25A0-\u25FF\u2713\u2717\u2718\u26A0\u2715\u2716\u00D7]/
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
    // A 1,000-line scoped stylesheet is where re-declaring `.panel` starts to
    // feel cheaper than importing it, so oversized views are where duplication
    // collects. Split on a tab or dialog boundary rather than raising this
    // number.
    const LIMIT = 1800
    const offenders: string[] = []
    for (const f of VUE) {
      const lines = readFileSync(f, 'utf-8').split('\n').length
      if (lines > LIMIT) offenders.push(`${relative(SRC, f)}: ${lines} lines`)
    }
    expect(offenders).toEqual([])
  })
})

describe('component conventions', () => {
  it('keeps the badge dot inside its badge', () => {
    // `.badge-dot` is the 0.4rem circle a badge leads with. On the badge
    // itself it wins on source order, collapsing the badge to 6.4px and
    // spilling its label over whatever follows - which is exactly what the
    // Online, Enabled and Running badges did.
    const offenders: string[] = []
    for (const f of VUE) {
      const text = readFileSync(f, 'utf-8')
      for (const m of text.matchAll(/class="([^"]*\bbadge\b[^"]*)"/g)) {
        const classes = m[1].split(/\s+/)
        if (classes.includes('badge') && classes.includes('badge-dot')) {
          offenders.push(`${relative(SRC, f)}: ${m[1]}`)
        }
      }
    }
    expect(offenders).toEqual([])
  })

  it('gives every table a scroll container', () => {
    // Without one a wide table pushes the whole page sideways on a phone
    // instead of scrolling inside its own box.
    const offenders: string[] = []
    for (const f of VUE) {
      const text = readFileSync(f, 'utf-8')
      for (const m of text.matchAll(/<table\b/g)) {
        const before = text.slice(Math.max(0, m.index - 400), m.index)
        if (!before.includes('table-wrap')) offenders.push(relative(SRC, f))
      }
    }
    expect(offenders).toEqual([])
  })

  it('writes labels, headings and buttons in sentence case', () => {
    // Title Case and sentence case were both in use, twice inside one file:
    // `Display Name` against `Display name`, `Exclude Patterns` against
    // `Exclude patterns`. Page titles keep their Title Case - they are the
    // names of pages, not labels.
    const PROPER = new Set([
      'SSH',
      'SMTP',
      'IMAP',
      'API',
      'TOTP',
      'URL',
      'URI',
      'ID',
      'UI',
      'CPU',
      'RAM',
      'TLS',
      'HTTP',
      'HTTPS',
      'JSON',
      'YAML',
      'CSV',
      'DNS',
      'IP',
      'UTC',
      'GiB',
      'TiB',
      'MiB',
      'MB',
      'GB',
      'Borg',
      'Gotify',
      'Telegram',
      'Slack',
      'Discord',
      'Matrix',
      'Ntfy',
      'Pushover',
      'Assimilate',
      'GitHub',
      'Linux',
      'Prometheus',
      'Docker',
      'Webhook',
      'OTP',
      'SFTP',
    ])
    const LABEL =
      /<(?:label|span|h2|h3|dt|th)[^>]*class="[^"]*(?:field-label|panel-title|group-label|section-title|stat-label)[^"]*"[^>]*>\s*([A-Za-z][^<>{}]*?)\s*</g
    const offenders: string[] = []
    for (const f of VUE) {
      const text = readFileSync(f, 'utf-8')
      for (const m of text.matchAll(LABEL)) {
        const words = m[1].split(/\s+/).slice(1)
        const shouted = words.filter((w) => {
          const bare = w.replace(/[^A-Za-z0-9/-]/g, '')
          return /^[A-Z]/.test(bare) && !PROPER.has(bare) && bare.toUpperCase() !== bare
        })
        if (shouted.length > 0) offenders.push(`${relative(SRC, f)}: ${m[1]}`)
      }
    }
    expect(offenders).toEqual([])
  })
})
