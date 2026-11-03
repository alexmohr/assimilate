// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'

import { excludePatternWarnings, parsePatternStyle, PatternStyle } from './excludePatterns'

describe('parsePatternStyle', () => {
  it('defaults to fnmatch without a prefix', () => {
    expect(parsePatternStyle('.cache')).toEqual({ style: PatternStyle.Fnmatch, body: '.cache' })
  })

  it('splits a known style prefix off', () => {
    expect(parsePatternStyle('pp:/proc')).toEqual({ style: PatternStyle.PathPrefix, body: '/proc' })
    expect(parsePatternStyle('re:\\.git/')).toEqual({ style: PatternStyle.Regex, body: '\\.git/' })
  })

  it('treats an unknown prefix as part of an fnmatch pattern', () => {
    expect(parsePatternStyle('xx:foo')).toEqual({ style: PatternStyle.Fnmatch, body: 'xx:foo' })
  })
})

describe('excludePatternWarnings', () => {
  it('flags a bare relative pattern and suggests the all-users form', () => {
    const [warning] = excludePatternWarnings('.local/share/containers')
    expect(warning?.line).toBe(1)
    expect(warning?.pattern).toBe('.local/share/containers')
    expect(warning?.message).toContain('"*/.local/share/containers"')
  })

  it('accepts absolute and wildcard-led patterns', () => {
    expect(excludePatternWarnings('/proc\n*/.cache\n*.pyc\n*cache*')).toEqual([])
  })

  it('accepts regex patterns, which are not anchored', () => {
    expect(excludePatternWarnings('re:\\.git/objects/')).toEqual([])
  })

  it('accepts a full path from the root written without a leading slash', () => {
    expect(
      excludePatternWarnings('home/*/Downloads\nsh:home/*/Downloads\npp:home/alex/.cache\nvar'),
    ).toEqual([])
  })

  it('flags a relative path whose first part is not a root directory', () => {
    const [warning] = excludePatternWarnings('target/debug')
    expect(warning?.message).toContain('"*/target/debug"')
  })

  it('treats an explicit fm: prefix like the default style', () => {
    expect(excludePatternWarnings('fm:.cache')).toHaveLength(1)
    expect(excludePatternWarnings('fm:*/.cache')).toEqual([])
  })

  it('suggests a double star for sh:, where a single star stops at a slash', () => {
    const [bare] = excludePatternWarnings('sh:.cache')
    expect(bare?.message).toContain('"**/.cache"')
    const [single] = excludePatternWarnings('sh:*/.cache')
    expect(single?.message).toContain('"**/.cache"')
    expect(excludePatternWarnings('sh:**/.cache')).toEqual([])
  })

  it('never suggests a wildcard for pp: and pf:, which take none', () => {
    for (const pattern of ['pp:__pycache__', 'pf:etc2/passwd']) {
      const [warning] = excludePatternWarnings(pattern)
      expect(warning?.message).toContain('takes no wildcards')
      expect(warning?.message).not.toContain('"*/')
      expect(warning?.message).toContain('"/')
    }
    expect(excludePatternWarnings('pp:*/__pycache__')).toHaveLength(1)
    expect(excludePatternWarnings('pp:/home/alex/.cache')).toEqual([])
  })

  it('skips blank lines and comments but keeps real line numbers', () => {
    const warnings = excludePatternWarnings('# comment\n\n/tmp\nnode_modules\n')
    expect(warnings).toHaveLength(1)
    expect(warnings[0]?.line).toBe(4)
  })

  it('ignores a style prefix with an empty pattern', () => {
    expect(excludePatternWarnings('pp:')).toEqual([])
  })
})
