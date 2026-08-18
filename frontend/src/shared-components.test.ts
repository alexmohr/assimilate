// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { SRC, vueFiles } from './test-utils/vueFiles'

/**
 * The panel, badge, table, tab and segmented-control styles live once, in
 * `src/style.css`. A second copy in a scoped block is not a second copy for
 * long: it drifts, and the drift is invisible in review because the two
 * declarations are in different files. See `skills/ui-design/SKILL.md`.
 */

const STYLE_CSS = readFileSync(join(SRC, 'style.css'), 'utf-8')

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
 * Classes that `style.css` owns. A scoped block redefining one of these wins
 * on specificity without saying so, so the component quietly stops matching
 * every other instance of the same control.
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
  'info-card',
  'info-title',
  'info-grid',
  'info-actions',
  'field-inline',
  'field-label-row',
  // Promote a class here once a second file needs it, and delete both scoped
  // copies; the two checks below then keep it that way.
  'detail-breadcrumb',
  'entity-card',
  'crumb-link',
  'crumb-sep',
  'crumb-current',
  'stat-label',
  'stat-value',
  'stat-sub',
  'group-label',
  'path-crumbs',
  'crumb',
  'crumb-last',
  'td-name',
  'name-text',
  'entry-icon',
  'td-action',
  'filter-input',
  'card-grid',
  'card-top',
  'card-info',
  'card-name',
  'card-meta',
  'card-stats',
  'card-actions',
  'meta-pill',
  'stat',
  'filter-toggle',
  'filter-badge',
  'sort-controls',
  'sort-label',
  'filters',
  'filter-row',
  'filter-group',
  'filter-label',
  'row-count',
  'form-grid',
  'form-stack',
  'field-full',
  'field-narrow',
  'field-row',
  'toggle-row',
  'toggle-row-label',
  'token-notice',
  'token-warning',
  'token-box',
  'token-text',
  'passphrase-warning',
  'passphrase-box',
  'passphrase-text',
  'info-card-header',
  'info-header-actions',
  'edit-form',
  'edit-actions',
  'error-banner',
  'error-page',
  'error-card',
  'error-code',
  'error-title',
  'error-source',
  'error-message',
  'muted',
  'select-input',
  'input-sm',
  'search-input',
  'table-wrap',
  'cell-ts',
  'cell-date',
  'cell-host',
  'cell-size',
  'cell-mono',
  'cell-muted',
  'cell-truncate',
  'loading-row',
  'chart-desc',
  'area-input',
  'area-input-sm',
  'paths-list',
  'path-item',
  'per-host-paths',
  'per-host-entry',
  'test-success',
  'test-failure',
  'deploy-result',
  'result-ok',
  'save-success',
  'break-lock-warning',
  'host-link',
  'match-ok',
  'match-warn',
  'dropdown-arrow',
  'progress-track',
  'progress-bar',
  'progress-row',
  'progress-label',
  'panel-title--truncate',
  'pulse-dot',
  'spinning',
  'fade-in',
  'detail-pre',
  'error-pre',
  'warning-pre',
  'settings-tab',
  'settings-nav',
  'settings-nav-item',
  'settings-pane',
]

/**
 * Animations are declared once, in `style.css`. A scoped `@keyframes` is
 * rewritten by the SFC compiler to a component-private name, so a second copy
 * is invisible to review but doubles the CSS and lets the two drift.
 * `BaseSkeleton`'s shimmer is genuinely its own.
 */
const SHARED_KEYFRAMES = ['spin', 'pulse', 'fade-in']

/** The exact declarations a top-level `.name { ... }` rule makes, normalised. */
function scopedDeclarations(file: string): Map<string, string> {
  const text = readFileSync(file, 'utf-8')
  const styles = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n')
  const flat = styles.replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, '')
  const out = new Map<string, string>()
  for (const m of flat.matchAll(/^\.([a-zA-Z][\w-]*)[ \t]*\{([^{}]*)\}/gm)) {
    const decls = m[2]
      .split(';')
      .map((d) => d.trim().replace(/\s+/g, ' '))
      .filter((d) => d && !d.startsWith('/*'))
      .sort()
      .join('; ')
    if (decls) out.set(m[1], decls)
  }
  return out
}

const FILES = vueFiles()

describe('shared components', () => {
  it('defines the shared component classes exactly once, in style.css', () => {
    for (const cls of OWNED) {
      expect(STYLE_CSS).toMatch(new RegExp(`^\\s*\\.${cls}\\s*[,{]`, 'm'))
    }
  })

  it('has no scoped stylesheet re-declaring a shared component property', () => {
    // Adding a local layout property to a shared class is fine - a widget may
    // need `min-width: 0` on its panel. Re-declaring a property the shared
    // rule already sets is not: the local value wins on specificity, so that
    // one instance stops matching the rest.
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

  it('declares each shared animation once, in style.css', () => {
    for (const name of SHARED_KEYFRAMES) {
      expect(STYLE_CSS).toMatch(new RegExp(`@keyframes\\s+${name}\\s*\\{`))
    }
    const offenders: string[] = []
    for (const file of FILES) {
      const text = readFileSync(file, 'utf-8')
      const styles = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)]
        .map((m) => m[1])
        .join('')
      for (const name of SHARED_KEYFRAMES) {
        if (new RegExp(`@keyframes\\s+${name}\\s*\\{`).test(styles)) {
          offenders.push(`${relative(SRC, file)}: @keyframes ${name}`)
        }
      }
    }
    expect(offenders).toEqual([])
  })

  it('has no rule copy-pasted verbatim between two scoped stylesheets', () => {
    // The property-level check above catches a partial override of a shared
    // class. This catches the other half: two components declaring the same
    // rule under the same name, before either is shared. A rule promoted to
    // `style.css` is deleted from both callers, so this stays at zero.
    const seen = new Map<string, string>()
    const offenders: string[] = []
    for (const file of FILES) {
      const name = relative(SRC, file)
      for (const [cls, decls] of scopedDeclarations(file)) {
        const key = `${cls}|${decls}`
        const first = seen.get(key)
        if (first) offenders.push(`.${cls}: ${first} and ${name}`)
        else seen.set(key, name)
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
