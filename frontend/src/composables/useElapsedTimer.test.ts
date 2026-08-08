// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { defineComponent, h, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { useElapsedClock } from './useElapsedTimer'

function mountClock(initialActive: boolean) {
  const active = ref(initialActive)
  let now: ReturnType<typeof useElapsedClock>['now'] | undefined
  const wrapper = mount(
    defineComponent({
      setup() {
        ;({ now } = useElapsedClock(active))
        return () => h('div')
      },
    }),
  )
  return { wrapper, active, getNow: () => now! }
}

describe('useElapsedClock', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('does not tick while inactive', () => {
    const { getNow } = mountClock(false)
    const initial = getNow().value
    vi.advanceTimersByTime(3000)
    expect(getNow().value).toBe(initial)
  })

  it('ticks once a second while active', () => {
    const { getNow } = mountClock(true)
    const initial = getNow().value
    vi.advanceTimersByTime(1000)
    expect(getNow().value).toBeGreaterThan(initial)
  })

  it('starts ticking when active flips true, and stops when it flips back to false', async () => {
    const { active, getNow } = mountClock(false)
    const beforeStart = getNow().value
    vi.advanceTimersByTime(2000)
    expect(getNow().value).toBe(beforeStart)

    active.value = true
    await nextTick()
    vi.advanceTimersByTime(1000)
    const afterOneTick = getNow().value
    expect(afterOneTick).toBeGreaterThan(beforeStart)

    active.value = false
    await nextTick()
    vi.advanceTimersByTime(3000)
    expect(getNow().value).toBe(afterOneTick)
  })

  it('stops the timer on unmount', () => {
    const clearIntervalSpy = vi.spyOn(globalThis, 'clearInterval')
    const { wrapper } = mountClock(true)

    wrapper.unmount()

    expect(clearIntervalSpy).toHaveBeenCalled()
    clearIntervalSpy.mockRestore()
  })
})
