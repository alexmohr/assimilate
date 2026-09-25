// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The timezone-aware half of the `domain` crate (crates/domain-wasm-tz). It
// embeds the IANA database (~1 MB), so unlike ./domain it is loaded on first
// use, as its own chunk, and only by views that preview schedule runs.
import type * as TimezoneBindings from './generated/domain_wasm_tz'

type TimezoneModule = typeof TimezoneBindings

let loaded: Promise<TimezoneModule> | null = null

function load(): Promise<TimezoneModule> {
  loaded ??= Promise.all([
    import('./generated/domain_wasm_tz'),
    import('./generated/domain_wasm_tz_bg.wasm?inline'),
  ]).then(([module, { default: wasmDataUrl }]) => {
    const base64 = wasmDataUrl.slice(wasmDataUrl.indexOf(',') + 1)
    module.initSync({ module: Uint8Array.from(atob(base64), (c) => c.charCodeAt(0)) })
    return module
  })
  return loaded
}

/**
 * The next `count` runs of `expression` after `from`, in the IANA `timezone`,
 * computed by the scheduler's own `next_runs` so DST gaps and repeats resolve
 * exactly as they will when the schedule fires.
 */
export async function nextCronRuns(
  expression: string,
  from: Date,
  timezone: string,
  count: number,
): Promise<Date[]> {
  const module = await load()
  return module
    .nextCronRuns(expression, from.toISOString(), timezone, count)
    .map((run) => new Date(run))
}
