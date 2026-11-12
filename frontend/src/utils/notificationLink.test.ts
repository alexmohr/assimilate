// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  DEFAULT_NOTIFICATION_LINK,
  NOTIFICATION_LINK_OPTIONS,
  channelLink,
  notificationLinkLabel,
  notificationLinkProblem,
  type NotificationLinkTarget,
} from './notificationLink'
import type { NotificationChannelResponse } from '../types/generated'

/** Every target the server knows, so a new one cannot slip past the picker. */
const ALL_TARGETS: NotificationLinkTarget[] = [
  'automatic',
  'activity',
  'agent',
  'schedule',
  'repository',
  'dashboard',
  'custom',
  'none',
]

describe('NOTIFICATION_LINK_OPTIONS', () => {
  it('offers every target exactly once, today’s behaviour first', () => {
    const values = NOTIFICATION_LINK_OPTIONS.map((o) => o.value)
    expect(values).toEqual(ALL_TARGETS)
    expect(new Set(values).size).toBe(values.length)
    expect(values[0]).toBe(DEFAULT_NOTIFICATION_LINK.target)
  })

  it('labels and explains every option', () => {
    for (const option of NOTIFICATION_LINK_OPTIONS) {
      expect(option.label.trim(), option.value).not.toBe('')
      expect(option.hint.trim(), option.value).not.toBe('')
    }
    expect(new Set(NOTIFICATION_LINK_OPTIONS.map((o) => o.label)).size).toBe(
      NOTIFICATION_LINK_OPTIONS.length,
    )
  })
})

describe('DEFAULT_NOTIFICATION_LINK', () => {
  it('links to where the event happened, with no address', () => {
    expect(DEFAULT_NOTIFICATION_LINK).toEqual({ target: 'automatic' })
  })
})

describe('channelLink', () => {
  it('reads the channel’s own link setting', () => {
    const link = { target: 'custom' as const, url: 'https://status.example.com' }
    expect(channelLink({ link } as NotificationChannelResponse)).toBe(link)
  })
})

describe('notificationLinkLabel', () => {
  it.each([
    ['automatic', 'Where the event happened'],
    ['activity', 'Activity log'],
    ['agent', 'Agent'],
    ['schedule', 'Schedule'],
    ['repository', 'Repository'],
    ['dashboard', 'Dashboard'],
    ['none', 'No link'],
  ] as const)('names %s as "%s"', (target, label) => {
    expect(notificationLinkLabel({ target })).toBe(label)
  })

  it('names your own address by the address itself', () => {
    expect(notificationLinkLabel({ target: 'custom', url: 'https://status.example.com' })).toBe(
      'https://status.example.com',
    )
  })

  it('falls back to a name for your own address before one is typed', () => {
    expect(notificationLinkLabel({ target: 'custom' })).toBe('Your own address')
    expect(notificationLinkLabel({ target: 'custom', url: '' })).toBe('Your own address')
  })

  it('ignores a leftover address once another target is chosen', () => {
    expect(notificationLinkLabel({ target: 'dashboard', url: 'https://old.example.com' })).toBe(
      'Dashboard',
    )
  })

  it('shows an unknown target from a newer server as it is', () => {
    expect(
      notificationLinkLabel({ target: 'somewhere-new' as unknown as NotificationLinkTarget }),
    ).toBe('somewhere-new')
  })
})

describe('notificationLinkProblem', () => {
  it.each(ALL_TARGETS.filter((t) => t !== 'custom'))('needs nothing more for %s', (target) => {
    expect(notificationLinkProblem({ target })).toBeNull()
    // Not even when a stale address is left over from an earlier choice.
    expect(notificationLinkProblem({ target, url: 'not a url' })).toBeNull()
  })

  it.each([
    ['no address', undefined],
    ['an empty address', ''],
    ['only whitespace', '   '],
  ])('asks for an address when your own has %s', (_case, url) => {
    expect(notificationLinkProblem({ target: 'custom', url })).toBe('an address to link to')
  })

  it.each([
    ['a bare host', 'status.example.com'],
    ['another scheme', 'ftp://status.example.com'],
    ['a script', 'javascript:alert(1)'],
    ['a scheme-relative address', '//status.example.com'],
  ])('refuses %s', (_case, url) => {
    expect(notificationLinkProblem({ target: 'custom', url })).toBe(
      'an address starting with http:// or https://',
    )
  })

  it.each([
    'http://status.example.com',
    'https://status.example.com/page?x=1',
    '  https://padded.example.com  ',
  ])('accepts %s', (url) => {
    expect(notificationLinkProblem({ target: 'custom', url })).toBeNull()
  })
})
