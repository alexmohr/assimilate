// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { defineComponent, h, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { useChartTheme } from './useChartTheme'

function mountTheme(): {
  wrapper: ReturnType<typeof mount>
  textMuted: ReturnType<typeof useChartTheme>['textMuted']
  border: ReturnType<typeof useChartTheme>['border']
} {
  let theme: ReturnType<typeof useChartTheme> | undefined
  const wrapper = mount(
    defineComponent({
      setup() {
        theme = useChartTheme()
        return () => h('div')
      },
    }),
  )
  const { textMuted, border } = theme!
  return { wrapper, textMuted, border }
}

describe('useChartTheme', () => {
  it('reads the current CSS custom properties', () => {
    document.documentElement.style.setProperty('--text-muted', '#111')
    document.documentElement.style.setProperty('--border', '#222')

    const { textMuted, border } = mountTheme()

    expect(textMuted.value).toBe('#111')
    expect(border.value).toBe('#222')
  })

  it('recomputes when the <html> class attribute changes (dark-mode toggle)', async () => {
    document.documentElement.style.setProperty('--text-muted', '#light')
    const { textMuted } = mountTheme()
    expect(textMuted.value).toBe('#light')

    document.documentElement.style.setProperty('--text-muted', '#dark')
    document.documentElement.classList.add('dark')
    await nextTick()
    // MutationObserver callbacks are microtasks; give one more tick to flush.
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(textMuted.value).toBe('#dark')
    document.documentElement.classList.remove('dark')
  })

  it('stops observing after unmount', async () => {
    document.documentElement.style.setProperty('--text-muted', '#before')
    const { wrapper, textMuted } = mountTheme()
    expect(textMuted.value).toBe('#before')
    wrapper.unmount()

    document.documentElement.style.setProperty('--text-muted', '#after')
    document.documentElement.classList.add('dark')
    await new Promise((resolve) => setTimeout(resolve, 0))
    document.documentElement.classList.remove('dark')

    expect(textMuted.value).toBe('#before')
  })
})
