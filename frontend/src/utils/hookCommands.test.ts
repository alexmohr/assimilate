// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import limits from '../../../testdata/parity/hooks.json'
import { MAX_HOOK_COMMAND_TIMEOUT_SECONDS, hookCommand } from './hookCommands'

describe('hookCommands', () => {
  it('caps the timeout at the limit the server enforces', () => {
    expect(MAX_HOOK_COMMAND_TIMEOUT_SECONDS).toBe(limits.max_hook_command_timeout_seconds)
  })

  it('builds a command that inherits the schedule timeout by default', () => {
    expect(hookCommand('echo hi')).toEqual({ command: 'echo hi', timeout_seconds: null })
    expect(hookCommand('sleep 5', 30)).toEqual({ command: 'sleep 5', timeout_seconds: 30 })
  })
})
