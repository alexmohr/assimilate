// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

function getLocalStorage(): Storage | undefined {
  return typeof localStorage === 'undefined' ? undefined : localStorage
}

export function readStorage(key: string): string | undefined {
  return getLocalStorage()?.getItem(key) ?? undefined
}

export function writeStorage(key: string, value: string): void {
  getLocalStorage()?.setItem(key, value)
}

export function removeStorage(key: string): void {
  getLocalStorage()?.removeItem(key)
}
