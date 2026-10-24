// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { isFsSnapshotOn, isValidLvmSnapshotPercent } from './fsSnapshot'
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

describe('isValidLvmSnapshotPercent', () => {
  it.each([
    [1, true],
    [10, true],
    [100, true],
    [0, false],
    [101, false],
    [2.5, false],
    [Number.NaN, false],
  ])('reads %s as %s', (percent, valid) => {
    expect(isValidLvmSnapshotPercent(percent)).toBe(valid)
  })
})
