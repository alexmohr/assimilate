// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineStore } from 'pinia'
import { ref, watch } from 'vue'
import { readStorage, writeStorage } from '../utils/storage'

const STORAGE_KEY = 'assimilate-sidebar-collapsed'

export const useUiStore = defineStore('ui', () => {
  const sidebarCollapsed = ref(readStorage(STORAGE_KEY) === 'true')
  const sidebarMobileOpen = ref(false)

  watch(sidebarMobileOpen, (open) => {
    document.documentElement.style.overflow = open ? 'hidden' : ''
  })

  function toggleSidebar(): void {
    sidebarCollapsed.value = !sidebarCollapsed.value
    writeStorage(STORAGE_KEY, String(sidebarCollapsed.value))
  }

  function openMobileSidebar(): void {
    sidebarMobileOpen.value = true
  }

  function closeMobileSidebar(): void {
    sidebarMobileOpen.value = false
  }

  return {
    sidebarCollapsed,
    sidebarMobileOpen,
    toggleSidebar,
    openMobileSidebar,
    closeMobileSidebar,
  }
})
