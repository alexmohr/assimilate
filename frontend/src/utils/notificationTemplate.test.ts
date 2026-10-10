// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Rendering is the server's (crates/server/src/notifications/{template,preview}.rs, tested
// there); what the frontend still owns is which placeholders it offers and which defaults it
// pre-fills, and those must match what the server delivers with.
import { describe, expect, it } from 'vitest'
import shared from '../../../testdata/parity/notification_template.json'
import {
  DEFAULT_BODY_TEMPLATE,
  DEFAULT_PUSH_BODY_TEMPLATE,
  DEFAULT_TITLE_TEMPLATE,
  TEMPLATE_PLACEHOLDERS,
} from './notificationTemplate'

describe('notification template defaults and placeholders', () => {
  it('pre-fills the same defaults the server delivers with', () => {
    expect(DEFAULT_TITLE_TEMPLATE).toBe(shared.default_title_template)
    expect(DEFAULT_BODY_TEMPLATE).toBe(shared.default_body_template)
    expect(DEFAULT_PUSH_BODY_TEMPLATE).toBe(shared.default_push_body_template)
  })

  it('offers exactly the placeholders the server substitutes, in the same order', () => {
    expect(TEMPLATE_PLACEHOLDERS.map((p) => p.key)).toEqual(shared.placeholder_keys)
  })

  it('gives web push its own short default body instead of the multi-line email default', () => {
    expect(DEFAULT_PUSH_BODY_TEMPLATE).not.toBe(DEFAULT_BODY_TEMPLATE)
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
})
