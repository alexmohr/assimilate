// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref } from 'vue'
import { getSystemSettings } from '../api/system'
import { logger } from '../utils/logger'
import { readStorage, removeStorage, writeStorage } from '../utils/storage'

const STORAGE_KEY = 'assimilate-timezone'

const timezone = ref<string | undefined>(readStorage(STORAGE_KEY))

export function getConfiguredTimezone(): string | undefined {
  return timezone.value || Intl.DateTimeFormat().resolvedOptions().timeZone
}

export function useTimezone(): {
  timezone: typeof timezone
  setTimezone: (tz: string | undefined) => void
  loadFromBackend: () => Promise<void>
} {
  function setTimezone(tz: string | undefined): void {
    timezone.value = tz
    if (tz) {
      writeStorage(STORAGE_KEY, tz)
    } else {
      removeStorage(STORAGE_KEY)
    }
  }

  async function loadFromBackend(): Promise<void> {
    try {
      const settings = await getSystemSettings()
      const tz = settings.timezone || undefined
      setTimezone(tz)
    } catch (e: unknown) {
      logger.debug('timezone load failed', e)
    }
  }

  return { timezone, setTimezone, loadFromBackend }
}
