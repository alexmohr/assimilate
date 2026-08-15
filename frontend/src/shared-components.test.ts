// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'

/**
 * The panel, badge, table, tab and segmented-control styles live once, in
 * `src/style.css`. Before the audit each was copy-pasted into every view that
 * used it, and the copies had drifted into visually distinct variants:
 * 12 panels, 9 badges, 7 tables, 4 segmented controls, 3 tab bars.
 *
 * See `docs/contributing/ui-design-audit.md` (F-06 to F-11, F-14, F-26, F-27).
 */

const SRC = resolve(process.cwd(), 'src')
const STYLE_CSS = readFileSync(join(SRC, 'style.css'), 'utf-8')

function vueFiles(): string[] {
  const out: string[] = []
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith('.vue')) out.push(full)
    }
  }
  walk(SRC)
  return out.sort()
}

/** Properties a top-level `.name { ... }` rule sets, per class name. */
function ruleProperties(css: string): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>()
  for (const m of css.matchAll(/^[ \t]*\.([a-zA-Z][\w-]*)[ \t]*\{([^{}]*)\}/gm)) {
    const props = out.get(m[1]) ?? new Set<string>()
    for (const decl of m[2].split(';')) {
      const name = decl.split(':')[0]?.trim()
      if (name && !name.startsWith('/*')) props.add(name)
    }
    out.set(m[1], props)
  }
  return out
}

function scopedRules(file: string): Map<string, Set<string>> {
  const text = readFileSync(file, 'utf-8')
  const styles = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n')
  // Responsive overrides inside @media are deliberate local adjustments to a
  // shared component, not a competing definition of it.
  return ruleProperties(styles.replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, ''))
}

/**
 * Classes that `style.css` owns. A scoped block redefining one of these
 * silently wins on specificity, which is how ScheduleDetailView ended up with
 * a Delete button that hovered differently from every other Delete button.
 */
const OWNED = [
  'btn',
  'btn-primary',
  'btn-ghost',
  'btn-danger',
  'btn-danger-text',
  'btn-sm',
  'btn-xs',
  'panel',
  'panel-header',
  'panel-title',
  'badge',
  'segmented',
  'segmented-option',
  'tabs',
  'tab',
  'data-table',
  'state-msg',
  'field',
  'field-label',
  'field-hint',
  'input',
  'form-error',
  'page-header',
  'page-title',
  'toolbar',
  'danger-zone',
  'danger-body',
  'danger-info',
  'danger-heading',
  'danger-desc',
  'tag-pill',
  'tag-dropdown',
]

const FILES = vueFiles()

describe('shared components', () => {
  it('defines the shared component classes exactly once, in style.css', () => {
    for (const cls of OWNED) {
      expect(STYLE_CSS).toMatch(new RegExp(`^\\s*\\.${cls}\\s*\\{`, 'm'))
    }
  })

  it('has no scoped stylesheet re-declaring a shared component property', () => {
    // Adding a local layout property to a shared class is fine - a widget may
    // need `min-width: 0` on its panel. Re-declaring a property the shared
    // rule already sets is not: it silently wins on specificity, which is how
    // one Delete button ended up hovering differently from every other one.
    const shared = ruleProperties(STYLE_CSS)
    const offenders: string[] = []
    for (const file of FILES) {
      const name = relative(SRC, file)
      // BaseModal, BaseTabs and BaseSegmented are the components themselves.
      if (name.startsWith('components/Base')) continue
      const scoped = scopedRules(file)
      for (const cls of OWNED) {
        const own = shared.get(cls)
        const local = scoped.get(cls)
        if (!own || !local) continue
        for (const prop of local) {
          if (own.has(prop)) offenders.push(`${name}: .${cls} { ${prop} }`)
        }
      }
    }
    expect(offenders).toEqual([])
  })

  it('has no leftover per-view badge, tab or segmented variants', () => {
    const retired = [
      'status-badge',
      'type-badge',
      'role-badge',
      'you-badge',
      'seeded-badge',
      'badge-imported',
      'badge-hidden',
      'badge-level',
      'tab-btn',
      'tab-bar',
      'toggle-btn',
      'seg-btn',
      'segment-btn',
      'mode-btn',
      'users-table',
      'tunnels-table',
      'sessions-table',
      'tokens-table',
      'reports-table',
      'storage-table',
      'matrix-table',
      'permissions-table',
    ]
    const offenders: string[] = []
    for (const file of FILES) {
      const text = readFileSync(file, 'utf-8')
      for (const cls of retired) {
        if (new RegExp(`class="[^"]*(?:^|")?(?:\\s|")${cls}(?:\\s|")`).test(` ${text}`)) {
          offenders.push(`${relative(SRC, file)}: ${cls}`)
        }
      }
    }
    expect(offenders).toEqual([])
  })

  it('gives tab strips and segmented controls their ARIA roles', () => {
    const tabs = readFileSync(join(SRC, 'components', 'BaseTabs.vue'), 'utf-8')
    expect(tabs).toContain('role="tablist"')
    expect(tabs).toContain('role="tab"')
    expect(tabs).toContain(':aria-selected')

    const segmented = readFileSync(join(SRC, 'components', 'BaseSegmented.vue'), 'utf-8')
    expect(segmented).toContain('role="radiogroup"')
    expect(segmented).toContain('role="radio"')
    expect(segmented).toContain(':aria-checked')
  })
})
