// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// Borg exclude patterns are matched against paths stored without their leading
// slash (`home/alex/.cache`). Every style except `re:` is anchored at the start
// of that path, so a pattern such as `.cache` only matches a top-level `.cache`
// and silently excludes nothing under `/home/<user>`. The pattern has to start
// with `/` (an absolute path) or `*` (any prefix) to reach deeper.

export const PatternStyle = {
  Fnmatch: 'fm',
  Shell: 'sh',
  Regex: 're',
  PathPrefix: 'pp',
  PathFull: 'pf',
} as const

export type PatternStyle = (typeof PatternStyle)[keyof typeof PatternStyle]

export interface ExcludeWarning {
  /** 1-based line number in the raw text. */
  line: number
  pattern: string
  message: string
}

const STYLE_PREFIX = /^([a-z]{2}):/

function isPatternStyle(value: string): value is PatternStyle {
  return (Object.values(PatternStyle) as string[]).includes(value)
}

/** Splits an optional `xx:` style prefix off a pattern; no prefix means fnmatch. */
export function parsePatternStyle(pattern: string): { style: PatternStyle; body: string } {
  const match = STYLE_PREFIX.exec(pattern)
  const prefix = match?.[1]
  if (prefix !== undefined && isPatternStyle(prefix)) {
    return { style: prefix, body: pattern.slice(prefix.length + 1) }
  }
  return { style: PatternStyle.Fnmatch, body: pattern }
}

function isAnchoredAtRoot(body: string): boolean {
  return !body.startsWith('/') && !body.startsWith('*')
}

function warningFor(pattern: string): string | null {
  const { style, body } = parsePatternStyle(pattern)
  if (style === PatternStyle.Regex || body === '' || !isAnchoredAtRoot(body)) {
    return null
  }
  const hint =
    style === PatternStyle.Fnmatch
      ? `use "*/${body}" to match it under every user, or "/${body}" for an absolute path`
      : 'start it with "/" for an absolute path or "*" to match under every user'
  return `"${pattern}" only matches at the very top of the backup root (borg paths have no leading slash); ${hint}.`
}

/** Flags patterns that look like they match anywhere but are anchored at the root. */
export function excludePatternWarnings(raw: string): ExcludeWarning[] {
  return raw.split('\n').flatMap((rawLine, index) => {
    const pattern = rawLine.trim()
    if (pattern === '' || pattern.startsWith('#')) {
      return []
    }
    const message = warningFor(pattern)
    return message === null ? [] : [{ line: index + 1, pattern, message }]
  })
}
