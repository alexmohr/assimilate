// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The grammar (`<glob pattern> [ignore|warn|fatal]` per line, action defaults
// to `warn`, blank lines and `#` comments dropped) is implemented once in
// `crates/domain/src/file_change.rs` and runs here as WebAssembly, so the
// editor and the server can never disagree about what a line means.
import type { FileChangeAction as WasmFileChangeAction } from '../wasm/domain'

export {
  parseFileChangePatterns,
  serializeFileChangePatterns,
  type FileChangePatternRow,
} from '../wasm/domain'

// `erasableSyntaxOnly` (tsconfig.app.json) forbids real `enum` declarations;
// this const-object + derived-type pair is the erasable equivalent. The values
// are checked against the union generated from the Rust enum.
export const FileChangeAction = {
  Ignore: 'ignore',
  Warn: 'warn',
  Fatal: 'fatal',
} as const satisfies Record<string, WasmFileChangeAction>

export type FileChangeAction = WasmFileChangeAction
