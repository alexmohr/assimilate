// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The `domain` crate compiled to WebAssembly (scripts/build-wasm.sh). The
// module is inlined and instantiated synchronously on first import so callers
// keep plain synchronous functions; it is small enough (~80 KB) that a
// separate fetch would cost more than it saves.
import { initSync } from './generated/domain_wasm'
import wasmDataUrl from './generated/domain_wasm_bg.wasm?inline'

const base64 = wasmDataUrl.slice(wasmDataUrl.indexOf(',') + 1)
initSync({ module: Uint8Array.from(atob(base64), (c) => c.charCodeAt(0)) })

export {
  parseFileChangePatterns,
  serializeFileChangePatterns,
  type FileChangeAction,
  type FileChangePatternRow,
} from './generated/domain_wasm'
