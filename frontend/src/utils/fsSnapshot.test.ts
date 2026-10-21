// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { isFsSnapshotOn } from './fsSnapshot'
import type { FsSnapshotMode } from '../types/generated'

describe('isFsSnapshotOn', () => {
  it.each([
    ['off', false],
    ['required', true],
    ['best_effort', true],
    [undefined, false],
  ] as [FsSnapshotMode | undefined, boolean][])('reads %s as %s', (mode, on) => {
    expect(isFsSnapshotOn(mode)).toBe(on)
  })
})
