// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  collapse,
  isUnder,
  parentOf,
  selectAll,
  tick,
  tickState,
  toggle,
  untick,
  type ChildrenOf,
} from './restoreSelection'

// etc/
//   hosts
//   nginx/
//     nginx.conf
//     conf.d/
//       default.conf
// var/
//   www/
//     index.html
const TREE: Record<string, string[]> = {
  '': ['etc', 'var'],
  etc: ['etc/hosts', 'etc/nginx'],
  'etc/nginx': ['etc/nginx/nginx.conf', 'etc/nginx/conf.d'],
  'etc/nginx/conf.d': ['etc/nginx/conf.d/default.conf'],
  var: ['var/www'],
  'var/www': ['var/www/index.html'],
}

const known: ChildrenOf = (dir) => TREE[dir] ?? null

/** Only some listings loaded, as in a tree the user has partly opened. */
function loaded(...dirs: string[]): ChildrenOf {
  return (dir) => (dirs.includes(dir) ? (TREE[dir] ?? null) : null)
}

describe('paths', () => {
  it('finds the parent directory, the root holding top-level entries', () => {
    expect(parentOf('etc/nginx/nginx.conf')).toBe('etc/nginx')
    expect(parentOf('etc')).toBe('')
  })

  it('treats a path as under itself and its ancestors, not a sibling with a shared prefix', () => {
    expect(isUnder('etc/nginx/nginx.conf', 'etc')).toBe(true)
    expect(isUnder('etc', 'etc')).toBe(true)
    expect(isUnder('etc-old/hosts', 'etc')).toBe(false)
    expect(isUnder('anything', '')).toBe(true)
  })
})

describe('tickState', () => {
  it('is checked under a ticked folder, partial above a ticked path, unchecked elsewhere', () => {
    const selection = ['etc/nginx']

    expect(tickState(selection, 'etc/nginx')).toBe('checked')
    expect(tickState(selection, 'etc/nginx/conf.d/default.conf')).toBe('checked')
    expect(tickState(selection, 'etc')).toBe('partial')
    expect(tickState(selection, 'etc/hosts')).toBe('unchecked')
    expect(tickState(selection, 'var')).toBe('unchecked')
  })
})

describe('tick', () => {
  it('replaces whatever was ticked inside a folder with the folder itself', () => {
    const selection = tick(['etc/nginx/nginx.conf', 'var/www'], 'etc/nginx', loaded('etc/nginx'))

    expect(selection).toEqual(['etc/nginx', 'var/www'])
  })

  it('collapses a folder whose every child is ticked into one basket entry', () => {
    let selection = tick([], 'etc/hosts', known)
    selection = tick(selection, 'etc/nginx/nginx.conf', known)
    selection = tick(selection, 'etc/nginx/conf.d/default.conf', known)

    expect(selection).toEqual(['etc'])
  })

  it('keeps the children when the folder listing is not complete', () => {
    const selection = tick(['etc/hosts'], 'etc/nginx', loaded('etc/nginx'))

    expect(selection).toEqual(['etc/hosts', 'etc/nginx'])
  })

  it('never collapses into the archive root', () => {
    expect(tick(['etc'], 'var', known)).toEqual(['etc', 'var'])
  })
})

describe('untick', () => {
  it('drops a ticked path and everything under it', () => {
    const result = untick(['etc/nginx/nginx.conf', 'etc/nginx/conf.d', 'var'], 'etc/nginx', known)

    expect(result).toEqual({ selection: ['var'] })
  })

  it('splits a ticked ancestor into the siblings along the way down', () => {
    const result = untick(['etc'], 'etc/nginx/conf.d/default.conf', known)

    expect(result).toEqual({ selection: ['etc/hosts', 'etc/nginx/nginx.conf'] })
  })

  it('asks for the listings it needs to split an ancestor', () => {
    const result = untick(['etc'], 'etc/nginx/nginx.conf', loaded('etc'))

    expect(result).toEqual({ missing: ['etc/nginx'] })
  })
})

describe('toggle', () => {
  it('ticks a partial folder entirely and unticks a checked one', () => {
    const ticked = toggle(['etc/hosts'], 'etc', known)
    expect(ticked).toEqual({ selection: ['etc'] })

    const unticked = toggle(['etc'], 'etc', known)
    expect(unticked).toEqual({ selection: [] })
  })
})

describe('selectAll and collapse', () => {
  it('selects every top-level entry', () => {
    expect(selectAll(['etc', 'var'], known)).toEqual(['etc', 'var'])
  })

  it('drops duplicates and paths already covered', () => {
    expect(collapse(['var/www', 'var', 'var', 'etc/hosts'], known)).toEqual(['etc/hosts', 'var'])
  })
})
