// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import RepoAppendOnlyBadges from './RepoAppendOnlyBadges.vue'

function mount(maintenanceStopped: boolean | null) {
  const appendOnly =
    maintenanceStopped === null
      ? null
      : { maintenance_stopped: maintenanceStopped, pending_deletion_count: 0 }
  return renderWithPlugins(RepoAppendOnlyBadges, { props: { appendOnly } })
}

describe('RepoAppendOnlyBadges', () => {
  it('renders nothing for a repository that is not append-only', () => {
    expect(mount(null).findAll('.badge')).toHaveLength(0)
  })

  it('shows the purple badge for an append-only repository', () => {
    const badges = mount(false).findAll('.badge')
    expect(badges.map((b) => b.text())).toEqual(['Append-only'])
    expect(badges[0]!.classes()).toContain('badge--violet')
  })

  it('adds the amber badge when maintenance stopped', () => {
    const badges = mount(true).findAll('.badge')
    expect(badges.map((b) => b.text())).toEqual(['Append-only', 'Maintenance stopped'])
    expect(badges[1]!.classes()).toContain('badge--warning')
  })
})
