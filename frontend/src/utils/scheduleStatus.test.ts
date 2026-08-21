// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { scheduleDisabledLabel } from './scheduleStatus'

describe('scheduleDisabledLabel', () => {
  it('renders an enabled schedule as Enabled regardless of stale failure fields', () => {
    expect(
      scheduleDisabledLabel({
        enabled: true,
        auto_disabled_agent_unreachable: false,
        consecutive_failures: 0,
      }),
    ).toBe('Enabled')
  })

  it('flags a schedule the scheduler disabled for an unreachable agent', () => {
    expect(
      scheduleDisabledLabel({
        enabled: false,
        auto_disabled_agent_unreachable: true,
        consecutive_failures: 3,
      }),
    ).toBe('Auto-disabled · agent unreachable')
  })

  it('flags a schedule the scheduler disabled for a local/config error', () => {
    expect(
      scheduleDisabledLabel({
        enabled: false,
        auto_disabled_agent_unreachable: false,
        consecutive_failures: 3,
      }),
    ).toBe('Auto-disabled · error')
  })

  it('falls back to a plain Disabled label for a human/quota disable', () => {
    expect(
      scheduleDisabledLabel({
        enabled: false,
        auto_disabled_agent_unreachable: false,
        consecutive_failures: 0,
      }),
    ).toBe('Disabled')
  })
})
