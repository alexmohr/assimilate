// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { CHANNEL_SECTIONS, isChannelSection, smtpSecurityLabel } from './channelSettings'

describe('smtpSecurityLabel', () => {
  it('names every SMTP security mode', () => {
    expect(smtpSecurityLabel('starttls')).toBe('STARTTLS')
    expect(smtpSecurityLabel('tls')).toBe('SSL/TLS')
    expect(smtpSecurityLabel('none')).toBe('None (insecure)')
  })
})

describe('isChannelSection', () => {
  it('accepts every section and rejects anything else', () => {
    for (const section of CHANNEL_SECTIONS) expect(isChannelSection(section)).toBe(true)
    expect(isChannelSection('logs')).toBe(false)
    expect(isChannelSection(undefined)).toBe(false)
  })
})
