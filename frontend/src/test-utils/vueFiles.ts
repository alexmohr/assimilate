// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

export const SRC = resolve(process.cwd(), 'src')

/**
 * Every `.vue` file under `src`, sorted. Shared by the meta-tests that sweep
 * the whole component tree - the shared-component audit and the modal-usage
 * audit - which each carried an identical copy of this walk.
 */
export function vueFiles(): string[] {
  const out: string[] = []
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith('.vue')) out.push(full)
    }
  }
  walk(SRC)
  return out.sort()
}
