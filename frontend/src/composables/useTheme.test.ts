// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UserPreferences } from '../types/generated'
import type { useTheme } from './useTheme'

const getPreferences = vi.fn<() => Promise<UserPreferences>>()

vi.mock('../api/auth', () => ({
  getPreferences: (): Promise<UserPreferences> => getPreferences(),
  updatePreferences: vi.fn().mockResolvedValue(undefined),
}))

// The theme lives in module state, so each test loads a fresh copy of the
// module with nothing stored.
async function loadTheme(): Promise<ReturnType<typeof useTheme>> {
  vi.resetModules()
  const module = await import('./useTheme')
  return module.useTheme()
}

describe('useTheme loadFromBackend', () => {
  beforeEach(() => {
    localStorage.clear()
    document.documentElement.classList.remove('dark', 'theme-charcoal', 'theme-warm')
    getPreferences.mockReset()
  })

  it('applies a theme stored in the backend preferences', async () => {
    getPreferences.mockResolvedValue({ theme: 'dark' })
    const { theme, loadFromBackend } = await loadTheme()

    await loadFromBackend()

    expect(theme.value).toBe('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it.each([
    ['dark', ['dark'], ['theme-charcoal', 'theme-warm']],
    ['charcoal', ['dark', 'theme-charcoal'], ['theme-warm']],
    ['warm', ['dark', 'theme-warm'], ['theme-charcoal']],
    ['light', [], ['dark', 'theme-charcoal', 'theme-warm']],
  ] as const)('sets exactly the classes of the %s theme', async (name, present, absent) => {
    getPreferences.mockResolvedValue({ theme: name })
    const { theme, loadFromBackend } = await loadTheme()

    await loadFromBackend()

    expect(theme.value).toBe(name)
    const classes = document.documentElement.classList
    for (const cls of present) expect(classes.contains(cls), cls).toBe(true)
    for (const cls of absent) expect(classes.contains(cls), cls).toBe(false)
  })

  it('drops the previous theme class when switching themes', async () => {
    const { setTheme } = await loadTheme()

    setTheme('charcoal')
    setTheme('warm')

    const classes = document.documentElement.classList
    expect(classes.contains('theme-warm')).toBe(true)
    expect(classes.contains('theme-charcoal')).toBe(false)
  })

  it('keeps the current theme when the backend has none stored', async () => {
    getPreferences.mockResolvedValue({})
    const { theme, loadFromBackend } = await loadTheme()

    await loadFromBackend()

    expect(theme.value).toBe('auto')
  })

  it('keeps the current theme when the preferences cannot be loaded', async () => {
    getPreferences.mockRejectedValue(new Error('offline'))
    const { theme, loadFromBackend } = await loadTheme()

    await loadFromBackend()

    expect(theme.value).toBe('auto')
  })
})
