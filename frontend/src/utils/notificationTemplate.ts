// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { NotificationEventType } from '../types/notifications'

// Mirrors crates/server/src/notifications/template.rs, which is the single source of truth
// for what a channel actually sends. Kept as its own formatter (not `utils/format.ts`'s
// formatBytes/formatDuration) because the preview must match the backend's exact output --
// binary units (GiB/MiB, truncated, not rounded) and an "Xh Ym Zs" duration -- rather than
// this app's other, differently-rounded display formats.

export interface TemplatePlaceholder {
  key: string
  description: string
  /** Marks the placeholder that carries the deduplicated ("new data") size. */
  dedup?: boolean
}

export const TEMPLATE_PLACEHOLDERS: TemplatePlaceholder[] = [
  { key: 'event', description: 'event label' },
  { key: 'host', description: 'hostname' },
  { key: 'repository', description: 'repo name' },
  { key: 'status', description: 'raw status' },
  { key: 'schedule', description: 'schedule name' },
  { key: 'next_run', description: 'next scheduled run' },
  { key: 'archive', description: 'archive name' },
  { key: 'duration', description: 'e.g. 4m 32s' },
  { key: 'original_size', description: 'e.g. 10.0 GiB' },
  { key: 'compressed_size', description: 'e.g. 2.0 GiB' },
  { key: 'dedup_size', description: 'new data written, e.g. 500.0 MiB', dedup: true },
  { key: 'files', description: 'files processed' },
  { key: 'time', description: 'event timestamp' },
  { key: 'warnings', description: 'warning list' },
  { key: 'error', description: 'error message' },
  { key: 'activity_url', description: 'Activity Log link' },
]

// Deliberately omits `{{repository}}`: four of the nine event types carry no repository, and
// this one static default has to read cleanly for all of them -- see the matching constant in
// crates/server/src/notifications/template.rs for the full reasoning.
export const DEFAULT_TITLE_TEMPLATE = '{{event}}: {{host}}'

// Every line is a single `Label: {{value}}` pair -- deliberately no literal words wrapped
// around more than one placeholder, so a missing value just leaves the label with a blank
// line instead of rendering nonsensical filler text (e.g. a single combined "Size: -> compressed
// ( new)" line for the six event types with no size data). See the matching constant in
// crates/server/src/notifications/template.rs for the full reasoning.
export const DEFAULT_BODY_TEMPLATE = [
  'Event:       {{event}}',
  'Host:        {{host}}',
  'Repository:  {{repository}}',
  'Schedule:    {{schedule}}',
  'Archive:     {{archive}}',
  'Duration:    {{duration}}',
  'Original:    {{original_size}}',
  'Compressed:  {{compressed_size}}',
  'Dedup:       {{dedup_size}}',
  'Files:       {{files}}',
  'Time:        {{time}}',
  '',
  'Warnings:',
  '{{warnings}}',
  '',
  'Error:',
  '{{error}}',
  '',
  'View activity log: {{activity_url}}',
].join('\n')

// The default body a new web-push channel starts with -- see the matching constant in
// crates/server/src/notifications/template.rs for why push gets its own short default instead
// of the multi-line DEFAULT_BODY_TEMPLATE, and why it leads with {{host}} (so an event with
// neither {{repository}} nor {{error}}, e.g. agent_connected, still renders something instead
// of a lone blank space).
export const DEFAULT_PUSH_BODY_TEMPLATE = '{{host}} {{repository}} {{error}}'

const EVENT_LABELS: Record<NotificationEventType, string> = {
  backup_success: 'Backup succeeded',
  backup_warning: 'Backup warning',
  backup_failed: 'Backup failed',
  check_success: 'Check succeeded',
  check_failed: 'Check failed',
  agent_connected: 'Agent connected',
  agent_disconnected: 'Agent disconnected',
  schedule_auto_disabled: 'Schedule auto-disabled',
  backup_skipped_agent_offline: 'Backup skipped',
  backup_skipped_repo_offline: 'Backup skipped',
}

export interface NotificationPayloadSample {
  event_type: NotificationEventType
  hostname?: string
  repo_name?: string
  status?: string
  schedule_name?: string
  next_run_at?: string
  archive_name?: string
  duration_secs?: number | null
  original_size?: number | null
  compressed_size?: number | null
  deduplicated_size?: number | null
  files_processed?: number | null
  timestamp?: string
  warnings?: string[]
  error_message?: string
  activity_url?: string
}

function formatBytesForTemplate(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return ''
  const units: Array<[string, number]> = [
    ['TiB', 1024 ** 4],
    ['GiB', 1024 ** 3],
    ['MiB', 1024 ** 2],
    ['KiB', 1024],
  ]
  for (const [unit, size] of units) {
    if (bytes >= size) {
      const whole = Math.floor(bytes / size)
      const tenths = Math.floor(((bytes % size) * 10) / size)
      return `${whole}.${tenths} ${unit}`
    }
  }
  return `${bytes} B`
}

function formatDurationForTemplate(secs: number | null | undefined): string {
  if (secs === null || secs === undefined) return ''
  const hours = Math.floor(secs / 3600)
  const minutes = Math.floor((secs % 3600) / 60)
  const seconds = secs % 60
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`
  if (minutes > 0) return `${minutes}m ${seconds}s`
  return `${seconds}s`
}

/**
 * Renders a `{{placeholder}}` template against a sample payload, the same substitution
 * crates/server/src/notifications/template.rs::render_template performs server-side. An
 * unrecognized `{{...}}` token is left verbatim, matching the backend, so a typo is visible
 * in the preview instead of silently disappearing.
 */
export function renderNotificationTemplate(
  template: string,
  sample: NotificationPayloadSample,
): string {
  const warnings =
    sample.warnings && sample.warnings.length > 0
      ? sample.warnings.map((w) => `- ${w}`).join('\n')
      : ''

  const values: Record<string, string> = {
    event: EVENT_LABELS[sample.event_type],
    host: sample.hostname ?? '',
    repository: sample.repo_name ?? '',
    status: sample.status ?? '',
    schedule: sample.schedule_name ?? '',
    next_run: sample.next_run_at ?? '',
    archive: sample.archive_name ?? '',
    duration: formatDurationForTemplate(sample.duration_secs),
    original_size: formatBytesForTemplate(sample.original_size),
    compressed_size: formatBytesForTemplate(sample.compressed_size),
    dedup_size: formatBytesForTemplate(sample.deduplicated_size),
    files:
      sample.files_processed === null || sample.files_processed === undefined
        ? ''
        : String(sample.files_processed),
    time: sample.timestamp ?? '',
    warnings,
    error: sample.error_message ?? '',
    activity_url: sample.activity_url ?? '',
  }

  return substitutePlaceholders(template, values)
}

/**
 * Substitutes `{{key}}` tokens in a single left-to-right pass over `template`, mirroring
 * crates/server/src/notifications/template.rs::substitute_placeholders. Not a fold of
 * per-key `.split().join()` calls: each of those would rescan the already-substituted
 * output for the next key, so a value that happens to contain literal `{{other_key}}` text
 * (e.g. a hostname of `{{error}}`) would get expanded a second time on a later pass even
 * though it never appeared in the template the caller wrote. Scanning the original template
 * only once avoids that.
 */
function substitutePlaceholders(template: string, values: Record<string, string>): string {
  let out = ''
  let rest = template
  for (;;) {
    const start = rest.indexOf('{{')
    if (start === -1) {
      out += rest
      return out
    }
    out += rest.slice(0, start)
    const afterOpen = rest.slice(start + 2)
    const end = afterOpen.indexOf('}}')
    if (end === -1) {
      out += rest.slice(start)
      return out
    }
    const key = afterOpen.slice(0, end)
    out += Object.hasOwn(values, key) ? values[key] : `{{${key}}}`
    rest = afterOpen.slice(end + 2)
  }
}
