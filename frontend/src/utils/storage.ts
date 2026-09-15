// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

function getLocalStorage(): Storage | undefined {
  return typeof localStorage === 'undefined' ? undefined : localStorage
}

// usePersistedRef reads/writes during component setup(), including via an
// immediate watcher - an uncaught throw here (quota exceeded, storage
// disabled, a restrictive private-browsing mode) would otherwise propagate
// out of setup() and break the view's initial render, not just this one
// persisted value.
export function readStorage(key: string): string | undefined {
  let value: string | undefined
  try {
    value = getLocalStorage()?.getItem(key) ?? undefined
  } catch {
    // Storage inaccessible - value stays undefined, its declared default.
  }
  return value
}

export function writeStorage(key: string, value: string): void {
  try {
    getLocalStorage()?.setItem(key, value)
  } catch {
    // Persisting is best-effort; the value still works for this session.
  }
}

export function removeStorage(key: string): void {
  try {
    getLocalStorage()?.removeItem(key)
  } catch {
    // Best-effort, same as writeStorage above.
  }
}
