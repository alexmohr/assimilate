// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { PreferencesResponse } from '../types/generated'
import type { useTheme } from './useTheme'

const getPreferences = vi.fn<() => Promise<PreferencesResponse>>()

vi.mock('../api/auth', () => ({
  getPreferences: (): Promise<PreferencesResponse> => getPreferences(),
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
    document.documentElement.classList.remove('dark')
    getPreferences.mockReset()
  })

  it('applies a theme stored in the backend preferences', async () => {
    getPreferences.mockResolvedValue({ theme: 'dark' })
    const { theme, loadFromBackend } = await loadTheme()

    await loadFromBackend()

    expect(theme.value).toBe('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it.each<[string, PreferencesResponse]>([
    ['no preferences', null],
    ['no theme', {}],
    ['a theme that is not a string', { theme: 1 }],
    ['an unknown theme name', { theme: 'sepia' }],
  ])('keeps the current theme when the backend has %s', async (_, preferences) => {
    getPreferences.mockResolvedValue(preferences)
    const { theme, loadFromBackend } = await loadTheme()

    await loadFromBackend()

    expect(theme.value).toBe('auto')
  })
})
