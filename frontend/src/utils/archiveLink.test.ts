// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { linkFolder, linksToFolder, linkTitle } from './archiveLink'
import type { SymlinkTarget } from '../types/generated'

const toFolder: SymlinkTarget = {
  target: 'releases/2026-10-25',
  resolved: { state: 'in_archive', path: 'srv/app/releases/2026-10-25', directory: true },
}
const toFile: SymlinkTarget = {
  target: 'shared/.env.production',
  resolved: { state: 'in_archive', path: 'srv/app/shared/.env.production', directory: false },
}
const toRoot: SymlinkTarget = {
  target: '/',
  resolved: { state: 'in_archive', path: '', directory: true },
}
const outside: SymlinkTarget = {
  target: '/usr/share/zoneinfo/UTC',
  resolved: { state: 'not_in_archive' },
}
const unknown: SymlinkTarget = { target: 'html', resolved: { state: 'unknown' } }

describe('linkFolder', () => {
  it('opens a folder target itself', () => {
    expect(linkFolder(toFolder)).toBe('srv/app/releases/2026-10-25')
    expect(linkFolder(toRoot)).toBe('')
  })

  it('opens the folder holding a file target', () => {
    expect(linkFolder(toFile)).toBe('srv/app/shared')
  })

  it('has nothing to open when the archive does not hold the target', () => {
    expect(linkFolder(outside)).toBeNull()
    expect(linkFolder(unknown)).toBeNull()
  })
})

describe('linksToFolder', () => {
  it('is only true for a folder in the archive', () => {
    expect(linksToFolder(toFolder)).toBe(true)
    expect(linksToFolder(toFile)).toBe(false)
    expect(linksToFolder(outside)).toBe(false)
    expect(linksToFolder(unknown)).toBe(false)
  })
})

describe('linkTitle', () => {
  it('names the link, its target, and what a restore brings back', () => {
    expect(linkTitle('srv/app/current', toFolder)).toBe(
      'Symbolic link: /srv/app/current → releases/2026-10-25. Restoring it brings back the link, not what it points to.',
    )
  })

  it('says when the archive does not hold the target', () => {
    expect(linkTitle('etc/localtime', outside)).toBe(
      "Symbolic link: /etc/localtime → /usr/share/zoneinfo/UTC. Nothing at the target's path is in this archive.",
    )
  })

  it('names only the link when the index does not record its target', () => {
    expect(linkTitle('var/www/current', { resolved: { state: 'unknown' } })).toBe(
      'Symbolic link: /var/www/current.',
    )
  })

  it('says only where it points when the target was not looked up', () => {
    expect(linkTitle('var/www/current', unknown)).toBe('Symbolic link: /var/www/current → html.')
  })
})
