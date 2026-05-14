// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, onUnmounted } from 'vue'
import { logger } from '../utils/logger'

export type WsConnectionStatus = 'connected' | 'disconnected' | 'reconnecting'

type MessageCallback<T> = (data: T) => void

interface WsMessage<T = unknown> {
  type: string
  payload: T
}

interface UseWebSocketReturn {
  status: ReturnType<typeof ref<WsConnectionStatus>>
  onMessage: <T>(type: string, callback: MessageCallback<T>) => void
}

const MAX_BACKOFF_MS = 30_000

const status = ref<WsConnectionStatus>('disconnected')
const listeners = new Map<string, Set<MessageCallback<unknown>>>()

let socket: WebSocket | null = null
let backoffMs = 1_000

function buildUrl(): string {
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
  return `${proto}://${window.location.host}/ws/ui`
}

function connect(): void {
  socket = new WebSocket(buildUrl())

  socket.addEventListener('open', () => {
    status.value = 'connected'
    backoffMs = 1_000
  })

  socket.addEventListener('message', (event: MessageEvent<string>) => {
    let parsed: WsMessage
    try {
      parsed = JSON.parse(event.data) as WsMessage
    } catch (e: unknown) {
      logger.debug('ws: failed to parse message', e)
      return
    }

    const handlers = listeners.get(parsed.type)
    if (handlers) {
      handlers.forEach((cb) => cb(parsed.payload))
    }
  })

  socket.addEventListener('close', () => {
    socket = null
    scheduleReconnect()
  })

  socket.addEventListener('error', (ev) => {
    logger.debug('ws: connection error', ev)
    socket?.close()
  })
}

function scheduleReconnect(): void {
  status.value = 'reconnecting'
  setTimeout(() => {
    connect()
    backoffMs = Math.min(backoffMs * 2, MAX_BACKOFF_MS)
  }, backoffMs)
}

connect()

export function useWebSocket(): UseWebSocketReturn {
  const localHandlers: Array<{ type: string; cb: MessageCallback<unknown> }> = []

  function onMessage<T>(type: string, callback: MessageCallback<T>): void {
    if (!listeners.has(type)) {
      listeners.set(type, new Set())
    }
    const cb = callback as MessageCallback<unknown>
    listeners.get(type)!.add(cb)
    localHandlers.push({ type, cb })
  }

  onUnmounted(() => {
    for (const { type, cb } of localHandlers) {
      listeners.get(type)?.delete(cb)
    }
  })

  return { status, onMessage }
}
