// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { EventType } from '../types/generated'

// Rendering, the default templates and the event labels all come from
// `crates/domain/src/notification/template.rs` via WebAssembly: the preview runs
// the exact code a channel uses to deliver, so it cannot drift from what is sent.
import {
  notificationTemplateDefaults,
  renderNotificationTemplate as renderInWasm,
} from '../wasm/domain'

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

// See the constants of the same name in crates/domain/src/notification/template.rs
// for why each default reads the way it does.
export const [DEFAULT_TITLE_TEMPLATE, DEFAULT_BODY_TEMPLATE, DEFAULT_PUSH_BODY_TEMPLATE] =
  notificationTemplateDefaults()

export interface NotificationPayloadSample {
  event_type: EventType
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

/**
 * Renders a `{{placeholder}}` template against a sample payload with the same
 * `render_template` a channel runs server-side. An unrecognized `{{...}}` token
 * is left verbatim, so a typo is visible in the preview instead of silently
 * disappearing.
 */
export function renderNotificationTemplate(
  template: string,
  sample: NotificationPayloadSample,
): string {
  return renderInWasm(template, JSON.stringify(sample))
}
