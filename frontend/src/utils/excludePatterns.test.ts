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

  it('flags anchored prefix styles such as pp: with a generic hint', () => {
    const [warning] = excludePatternWarnings('pp:__pycache__')
    expect(warning?.message).toContain('start it with "/"')
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
