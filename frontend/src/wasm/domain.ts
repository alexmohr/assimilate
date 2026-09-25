// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The `domain` crate compiled to WebAssembly (scripts/build-wasm.sh). The
// module is inlined and instantiated synchronously on first import so callers
// keep plain synchronous functions; it is small enough (~180 KB, ~75 KB gzipped) that a
// separate fetch would cost more than it saves.
import { initSync, nextCronRuns as nextCronRunsInWasm } from './generated/domain_wasm'
import wasmDataUrl from './generated/domain_wasm_bg.wasm?inline'

const base64 = wasmDataUrl.slice(wasmDataUrl.indexOf(',') + 1)
initSync({ module: Uint8Array.from(atob(base64), (c) => c.charCodeAt(0)) })

export {
  defaultBodyTemplate,
  defaultPushBodyTemplate,
  defaultTitleTemplate,
  maxHookCommandTimeoutSeconds,
  notificationTemplatePlaceholderKeys,
  parseFileChangePatterns,
  renderNotificationTemplate,
  serializeFileChangePatterns,
  validateCron,
  type FileChangeAction,
  type FileChangePatternRow,
} from './generated/domain_wasm'
import type { TimezoneOffsets } from './generated/domain_wasm'

/**
 * The UTC offset of `timezone` at any instant, from the browser's own IANA
 * database via `Intl`, so the WebAssembly module needs no copy of it.
 */
function browserOffsets(timezone: string): TimezoneOffsets {
  const format = new Intl.DateTimeFormat('en-US', {
    timeZone: timezone,
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
  })
  return {
    offsetSecondsAt(epochMinutes: number): number {
      const instant = epochMinutes * 60_000
      const parts = format.formatToParts(new Date(instant))
      const part = (type: Intl.DateTimeFormatPartTypes): number =>
        Number(parts.find((p) => p.type === type)?.value)
      const wallClock = Date.UTC(
        part('year'),
        part('month') - 1,
        part('day'),
        part('hour'),
        part('minute'),
      )
      return (wallClock - instant) / 1000
    },
  }
}

/**
 * The next `count` runs of `expression` after `from` in `timezone`, computed by
 * the scheduler's own `next_runs_in`, so DST gaps and repeats resolve exactly as
 * the schedule will fire. Throws for an invalid expression or timezone.
 */
export function nextCronRuns(
  expression: string,
  from: Date,
  timezone: string,
  count: number,
): Date[] {
  return nextCronRunsInWasm(
    expression,
    from.toISOString(),
    timezone,
    browserOffsets(timezone),
    count,
  ).map((run) => new Date(run))
}
