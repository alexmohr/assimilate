// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  checkDescription,
  presetForPort,
  presetPort,
  protocolLabel,
  reachabilityBadge,
  testResultText,
} from './dependencyHost'
import { isDependencyHostSection } from './dependencyHostSettings'

describe('dependency host helpers', () => {
  it('names the well-known ports and calls the rest TCP', () => {
    expect([445, 2049, 22, 8443].map(protocolLabel)).toEqual(['SMB', 'NFS', 'SSH', 'TCP'])
    expect(checkDescription(445)).toBe('SMB, TCP port 445')
    expect(checkDescription(8443)).toBe('TCP port 8443')
  })

  it('maps presets to ports and back', () => {
    expect(presetPort('nfs')).toBe(2049)
    expect(presetPort('other')).toBeNull()
    expect(presetForPort(22)).toBe('ssh')
    expect(presetForPort(8443)).toBe('other')
  })

  it('tells a dependency never checked apart from one that did not answer', () => {
    expect(reachabilityBadge(true)).toEqual({ tone: 'success', label: 'Reachable' })
    expect(reachabilityBadge(false)).toEqual({ tone: 'warning', label: 'Not answering' })
    expect(reachabilityBadge(null)).toEqual({ tone: 'neutral', label: 'Not checked yet' })
  })

  it('reports a connection test', () => {
    expect(testResultText({ reachable: true, address: 'nas.lan', port: 445 })).toBe(
      'nas.lan answered on port 445',
    )
    expect(testResultText({ reachable: false, address: 'nas.lan', port: 445 })).toBe(
      'nas.lan did not answer on port 445 within 5 seconds',
    )
  })

  it('parses only the sections the page has', () => {
    expect(isDependencyHostSection('used-by')).toBe(true)
    expect(isDependencyHostSection('repositories')).toBe(false)
    expect(isDependencyHostSection(undefined)).toBe(false)
  })
})
