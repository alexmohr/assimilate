// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import type { RouteMeta } from 'vue-router'
import { getSystemMode } from '../api/system'
import type { DeploymentMode } from '../types/generated'
import { logger } from '../utils/logger'

/**
 * How the server is deployed. Only decides which pages the UI offers: the
 * server enforces permissions the same way in every mode, so a page hidden
 * here is a convenience, never a security boundary.
 */
export const useSystemModeStore = defineStore('systemMode', () => {
  const mode = ref<DeploymentMode | null>(null)
  const isDesktop = computed(() => mode.value === 'desktop')
  let pending: Promise<void> | null = null

  // A failed lookup leaves the mode unknown, which shows every page - the
  // server-mode UI - and lets the next call try again.
  function load(): Promise<void> {
    if (mode.value !== null) return Promise.resolve()
    pending ??= getSystemMode()
      .then((response) => {
        mode.value = response.mode
      })
      .catch((err: unknown) => {
        logger.warn('failed to load deployment mode', err)
      })
      .finally(() => {
        pending = null
      })
    return pending
  }

  /** Whether a route marked `serverOnly` should be hidden in this mode. */
  function hidesRoute(meta: RouteMeta): boolean {
    return isDesktop.value && meta.serverOnly === true
  }

  return { mode, isDesktop, load, hidesRoute }
})
