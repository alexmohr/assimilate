// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mockErrorUtils, mockErrorUtilsPassthrough } from './sharedMocks'

// The error extractors specs swap in for `../utils/error`. A spec asserting on
// the text a failure shows relies on exactly these answers.
describe('mockErrorUtils', () => {
  it('answers every failure with the same canned message', async () => {
    const utils = mockErrorUtils()
    expect(utils.extractError(new Error('refused'))).toBe('API error')
    await expect(utils.extractBlobError(new Error('refused'))).resolves.toBe('API error')
  })
})

describe('mockErrorUtilsPassthrough', () => {
  it("passes a thrown error's own message through", async () => {
    const utils = mockErrorUtilsPassthrough()
    expect(utils.extractError(new Error('refused'))).toBe('refused')
    await expect(utils.extractBlobError(new Error('refused'))).resolves.toBe('refused')
  })

  it('names anything that is not an error as unknown', async () => {
    const utils = mockErrorUtilsPassthrough()
    expect(utils.extractError('refused')).toBe('Unknown error')
    await expect(utils.extractBlobError(null)).resolves.toBe('Unknown error')
  })
})
