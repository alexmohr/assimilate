// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { type Ref, ref, onMounted, onUnmounted } from 'vue'

const BREAKPOINT_SM = 640
const BREAKPOINT_MD = 768
const BREAKPOINT_LG = 1024

export type ScreenSize = 'sm' | 'md' | 'lg' | 'xl'

export function useMobile(): {
  isMobile: Ref<boolean>
  isTablet: Ref<boolean>
  screenSize: Ref<ScreenSize>
} {
  const isMobile = ref(false)
  const isTablet = ref(false)
  const screenSize = ref<ScreenSize>('xl')

  let rafId: number | null = null

  function check(): void {
    const w = window.innerWidth
    isMobile.value = w < BREAKPOINT_MD
    isTablet.value = w >= BREAKPOINT_SM && w < BREAKPOINT_LG

    if (w < BREAKPOINT_SM) screenSize.value = 'sm'
    else if (w < BREAKPOINT_MD) screenSize.value = 'md'
    else if (w < BREAKPOINT_LG) screenSize.value = 'lg'
    else screenSize.value = 'xl'
  }

  function onResize(): void {
    if (rafId !== null) return
    rafId = requestAnimationFrame(() => {
      check()
      rafId = null
    })
  }

  onMounted(() => {
    check()
    window.addEventListener('resize', onResize, { passive: true })
  })

  onUnmounted(() => {
    window.removeEventListener('resize', onResize)
    if (rafId !== null) cancelAnimationFrame(rafId)
  })

  return { isMobile, isTablet, screenSize }
}
