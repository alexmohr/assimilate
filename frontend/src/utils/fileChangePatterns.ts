// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Mirrors `parse_raw_file_change_patterns` in
// `crates/server/src/config_assembler.rs` - keep the two grammars in sync:
// each line is `<glob pattern> [ignore|warn|fatal]`, trailing action keyword
// defaults to `warn`, blank lines and `#`-prefixed comments are dropped.

// `erasableSyntaxOnly` (tsconfig.app.json) forbids real `enum` declarations;
// this const-object + derived-type pair is the erasable equivalent, mirroring
// the Rust `FileChangeAction` enum in `crates/shared/src/types.rs`.
export const FileChangeAction = {
  Ignore: 'ignore',
  Warn: 'warn',
  Fatal: 'fatal',
} as const

export type FileChangeAction = (typeof FileChangeAction)[keyof typeof FileChangeAction]

export interface FileChangePatternRow {
  path: string
  action: FileChangeAction
}

export function parseFileChangePatterns(raw: string): FileChangePatternRow[] {
  return raw
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith('#'))
    .map((line) => {
      const lastSpace = line.lastIndexOf(' ')
      if (lastSpace > 0) {
        const action = line.slice(lastSpace + 1).trim()
        if (
          action === FileChangeAction.Ignore ||
          action === FileChangeAction.Warn ||
          action === FileChangeAction.Fatal
        ) {
          return { path: line.slice(0, lastSpace).trim(), action }
        }
      }
      return { path: line, action: FileChangeAction.Warn }
    })
}

export function serializeFileChangePatterns(rows: FileChangePatternRow[]): string {
  return rows
    .map((r) => (r.action === FileChangeAction.Warn ? r.path : `${r.path} ${r.action}`))
    .join('\n')
}
