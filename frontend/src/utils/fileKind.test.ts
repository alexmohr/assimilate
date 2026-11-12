// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { isImagePath } from './fileKind'

describe('isImagePath', () => {
  it.each([
    'logo.png',
    'srv/www/photo.jpg',
    'photo.jpeg',
    'anim.gif',
    'pic.webp',
    'pic.avif',
    'scan.bmp',
    'icon.svg',
    'favicon.ico',
  ])('compares %s as an image', (path) => {
    expect(isImagePath(path)).toBe(true)
  })

  it('reads the extension whatever its case', () => {
    expect(isImagePath('DCIM/IMG_0001.JPG')).toBe(true)
    expect(isImagePath('banner.Png')).toBe(true)
  })

  it('reads only the last extension', () => {
    expect(isImagePath('backup.png.gz')).toBe(false)
    expect(isImagePath('archive.tar.svg')).toBe(true)
  })

  it.each(['etc/nginx/nginx.conf', 'notes.txt', 'image.pdf', 'logo.png.bak', 'tiff.tiff'])(
    'compares %s as text',
    (path) => {
      expect(isImagePath(path)).toBe(false)
    },
  )

  it('does not take a file without an extension for an image', () => {
    expect(isImagePath('png')).toBe(false)
    expect(isImagePath('srv/png/README')).toBe(false)
    expect(isImagePath('')).toBe(false)
  })

  it('does not take a dot file named like an extension for an image', () => {
    expect(isImagePath('.png')).toBe(false)
    expect(isImagePath('home/alex/.svg')).toBe(false)
  })

  it('looks at the file name, not a dot in a folder name', () => {
    expect(isImagePath('photos.png/readme')).toBe(false)
    expect(isImagePath('a.b/c.gif')).toBe(true)
  })

  it('does not count a trailing dot as an extension', () => {
    expect(isImagePath('logo.')).toBe(false)
  })
})
