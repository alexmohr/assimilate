// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { firstErrorStack, formatLogArgs, redactText } from './redact'

/**
 * In-browser log capture: a bounded ring buffer of what the frontend logged,
 * shown on the Activity page's Browser logs tab.
 *
 * Every entry is redacted when it is recorded (see `redact.ts`), so the
 * buffer never holds a secret, and it holds strings only - never a reference
 * to the logged object - so a logged component, response or DOM node is not
 * kept alive by it.
 */

export const CLIENT_LOG_LEVELS = ['error', 'warn', 'debug'] as const
export type ClientLogLevel = (typeof CLIENT_LOG_LEVELS)[number]

export const DEFAULT_CLIENT_LOG_CAPACITY = 500
export const MAX_CLIENT_LOG_CAPACITY = 10_000

export interface ClientLogEntry {
  /** Monotonic per buffer; stable across evictions, so usable as a key. */
  id: number
  /** ISO 8601. */
  timestamp: string
  level: ClientLogLevel
  message: string
  /** `function (file:line:col)` of the logger's caller, or `''` when unknown. */
  source: string
  /** Redacted stack of the first `Error` argument, if there was one. */
  stack: string | null
}

export type ClientLogListener = () => void

function clampCapacity(capacity: number): number {
  if (!Number.isFinite(capacity)) return DEFAULT_CLIENT_LOG_CAPACITY
  return Math.min(MAX_CLIENT_LOG_CAPACITY, Math.max(1, Math.floor(capacity)))
}

export class ClientLogBuffer {
  private slots: (ClientLogEntry | undefined)[]
  private start = 0
  private count = 0
  private nextId = 1
  private readonly listeners = new Set<ClientLogListener>()
  private notifying = false

  constructor(capacity: number = DEFAULT_CLIENT_LOG_CAPACITY) {
    this.slots = new Array<ClientLogEntry | undefined>(clampCapacity(capacity))
  }

  get capacity(): number {
    return this.slots.length
  }

  get size(): number {
    return this.count
  }

  /** Resizes the buffer, keeping the newest entries that still fit. */
  setCapacity(capacity: number): void {
    const next = clampCapacity(capacity)
    if (next === this.slots.length) return
    const kept = this.entries().slice(-next)
    this.slots = new Array<ClientLogEntry | undefined>(next)
    kept.forEach((entry, i) => {
      this.slots[i] = entry
    })
    this.start = 0
    this.count = kept.length
    this.notify()
  }

  /** Records one logger call. `args` are redacted here and not retained. */
  record(level: ClientLogLevel, args: readonly unknown[], source = ''): ClientLogEntry {
    const entry: ClientLogEntry = {
      id: this.nextId++,
      timestamp: new Date().toISOString(),
      level,
      message: formatLogArgs(args),
      source: redactText(source),
      stack: firstErrorStack(args),
    }
    const capacity = this.slots.length
    if (this.count < capacity) {
      this.slots[(this.start + this.count) % capacity] = entry
      this.count++
    } else {
      this.slots[this.start] = entry
      this.start = (this.start + 1) % capacity
    }
    this.notify()
    return entry
  }

  /** Oldest first. A copy: mutating it does not touch the buffer. */
  entries(): ClientLogEntry[] {
    const out: ClientLogEntry[] = []
    const capacity = this.slots.length
    for (let i = 0; i < this.count; i++) {
      const entry = this.slots[(this.start + i) % capacity]
      if (entry) out.push(entry)
    }
    return out
  }

  clear(): void {
    this.slots = new Array<ClientLogEntry | undefined>(this.slots.length)
    this.start = 0
    this.count = 0
    this.notify()
  }

  /** Calls `listener` after every change. Returns the unsubscribe function. */
  subscribe(listener: ClientLogListener): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  get listenerCount(): number {
    return this.listeners.size
  }

  private notify(): void {
    // A listener that itself logs would otherwise recurse without bound.
    if (this.notifying) return
    this.notifying = true
    try {
      for (const listener of this.listeners) {
        try {
          listener()
        } catch {
          // A broken listener must not break logging, and logging the
          // failure from here would re-enter this loop.
        }
      }
    } finally {
      this.notifying = false
    }
  }
}

/** The app-wide buffer the logger records into. */
export const clientLogBuffer = new ClientLogBuffer()

const FRAME_PATTERN = /^\s*at\s+|@/

/**
 * The frame `skip` levels above the caller of this function, from a stack in
 * either the V8 (`at fn (url:1:2)`) or the Gecko/WebKit (`fn@url:1:2`)
 * format. The page origin is dropped, since it is the same for every frame.
 */
export function callerFrame(stack: string | undefined, skip: number, origin = ''): string {
  if (!stack) return ''
  const frames = stack.split('\n').filter((line) => FRAME_PATTERN.test(line))
  const frame = frames[skip]
  if (!frame) return ''
  let text = frame.trim().replace(/^at\s+/, '')
  if (origin) text = text.split(origin).join('')
  return text
}

/** `[time] LEVEL source: message` lines plus stacks, for the clipboard. */
export function formatClientLogs(entries: readonly ClientLogEntry[]): string {
  return entries
    .map((entry) => {
      const head = `[${entry.timestamp}] ${entry.level.toUpperCase()}${entry.source ? ` ${entry.source}` : ''}: ${entry.message}`
      return entry.stack ? `${head}\n${entry.stack}` : head
    })
    .join('\n')
}

interface ErrorEventTarget {
  addEventListener(type: 'error', listener: (event: ErrorEvent) => void): void
  addEventListener(
    type: 'unhandledrejection',
    listener: (event: PromiseRejectionEvent) => void,
  ): void
  removeEventListener(type: 'error', listener: (event: ErrorEvent) => void): void
  removeEventListener(
    type: 'unhandledrejection',
    listener: (event: PromiseRejectionEvent) => void,
  ): void
}

/**
 * Records uncaught errors and unhandled promise rejections into `buffer`.
 * The browser already prints both to the console, so they are not forwarded
 * again. Returns the function that removes both listeners.
 */
export function captureGlobalErrors(
  target: ErrorEventTarget,
  buffer: ClientLogBuffer = clientLogBuffer,
): () => void {
  const onError = (event: ErrorEvent): void => {
    const location = event.filename ? `${event.filename}:${event.lineno}:${event.colno}` : ''
    buffer.record('error', ['Uncaught', event.error ?? event.message], location)
  }
  const onRejection = (event: PromiseRejectionEvent): void => {
    buffer.record('error', ['Unhandled rejection', event.reason])
  }
  target.addEventListener('error', onError)
  target.addEventListener('unhandledrejection', onRejection)
  return () => {
    target.removeEventListener('error', onError)
    target.removeEventListener('unhandledrejection', onRejection)
  }
}
