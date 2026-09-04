// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { dropBlankCommands } from './validation'
import { hookCommand } from './hookCommands'

describe('dropBlankCommands', () => {
  it('drops blank and whitespace-only entries', () => {
    expect(
      dropBlankCommands([
        hookCommand('echo one'),
        hookCommand(''),
        hookCommand('   '),
        hookCommand('echo two'),
      ]),
    ).toEqual([hookCommand('echo one'), hookCommand('echo two')])
  })

  it('keeps leading whitespace in a surviving entry, since it may be meaningful indentation', () => {
    const script = '  if true; then\n    echo indented\n  fi'
    expect(dropBlankCommands([hookCommand(script)])).toEqual([hookCommand(script)])
  })

  it('keeps a surviving entry timeout', () => {
    expect(dropBlankCommands([hookCommand('vzdump --all 1', 7200), hookCommand('')])).toEqual([
      hookCommand('vzdump --all 1', 7200),
    ])
  })

  it('returns an empty array unchanged', () => {
    expect(dropBlankCommands([])).toEqual([])
  })
})
