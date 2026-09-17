// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  DEFAULT_BODY_TEMPLATE,
  DEFAULT_TITLE_TEMPLATE,
  TEMPLATE_PLACEHOLDERS,
  renderNotificationTemplate,
  type NotificationPayloadSample,
} from './notificationTemplate'

const SUCCESS_SAMPLE: NotificationPayloadSample = {
  event_type: 'backup_success',
  hostname: 'web-server-01',
  repo_name: 'daily-backup',
  status: 'success',
  schedule_name: 'Nightly',
  next_run_at: '2026-06-10T10:00:00Z',
  archive_name: 'web-server-01-2026-06-09',
  duration_secs: 272,
  original_size: 10_737_418_240,
  compressed_size: 2_147_483_648,
  deduplicated_size: 524_288_000,
  files_processed: 184_203,
  timestamp: '2026-06-09T10:00:00Z',
  warnings: ['disk almost full'],
  error_message: 'none',
  activity_url: 'https://assimilate.example.com/activity?run_id=abc',
}

describe('renderNotificationTemplate', () => {
  it('substitutes every placeholder, matching the Rust formatting exactly', () => {
    const template = TEMPLATE_PLACEHOLDERS.map((p) => `{{${p.key}}}`).join(' ')
    const rendered = renderNotificationTemplate(template, SUCCESS_SAMPLE)
    expect(rendered).toBe(
      'Backup succeeded web-server-01 daily-backup success Nightly ' +
        '2026-06-10T10:00:00Z web-server-01-2026-06-09 4m 32s 10.0 GiB 2.0 GiB ' +
        '500.0 MiB 184203 2026-06-09T10:00:00Z - disk almost full none ' +
        'https://assimilate.example.com/activity?run_id=abc',
    )
  })

  it('shows the deduplicated size on the default body template for a successful backup', () => {
    const rendered = renderNotificationTemplate(DEFAULT_BODY_TEMPLATE, SUCCESS_SAMPLE)
    expect(rendered).toContain('Size:        10.0 GiB -> 2.0 GiB compressed (500.0 MiB new)')
  })

  it('renders the default title template as a readable summary', () => {
    expect(renderNotificationTemplate(DEFAULT_TITLE_TEMPLATE, SUCCESS_SAMPLE)).toBe(
      'Backup succeeded: web-server-01 / daily-backup',
    )
  })

  it('substitutes missing fields with an empty string', () => {
    const rendered = renderNotificationTemplate('dedup=[{{dedup_size}}] err=[{{error}}]', {
      event_type: 'agent_connected',
      hostname: 'myhost',
    })
    expect(rendered).toBe('dedup=[] err=[]')
  })

  it('leaves an unrecognized placeholder verbatim', () => {
    const rendered = renderNotificationTemplate('{{not_a_real_field}}', {
      event_type: 'agent_connected',
    })
    expect(rendered).toBe('{{not_a_real_field}}')
  })

  it('every placeholder in the picker appears in the default body template', () => {
    // `status` and `next_run` are offered as chips but deliberately left out of the default
    // body -- `status` duplicates the human-readable `event` label, and `next_run` only
    // matters alongside a schedule name, which the default already covers.
    const omittedFromDefault = new Set(['status', 'next_run'])
    for (const p of TEMPLATE_PLACEHOLDERS) {
      if (omittedFromDefault.has(p.key)) continue
      expect(DEFAULT_BODY_TEMPLATE).toContain(`{{${p.key}}}`)
    }
  })

  it('marks exactly the dedup_size placeholder as the dedup chip', () => {
    const dedupChips = TEMPLATE_PLACEHOLDERS.filter((p) => p.dedup)
    expect(dedupChips.map((p) => p.key)).toEqual(['dedup_size'])
  })

  it('renders a sub-KiB size in plain bytes', () => {
    const rendered = renderNotificationTemplate('{{dedup_size}}', {
      event_type: 'backup_success',
      deduplicated_size: 512,
    })
    expect(rendered).toBe('512 B')
  })

  it('renders a sub-minute duration in plain seconds', () => {
    const rendered = renderNotificationTemplate('{{duration}}', {
      event_type: 'backup_success',
      duration_secs: 9,
    })
    expect(rendered).toBe('9s')
  })
})
