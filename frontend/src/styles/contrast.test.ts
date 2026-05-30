// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

type RGB = [number, number, number]

interface ColorPair {
  name: string
  foreground: RGB
  background: RGB
}

function relativeLuminance(r: number, g: number, b: number): number {
  const linearize = (c: number): number => {
    const sRGB = c / 255
    return sRGB <= 0.04045 ? sRGB / 12.92 : Math.pow((sRGB + 0.055) / 1.055, 2.4)
  }
  return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)
}

function contrastRatio(color1: RGB, color2: RGB): number {
  const l1 = relativeLuminance(...color1)
  const l2 = relativeLuminance(...color2)
  const lighter = Math.max(l1, l2)
  const darker = Math.min(l1, l2)
  return (lighter + 0.05) / (darker + 0.05)
}

function hex(h: string): RGB {
  const clean = h.replace('#', '')
  return [
    parseInt(clean.slice(0, 2), 16),
    parseInt(clean.slice(2, 4), 16),
    parseInt(clean.slice(4, 6), 16),
  ]
}

// Color tokens extracted from frontend/src/style.css :root (light theme)
const light = {
  bgBase: hex('#ffffff'),
  bgCard: hex('#f8fafc'),
  bgSidebar: hex('#f1f5f9'),
  bgInput: hex('#ffffff'),
  bgHover: hex('#f1f5f9'),
  bgElevated: hex('#ffffff'),
  textPrimary: hex('#0f172a'),
  textSecondary: hex('#475569'),
  textMuted: hex('#94a3b8'),
  accent: hex('#1d4ed8'),
  textOnAccent: hex('#ffffff'),
  danger: hex('#dc2626'),
  success: hex('#16a34a'),
  warning: hex('#d97706'),
  info: hex('#0ea5e9'),
}

// Color tokens extracted from frontend/src/style.css .dark (dark theme)
const dark = {
  bgBase: hex('#111113'),
  bgCard: hex('#18181b'),
  bgSidebar: hex('#131315'),
  bgInput: hex('#111113'),
  bgHover: hex('#202023'),
  bgElevated: hex('#1c1c1f'),
  textPrimary: hex('#f0f0f2'),
  textSecondary: hex('#b4b4bc'),
  textMuted: hex('#76767e'),
  accent: hex('#3b82f6'),
  textOnAccent: hex('#ffffff'),
  danger: hex('#f87171'),
  success: hex('#4ade80'),
  warning: hex('#fbbf24'),
  info: hex('#38bdf8'),
}

const WCAG_AA = 4.5

const lightPairs: ColorPair[] = [
  { name: 'light: primary text on base', foreground: light.textPrimary, background: light.bgBase },
  { name: 'light: primary text on card', foreground: light.textPrimary, background: light.bgCard },
  {
    name: 'light: primary text on sidebar',
    foreground: light.textPrimary,
    background: light.bgSidebar,
  },
  {
    name: 'light: primary text on elevated',
    foreground: light.textPrimary,
    background: light.bgElevated,
  },
  {
    name: 'light: secondary text on base',
    foreground: light.textSecondary,
    background: light.bgBase,
  },
  {
    name: 'light: secondary text on card',
    foreground: light.textSecondary,
    background: light.bgCard,
  },
  {
    name: 'light: secondary text on sidebar',
    foreground: light.textSecondary,
    background: light.bgSidebar,
  },
  {
    name: 'light: text-on-accent on accent bg',
    foreground: light.textOnAccent,
    background: light.accent,
  },
  {
    name: 'light: text-on-accent on danger bg',
    foreground: light.textOnAccent,
    background: light.danger,
  },
  { name: 'light: accent on base', foreground: light.accent, background: light.bgBase },
  { name: 'light: accent on card', foreground: light.accent, background: light.bgCard },
  { name: 'light: danger on base', foreground: light.danger, background: light.bgBase },
  {
    name: 'light: primary text on input',
    foreground: light.textPrimary,
    background: light.bgInput,
  },
  {
    name: 'light: primary text on hover',
    foreground: light.textPrimary,
    background: light.bgHover,
  },
]

const darkPairs: ColorPair[] = [
  { name: 'dark: primary text on base', foreground: dark.textPrimary, background: dark.bgBase },
  { name: 'dark: primary text on card', foreground: dark.textPrimary, background: dark.bgCard },
  {
    name: 'dark: primary text on sidebar',
    foreground: dark.textPrimary,
    background: dark.bgSidebar,
  },
  {
    name: 'dark: primary text on elevated',
    foreground: dark.textPrimary,
    background: dark.bgElevated,
  },
  {
    name: 'dark: secondary text on base',
    foreground: dark.textSecondary,
    background: dark.bgBase,
  },
  {
    name: 'dark: secondary text on card',
    foreground: dark.textSecondary,
    background: dark.bgCard,
  },
  {
    name: 'dark: secondary text on sidebar',
    foreground: dark.textSecondary,
    background: dark.bgSidebar,
  },
  { name: 'dark: accent on base', foreground: dark.accent, background: dark.bgBase },
  { name: 'dark: accent on card', foreground: dark.accent, background: dark.bgCard },
  { name: 'dark: danger on base', foreground: dark.danger, background: dark.bgBase },
  { name: 'dark: success on base', foreground: dark.success, background: dark.bgBase },
  { name: 'dark: primary text on input', foreground: dark.textPrimary, background: dark.bgInput },
  { name: 'dark: primary text on hover', foreground: dark.textPrimary, background: dark.bgHover },
]

const knownViolations: ColorPair[] = [
  {
    name: 'VIOLATION light: success (#16a34a) on base (#ffffff) — ~3.3:1, fix: darken --success to #15803d',
    foreground: light.success,
    background: light.bgBase,
  },
  {
    name: 'VIOLATION dark: white text on accent (#3b82f6) — ~3.7:1, fix: darken --accent or use dark text',
    foreground: dark.textOnAccent,
    background: dark.accent,
  },
  {
    name: 'VIOLATION dark: white text on danger (#f87171) — ~2.8:1, fix: darken --danger or use dark text',
    foreground: dark.textOnAccent,
    background: dark.danger,
  },
]

describe('contrastRatio utility', () => {
  it('returns 21:1 for black on white', () => {
    const ratio = contrastRatio([0, 0, 0], [255, 255, 255])
    expect(ratio).toBeCloseTo(21, 0)
  })

  it('returns 1:1 for same color', () => {
    const ratio = contrastRatio([128, 64, 200], [128, 64, 200])
    expect(ratio).toBeCloseTo(1, 5)
  })

  it('is symmetric', () => {
    const a: RGB = [30, 30, 30]
    const b: RGB = [200, 200, 200]
    expect(contrastRatio(a, b)).toBeCloseTo(contrastRatio(b, a), 10)
  })
})

describe('WCAG AA contrast >= 4.5:1 — light theme', () => {
  for (const pair of lightPairs) {
    it(pair.name, () => {
      const ratio = contrastRatio(pair.foreground, pair.background)
      expect(
        ratio,
        `${pair.name}: ratio ${ratio.toFixed(2)}:1 < required ${WCAG_AA}:1`,
      ).toBeGreaterThanOrEqual(WCAG_AA)
    })
  }
})

describe('WCAG AA contrast >= 4.5:1 — dark theme', () => {
  for (const pair of darkPairs) {
    it(pair.name, () => {
      const ratio = contrastRatio(pair.foreground, pair.background)
      expect(
        ratio,
        `${pair.name}: ratio ${ratio.toFixed(2)}:1 < required ${WCAG_AA}:1`,
      ).toBeGreaterThanOrEqual(WCAG_AA)
    })
  }
})

describe('known WCAG AA violations — tracked bugs, fix --success and dark --accent/--danger', () => {
  for (const pair of knownViolations) {
    it.fails(pair.name, () => {
      const ratio = contrastRatio(pair.foreground, pair.background)
      expect(
        ratio,
        `${pair.name}: ratio ${ratio.toFixed(2)}:1 < required ${WCAG_AA}:1`,
      ).toBeGreaterThanOrEqual(WCAG_AA)
    })
  }
})
