// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { callerFrame, clientLogBuffer, type ClientLogLevel } from './clientLog'

type LogFn = (...args: unknown[]) => void

/**
 * Frames between `new Error()` in `record` and the code that called the
 * logger: `record` itself, then the level wrapper below.
 */
const LOGGER_FRAMES = 2

function pageOrigin(): string {
  return typeof window === 'undefined' ? '' : window.location.origin
}

function record(level: ClientLogLevel, args: unknown[]): void {
  try {
    const source = callerFrame(new Error().stack, LOGGER_FRAMES, pageOrigin())
    clientLogBuffer.record(level, args, source)
  } catch {
    // Capturing is best effort; the console still gets the call below.
  }
}

/* eslint-disable no-console */
function wrap(level: ClientLogLevel, forward: LogFn): LogFn {
  return (...args: unknown[]): void => {
    record(level, args)
    forward(...args)
  }
}

/**
 * The frontend's logger. Each call goes to the real `console` method, so
 * DevTools work as before, and a redacted copy lands in the client log
 * buffer for the Activity page's Browser logs tab.
 */
export const logger = {
  error: wrap('error', console.error.bind(console)),
  warn: wrap('warn', console.warn.bind(console)),
  debug: wrap('debug', console.debug.bind(console)),
}
/* eslint-enable no-console */
