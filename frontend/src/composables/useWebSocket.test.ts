// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { mount } from '@vue/test-utils'
import type * as WsModuleExports from './useWebSocket'

vi.mock('../utils/logger', () => ({
  logger: { debug: vi.fn(), error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

/** Records every socket the module opens; nothing touches the network. */
class FakeWebSocket extends EventTarget {
  static instances: FakeWebSocket[] = []
  readonly url: string
  closed = false
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null

  constructor(url: string) {
    super()
    this.url = url
    FakeWebSocket.instances.push(this)
  }

  open(): void {
    this.dispatchEvent(new Event('open'))
  }

  receive(data: string): void {
    this.dispatchEvent(new MessageEvent('message', { data }))
  }

  fail(): void {
    this.dispatchEvent(new Event('error'))
  }

  close(): void {
    if (this.closed) return
    this.closed = true
    this.dispatchEvent(new Event('close'))
  }
}

type WsModule = typeof WsModuleExports

function latestSocket(): FakeWebSocket {
  const socket = FakeWebSocket.instances.at(-1)
  if (!socket) throw new Error('no socket was opened')
  return socket
}

/**
 * The `visibilitychange` listeners each fresh module copy adds to `document`,
 * removed after every test so an earlier copy cannot react to a later test.
 */
const visibilityListeners: EventListenerOrEventListenerObject[] = []

/** Imports a fresh copy of the module, whose import opens the first socket. */
async function loadModule(): Promise<WsModule> {
  vi.resetModules()
  const addListener = document.addEventListener.bind(document)
  const spy = vi
    .spyOn(document, 'addEventListener')
    .mockImplementation((type, listener, options) => {
      if (type === 'visibilitychange' && listener) visibilityListeners.push(listener)
      addListener(type, listener, options)
    })
  try {
    return await import('./useWebSocket')
  } finally {
    spy.mockRestore()
  }
}

/** Calls `useWebSocket` from a mounted component so its unmount hook works. */
function mountUser(mod: WsModule): {
  api: ReturnType<WsModule['useWebSocket']>
  unmount: () => void
} {
  let api: ReturnType<WsModule['useWebSocket']> | undefined
  const wrapper = mount(
    defineComponent({
      setup() {
        api = mod.useWebSocket()
        return () => h('div')
      },
    }),
  )
  if (!api) throw new Error('useWebSocket was not called')
  return { api, unmount: () => wrapper.unmount() }
}

/** Drops the current socket and checks the retry comes after exactly `ms`. */
function expectNextDropRetriesAfter(ms: number): void {
  const before = FakeWebSocket.instances.length
  latestSocket().close()
  vi.advanceTimersByTime(ms - 1)
  expect(FakeWebSocket.instances).toHaveLength(before)
  vi.advanceTimersByTime(1)
  expect(FakeWebSocket.instances).toHaveLength(before + 1)
}

function setVisibility(state: DocumentVisibilityState): void {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state })
  document.dispatchEvent(new Event('visibilitychange'))
}

describe('useWebSocket', () => {
  beforeEach(() => {
    FakeWebSocket.instances = []
    vi.stubGlobal('WebSocket', FakeWebSocket)
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
    visibilityListeners
      .splice(0)
      .forEach((listener) => document.removeEventListener('visibilitychange', listener))
  })

  it('connects to the UI endpoint on the current host when loaded', async () => {
    const mod = await loadModule()
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(latestSocket().url).toBe(`ws://${window.location.host}/ws/ui`)

    const { api } = mountUser(mod)
    expect(api.status.value).toBe('disconnected')
    latestSocket().open()
    expect(api.status.value).toBe('connected')
  })

  it('hands each message payload to the listeners of its type only', async () => {
    const mod = await loadModule()
    const { api } = mountUser(mod)
    const progress = vi.fn()
    const other = vi.fn()
    api.onMessage('backup_progress', progress)
    api.onMessage('agent_status', other)

    latestSocket().receive(JSON.stringify({ type: 'backup_progress', payload: { pct: 42 } }))
    latestSocket().receive(JSON.stringify({ type: 'nobody_listens', payload: 1 }))
    latestSocket().receive('not json')

    expect(progress).toHaveBeenCalledExactlyOnceWith({ pct: 42 })
    expect(other).not.toHaveBeenCalled()
  })

  it('stops delivering to a component once it unmounts', async () => {
    const mod = await loadModule()
    const first = mountUser(mod)
    const second = mountUser(mod)
    const gone = vi.fn()
    const stays = vi.fn()
    first.api.onMessage('agent_status', gone)
    second.api.onMessage('agent_status', stays)

    first.unmount()
    latestSocket().receive(JSON.stringify({ type: 'agent_status', payload: 'up' }))

    expect(gone).not.toHaveBeenCalled()
    expect(stays).toHaveBeenCalledExactlyOnceWith('up')
  })

  it('reconnects with a doubling backoff that a successful open resets', async () => {
    const mod = await loadModule()
    const { api } = mountUser(mod)

    latestSocket().close()
    expect(api.status.value).toBe('reconnecting')
    vi.advanceTimersByTime(999)
    expect(FakeWebSocket.instances).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(2)

    // The next attempt waits twice as long.
    latestSocket().close()
    vi.advanceTimersByTime(1_999)
    expect(FakeWebSocket.instances).toHaveLength(2)
    vi.advanceTimersByTime(1)
    expect(FakeWebSocket.instances).toHaveLength(3)

    // Once a connection succeeds, the next drop starts from one second again.
    latestSocket().open()
    expect(api.status.value).toBe('connected')
    latestSocket().close()
    vi.advanceTimersByTime(1_000)
    expect(FakeWebSocket.instances).toHaveLength(4)
  })

  it('caps the backoff at thirty seconds', async () => {
    await loadModule()
    // 1s, 2s, 4s, 8s, 16s: each failed attempt doubles the wait...
    for (const wait of [1_000, 2_000, 4_000, 8_000, 16_000]) {
      latestSocket().close()
      vi.advanceTimersByTime(wait)
    }
    expect(FakeWebSocket.instances).toHaveLength(6)
    // ...until it would pass 30s, where it stays.
    for (let attempt = 0; attempt < 2; attempt++) {
      latestSocket().close()
      vi.advanceTimersByTime(29_999)
      const before = FakeWebSocket.instances.length
      vi.advanceTimersByTime(1)
      expect(FakeWebSocket.instances).toHaveLength(before + 1)
    }
  })

  it('treats a socket error as a dropped connection', async () => {
    const mod = await loadModule()
    const { api } = mountUser(mod)
    const socket = latestSocket()

    socket.fail()
    expect(socket.closed).toBe(true)
    expect(api.status.value).toBe('reconnecting')
    vi.advanceTimersByTime(1_000)
    expect(FakeWebSocket.instances).toHaveLength(2)
  })

  it('forces a reconnect by closing a live socket, or by connecting when there is none', async () => {
    const mod = await loadModule()
    const { api } = mountUser(mod)
    const socket = latestSocket()

    api.forceReconnect()
    expect(socket.closed).toBe(true)
    expect(api.status.value).toBe('reconnecting')
    expect(FakeWebSocket.instances).toHaveLength(1)

    // Between the drop and the scheduled retry there is no socket to close.
    api.forceReconnect()
    expect(FakeWebSocket.instances).toHaveLength(2)

    // The retry that was pending must not open a third socket as well...
    vi.advanceTimersByTime(30_000)
    expect(FakeWebSocket.instances).toHaveLength(2)
    // ...nor have doubled the backoff the next drop starts from.
    expectNextDropRetriesAfter(1_000)
  })

  it('reconnects at once when a disconnected tab becomes visible again', async () => {
    const mod = await loadModule()
    const { api } = mountUser(mod)
    latestSocket().close()
    expect(FakeWebSocket.instances).toHaveLength(1)

    setVisibility('visible')
    expect(FakeWebSocket.instances).toHaveLength(2)
    expect(api.status.value).toBe('reconnecting')

    // The retry that was pending was cancelled rather than run as well.
    vi.advanceTimersByTime(1_000)
    expect(FakeWebSocket.instances).toHaveLength(2)
  })

  it('replaces a socket that never opened when the tab becomes visible', async () => {
    await loadModule()
    const stale = latestSocket()

    setVisibility('visible')
    expect(stale.closed).toBe(true)
    expect(FakeWebSocket.instances).toHaveLength(2)
    const current = latestSocket()
    expect(current).not.toBe(stale)

    // The stale socket's close must not schedule a retry of its own.
    vi.advanceTimersByTime(30_000)
    expect(FakeWebSocket.instances).toHaveLength(2)
    // A browser delivers that socket's events later still; they must not
    // touch the socket that replaced it.
    stale.fail()
    stale.dispatchEvent(new Event('close'))
    vi.advanceTimersByTime(30_000)
    expect(current.closed).toBe(false)
    expect(FakeWebSocket.instances).toHaveLength(2)
    // And the replacement's own drop still retries on the reset backoff.
    expectNextDropRetriesAfter(1_000)
  })

  it('leaves a connected socket alone when the tab becomes visible', async () => {
    await loadModule()
    latestSocket().open()

    setVisibility('visible')
    expect(FakeWebSocket.instances).toHaveLength(1)
  })
})
