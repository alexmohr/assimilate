// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { SRC, sourceFiles } from './test-utils/vueFiles'

/**
 * Guards the design tokens declared in `src/style.css`.
 *
 * A literal font size, radius or transition duration is a value nothing else
 * can follow: it does not move when the scale does, and it does not show up
 * when someone greps for the token. A `var()` naming a token that does not
 * exist resolves to nothing at all. See `skills/ui-design/SKILL.md`.
 */

const STYLE_CSS = join(SRC, 'style.css')

/** Everything a `<style>` block or `.css` file contributes, joined per file. */
function styleBlocks(file: string): string {
  const text = readFileSync(file, 'utf-8')
  if (file.endsWith('.css')) return text
  return [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n')
}

/** Custom properties defined anywhere in style.css (`--name:` at rule level). */
function definedTokens(): Set<string> {
  const css = readFileSync(STYLE_CSS, 'utf-8')
  return new Set([...css.matchAll(/^\s*(--[a-z0-9-]+)\s*:/gim)].map((m) => m[1]))
}

const FILES = sourceFiles(['.vue', '.css'])
const DEFINED = definedTokens()

describe('design tokens', () => {
  it('declares the full type scale', () => {
    for (const step of ['2xs', 'xs', 'sm', 'base', 'md', 'lg', 'xl', '2xl', '3xl']) {
      expect(DEFINED.has(`--fs-${step}`)).toBe(true)
    }
  })

  it('declares radius, duration and colour tokens used across the app', () => {
    for (const token of [
      '--radius',
      '--radius-sm',
      '--radius-pill',
      '--duration-fast',
      '--duration-base',
      '--duration-slow',
      '--duration-value',
      '--danger-hover',
      '--bg-code',
    ]) {
      expect(DEFINED.has(token)).toBe(true)
    }
  })

  it('names the type scale outside the Tailwind --text-* namespace', () => {
    // Tailwind's theme defines --text-sm/--text-base/... to drive its text-*
    // utilities. Re-defining those names would make the same token resolve to
    // two different values depending on how it was reached.
    const css = readFileSync(STYLE_CSS, 'utf-8')
    expect(css).not.toMatch(/^\s*--text-(2xs|xs|sm|base|md|lg|xl|2xl|3xl)\s*:/m)
  })

  it('resolves every var() reference to a defined token', () => {
    const unresolved: string[] = []
    for (const file of FILES) {
      for (const m of styleBlocks(file).matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
        // Tailwind emits its own theme variables; only guard our namespaces.
        if (!DEFINED.has(m[1])) unresolved.push(`${relative(SRC, file)}: ${m[1]}`)
      }
    }
    expect(unresolved).toEqual([])
  })

  it('uses the type scale for every font-size', () => {
    const literals: string[] = []
    for (const file of FILES) {
      for (const m of styleBlocks(file).matchAll(/font-size:\s*([^;]+);/g)) {
        const value = m[1].trim()
        if (value.startsWith('var(--fs-') || value === 'inherit') continue
        literals.push(`${relative(SRC, file)}: ${value}`)
      }
    }
    expect(literals).toEqual([])
  })

  it('declares the full spacing scale', () => {
    for (let step = 1; step <= 10; step += 1) {
      expect(DEFINED.has(`--space-${step}`)).toBe(true)
    }
  })

  it('uses the spacing scale for every padding, margin and gap', () => {
    // A raw rem here is what turned one gap into 32 near-identical gaps. Other
    // units stay legal: a hairline is px, a centred block is auto, and a value
    // sized to its own text is em.
    const spacing =
      /(?<![-\w])((?:padding|margin|gap|row-gap|column-gap)(?:-[a-z-]+)?):\s*([^;]+);/g
    const literals: string[] = []
    for (const file of FILES) {
      for (const m of styleBlocks(file).matchAll(spacing)) {
        if (!/\d*\.?\d+rem/.test(m[2])) continue
        literals.push(`${relative(SRC, file)}: ${m[1]}: ${m[2].trim()}`)
      }
    }
    expect(literals).toEqual([])
  })

  it('uses the spacing scale in inline style attributes too', () => {
    // A `<style>` block is not the only place a spacing value can hide: an
    // inline `style=` on the element escapes the rule above entirely, which
    // is how a lone `margin-top: 0.75rem` survived the migration.
    const attrs = /:?style="([^"]*)"/g
    const spacing =
      /(?<![-\w])(?:padding|margin|gap|row-gap|column-gap)(?:-[a-z-]+)?:\s*[^;"]*?\d*\.?\d+rem/
    const literals: string[] = []
    for (const file of FILES) {
      const text = readFileSync(file, 'utf-8')
      for (const m of text.matchAll(attrs)) {
        if (spacing.test(m[1])) literals.push(`${relative(SRC, file)}: ${m[1].trim()}`)
      }
    }
    expect(literals).toEqual([])
  })

  it('uses radius tokens for every rounded corner', () => {
    const literals: string[] = []
    for (const file of FILES) {
      for (const m of styleBlocks(file).matchAll(/border-radius:\s*([^;]+);/g)) {
        // Shorthand corners are checked per corner: `0` and `50%` are a square
        // edge and a circle respectively, neither of which is a token.
        const corners = m[1].trim().split(/\s+/)
        const offScale = corners.filter(
          (c) => !c.startsWith('var(--radius') && c !== '50%' && c !== '0' && c !== 'inherit',
        )
        if (offScale.length > 0) literals.push(`${relative(SRC, file)}: ${m[1].trim()}`)
      }
    }
    expect(literals).toEqual([])
  })

  it('uses duration tokens for every transition', () => {
    const literals: string[] = []
    for (const file of FILES) {
      for (const m of styleBlocks(file).matchAll(/transition(?:-duration)?:\s*([^;{}]+);/g)) {
        // The reduced-motion override deliberately collapses every duration to
        // a near-zero literal; it is the one place a token would be wrong.
        if (m[1].includes('!important')) continue
        if (/(?<![\d.])\d*\.?\d+m?s/.test(m[1])) {
          literals.push(`${relative(SRC, file)}: ${m[1].trim().replace(/\s+/g, ' ')}`)
        }
      }
    }
    expect(literals).toEqual([])
  })

  it('restores a visible keyboard focus indicator globally', () => {
    const css = readFileSync(STYLE_CSS, 'utf-8')
    expect(css).toMatch(/:focus-visible\s*\{[^}]*outline:/)
  })

  it('honours prefers-reduced-motion', () => {
    const css = readFileSync(STYLE_CSS, 'utf-8')
    expect(css).toMatch(/@media\s*\(prefers-reduced-motion:\s*reduce\)/)
  })
})
