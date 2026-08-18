// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

export const SRC = resolve(process.cwd(), 'src')

/**
 * Every file under `src` ending in one of `extensions`, sorted. Shared by the
 * meta-tests that sweep the whole tree - the shared-component audit, the
 * modal-usage audit, the UI conventions audit and the design-token audit -
 * which each carried an identical copy of this walk.
 *
 * `src/types/generated` is skipped: it is ts-rs output, so its contents are
 * neither hand-written nor ours to lint.
 */
export function sourceFiles(extensions: readonly string[]): string[] {
  const out: string[] = []
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) {
        if (entry.name !== 'generated') walk(full)
      } else if (extensions.some((e) => entry.name.endsWith(e))) {
        out.push(full)
      }
    }
  }
  walk(SRC)
  return out.sort()
}

/** Every single-file component under `src`. */
export function vueFiles(): string[] {
  return sourceFiles(['.vue'])
}
