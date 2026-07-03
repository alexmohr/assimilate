// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, watch } from 'vue'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'
import { readStorage, writeStorage } from '../utils/storage'

export type Theme = 'light' | 'dark' | 'auto'
type ResolvedTheme = 'light' | 'dark'

const STORAGE_KEY = 'theme'

function getSystemPreference(): ResolvedTheme {
  if (window.matchMedia('(prefers-color-scheme: dark)').matches) {
    return 'dark'
  }
  return 'light'
}

function isTheme(v: string): v is Theme {
  return v === 'light' || v === 'dark' || v === 'auto'
}

function getStoredTheme(): Theme | null {
  const stored = readStorage(STORAGE_KEY)
  return stored !== undefined && isTheme(stored) ? stored : null
}

function resolveTheme(t: Theme): ResolvedTheme {
  if (t === 'auto') {
    return getSystemPreference()
  }
  return t
}

function applyTheme(t: ResolvedTheme): void {
  const html = document.documentElement
  if (t === 'dark') {
    html.classList.add('dark')
  } else {
    html.classList.remove('dark')
  }
}

const theme = ref<Theme>(getStoredTheme() ?? 'auto')
applyTheme(resolveTheme(theme.value))

const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
mediaQuery.addEventListener('change', () => {
  if (theme.value === 'auto') {
    applyTheme(getSystemPreference())
  }
})

let syncing = false

async function syncToBackend(t: Theme): Promise<void> {
  try {
    await apiClient.put('/auth/preferences', { theme: t })
  } catch (e: unknown) {
    logger.debug('theme sync failed', e)
  }
}

watch(theme, (val) => {
  if (!syncing) {
    syncToBackend(val).catch(logger.debug)
  }
})

export function useTheme(): {
  theme: typeof theme
  setTheme: (t: Theme) => void
  loadFromBackend: () => Promise<void>
} {
  function setTheme(t: Theme): void {
    theme.value = t
    writeStorage(STORAGE_KEY, t)
    applyTheme(resolveTheme(t))
  }

  async function loadFromBackend(): Promise<void> {
    try {
      const res = await apiClient.get<{ theme?: string }>('/auth/preferences')
      const backendTheme = res.data?.theme
      if (backendTheme !== undefined && isTheme(backendTheme)) {
        syncing = true
        theme.value = backendTheme
        writeStorage(STORAGE_KEY, backendTheme)
        applyTheme(resolveTheme(backendTheme))
        syncing = false
      }
    } catch (e: unknown) {
      logger.debug('loadFromBackend: preferences fetch failed', e)
    }
  }

  return { theme, setTheme, loadFromBackend }
}
