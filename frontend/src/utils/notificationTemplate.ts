// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The placeholders the editor offers and the templates it pre-fills. Rendering happens on the
// server (POST /api/notifications/template-preview) with the renderer a delivery uses, so
// the only things kept here are these lists, and testdata/parity/notification_template.json
// holds both the frontend and crates/server/src/notifications/template.rs to the same ones.

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
