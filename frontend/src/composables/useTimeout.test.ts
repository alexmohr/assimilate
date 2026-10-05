// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, effectScope, h } from 'vue'
import { mount } from '@vue/test-utils'
import { useTimeout, type UseTimeoutReturn } from './useTimeout'

function mountTimeout(): { wrapper: ReturnType<typeof mount>; timeout: UseTimeoutReturn } {
  let timeout: UseTimeoutReturn | undefined
  const wrapper = mount(
    defineComponent({
      setup() {
        timeout = useTimeout()
        return () => h('div')
      },
    }),
  )
  return { wrapper, timeout: timeout! }
}

describe('useTimeout', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('runs the callback once the delay has passed', () => {
    const { timeout } = mountTimeout()
    const cb = vi.fn()
    timeout.start(cb, 300)

    vi.advanceTimersByTime(299)
    expect(cb).not.toHaveBeenCalled()
    expect(timeout.isPending()).toBe(true)

    vi.advanceTimersByTime(1)
    expect(cb).toHaveBeenCalledTimes(1)
    expect(timeout.isPending()).toBe(false)
  })

  it('replaces a pending callback when started again, like a debounce', () => {
    const { timeout } = mountTimeout()
    const first = vi.fn()
    const second = vi.fn()
    timeout.start(first, 300)
    vi.advanceTimersByTime(200)
    timeout.start(second, 300)
    vi.advanceTimersByTime(299)
    expect(first).not.toHaveBeenCalled()
    expect(second).not.toHaveBeenCalled()

    vi.advanceTimersByTime(1)
    expect(first).not.toHaveBeenCalled()
    expect(second).toHaveBeenCalledTimes(1)
  })

  it('cancels a pending callback on clear', () => {
    const { timeout } = mountTimeout()
    const cb = vi.fn()
    timeout.start(cb, 100)
    timeout.clear()
    vi.advanceTimersByTime(1000)
    expect(cb).not.toHaveBeenCalled()
    expect(timeout.isPending()).toBe(false)
  })

  it('cancels a pending callback when the component unmounts', () => {
    const { wrapper, timeout } = mountTimeout()
    const cb = vi.fn()
    timeout.start(cb, 100)
    wrapper.unmount()
    vi.advanceTimersByTime(1000)
    expect(cb).not.toHaveBeenCalled()
  })

  it('cancels a pending callback when its effect scope stops', () => {
    const scope = effectScope()
    const timeout = scope.run(() => useTimeout())!
    const cb = vi.fn()
    timeout.start(cb, 100)
    scope.stop()
    vi.advanceTimersByTime(1000)
    expect(cb).not.toHaveBeenCalled()
  })

  it('still works outside any scope, leaving cleanup to the caller', () => {
    const timeout = useTimeout()
    const cb = vi.fn()
    timeout.start(cb, 100)
    vi.advanceTimersByTime(100)
    expect(cb).toHaveBeenCalledTimes(1)
  })
})
