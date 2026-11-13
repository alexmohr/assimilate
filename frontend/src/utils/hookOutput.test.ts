// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  appendHookOutput,
  formatHookOutput,
  MAX_HOOK_OUTPUT_LINES,
  parseHookOutput,
  type HookOutputEntry,
} from './hookOutput'

const entry = (line: string): HookOutputEntry => ({
  type: 'hook_output',
  phase: 'post_backup',
  stream: 'stdout',
  line,
})

describe('parseHookOutput', () => {
  it('parses a hook line from the agent', () => {
    const raw = JSON.stringify(entry('done'))
    expect(parseHookOutput(raw)).toEqual(entry('done'))
  })

  it('ignores borg progress lines', () => {
    const raw = JSON.stringify({ type: 'archive_progress', nfiles: 1, original_size: 2, path: 'a' })
    expect(parseHookOutput(raw)).toBeNull()
  })

  it('ignores plain text and malformed hook lines', () => {
    expect(parseHookOutput('Creating archive at ...')).toBeNull()
    expect(parseHookOutput(JSON.stringify({ type: 'hook_output', line: 'x' }))).toBeNull()
    expect(
      parseHookOutput(
        JSON.stringify({ type: 'hook_output', phase: 'sideways', stream: 'stdout', line: 'x' }),
      ),
    ).toBeNull()
  })
})

describe('formatHookOutput', () => {
  it('names the phase and keeps what the script printed', () => {
    expect(formatHookOutput(entry('dumping'))).toBe('post-backup: dumping')
    expect(formatHookOutput({ ...entry('mounting'), phase: 'pre_backup' })).toBe(
      'pre-backup: mounting',
    )
  })
})

describe('appendHookOutput', () => {
  it('keeps only the newest lines past the cap', () => {
    const full = Array.from({ length: MAX_HOOK_OUTPUT_LINES }, (_, i) => entry(`line ${i}`))
    const next = appendHookOutput(full, entry('newest'))
    expect(next).toHaveLength(MAX_HOOK_OUTPUT_LINES)
    expect(next[0]?.line).toBe('line 1')
    expect(next.at(-1)?.line).toBe('newest')
  })

  it('does not mutate the lines it was given', () => {
    const lines = [entry('a')]
    appendHookOutput(lines, entry('b'))
    expect(lines).toHaveLength(1)
  })
})
